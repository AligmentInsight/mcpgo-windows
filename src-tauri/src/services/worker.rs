//! The McpGo service worker - the SCM-facing host of one service
//! definition.
//!
//! # Why a worker exists
//!
//! Plain runtimes (node.exe, python.exe, powershell.exe) are not
//! SCM-aware: they never call `StartServiceCtrlDispatcher`, so Windows
//! refuses to run them as services (error 1053 after a 30 s timeout).
//! The macOS original had the same shape - a LaunchAgent plist never
//! runs a raw binary either, launchd supervises it. The Windows port
//! therefore registers ONE SCM-aware host per definition: the McpGo
//! application executable itself, re-invoked by the SCM in a headless
//! mode:
//!
//! ```text
//! "<McpGo.exe>" --service-host <id> --data-dir "<data dir>"
//! ```
//!
//! The SCM starts that ImagePath, the process detects the flags before
//! building any GUI (see `main.rs`), connects to the Service Control
//! Manager as the service `McpGo-<id>` and supervises the runtime
//! process described by `<data dir>\services\<id>.json`.
//!
//! # Lifecycle (mirrors the LaunchAgent semantics)
//!
//! | SCM event          | Worker action                                   |
//! |--------------------|-------------------------------------------------|
//! | service start      | load the definition, spawn the runtime process, |
//! |                    | report Running, pipe output to the log file     |
//! | runtime exits      | without keep-alive: report Stopped; with        |
//! |                    | keep-alive: restart it after 5 s (port of the   |
//! |                    | LaunchAgent `KeepAlive` flag)                   |
//! | SCM Stop request   | terminate the runtime process tree, report     |
//! |                    | StopPending -> Stopped                          |
//! | worker crash       | restarted by the SCM failure recovery actions   |
//! |                    | configured by Install-McpGoService.ps1          |
//!
//! # Account and data directory
//!
//! Services created by `Install-McpGoService.ps1` run as LocalSystem
//! whose `%APPDATA%` is NOT the installing user's. The install flow
//! therefore bakes the real data directory into the ImagePath via
//! `--data-dir`, keeping the definition files and logs where the GUI
//! (and the user) can read them.
//!
//! # Logging
//!
//! All output lands in `<data dir>\services\logs\<id>.log` using the
//! same canonical line format as the job logs:
//! `[RFC3339-UTC] OUT|ERR|SYS <text>`. The GUI user opens the file to
//! inspect a service; the SCM event log is untouched.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::{define_windows_service, service_dispatcher};

use crate::paths;
use crate::services::definition::{RuntimeKind, ServiceDefinition, SERVICE_NAME_PREFIX};
use crate::time;

/// ImagePath flag selecting the service-worker mode (see module docs).
pub const SERVICE_HOST_FLAG: &str = "--service-host";
/// ImagePath flag carrying the McpGo data directory (see module docs).
pub const DATA_DIR_FLAG: &str = "--data-dir";

/// Delay before a keep-alive restart of the runtime process.
const RESTART_DELAY: Duration = Duration::from_secs(5);
/// Supervision poll interval: the granularity of stop detection and
/// child-exit detection.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Status wait hint reported while the definition loads and the runtime
/// process spawns (START_PENDING / STOP_PENDING).
const TRANSITION_HINT: Duration = Duration::from_secs(30);

/// Windows process creation flag CREATE_NO_WINDOW (0x0800_0000): the
/// runtime process must never flash a console window, not even in
/// session 0.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Tree-killing helper shipped with every Windows installation.
const TASKKILL: &str = "taskkill.exe";

// ----------------------------------------------------------------------------
// Service-host mode detection
// ----------------------------------------------------------------------------

/// Parsed service-worker invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceHostArgs {
    /// Definition id (the service is `McpGo-<id>`).
    pub id: String,
    /// McpGo data directory carried by `--data-dir`.
    pub data_dir: PathBuf,
}

/// Extracts the worker flags from a plain argument list. Returns `None`
/// unless BOTH flags are present - a GUI invocation never carries them,
/// and a half-configured ImagePath is treated as a GUI start rather than
/// crashing the worker loop.
///
/// Order-agnostic and tolerant of unrelated arguments (forward-compat
/// for future flags).
pub fn parse_service_host_args(args: &[String]) -> Option<ServiceHostArgs> {
    let mut id: Option<String> = None;
    let mut data_dir: Option<String> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            SERVICE_HOST_FLAG if id.is_none() => {
                id = args.get(index + 1).cloned();
                index += 2;
                continue;
            }
            DATA_DIR_FLAG if data_dir.is_none() => {
                data_dir = args.get(index + 1).cloned();
                index += 2;
                continue;
            }
            _ => {}
        }
        index += 1;
    }

    Some(ServiceHostArgs {
        id: id?,
        data_dir: data_dir.map(PathBuf::from)?,
    })
}

/// Entry hook called first from `main()`. Returns `Some(exit_code)` when
/// this process was started by the SCM in service-worker mode - the GUI
/// must then never be built. Returns `None` for ordinary GUI starts.
///
/// Failures are dual-reported: the service log file (works in release
/// builds with `windows_subsystem = "windows"`, where stderr is void)
/// and stderr (visible in debug consoles).
pub fn maybe_run_service_host() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let host_args = parse_service_host_args(&args)?;

    match run_dispatcher(&host_args) {
        Ok(()) => Some(0),
        Err(error) => {
            let message = format!("service worker failed: {error}");
            worker_log(&host_args, "SYS", &message);
            eprintln!("[mcpgo-service] {message}");
            Some(1)
        }
    }
}

/// Connects to the Service Control Manager and blocks until the service
/// stops. Outside a real service context (manual `--service-host` run
/// from a console) this fails fast with a dispatcher error - which is
/// exactly what a developer testing the mode wants to see.
fn run_dispatcher(host_args: &ServiceHostArgs) -> windows_service::Result<()> {
    // The dispatcher wants the SCM service name, which by convention is
    // the definition id under the McpGo- namespace (see definition.rs).
    let service_name = format!("{SERVICE_NAME_PREFIX}{}", host_args.id);
    service_dispatcher::start(service_name, ffi_service_main)
}

// ----------------------------------------------------------------------------
// SCM protocol plumbing
// ----------------------------------------------------------------------------

// Generates the `extern "system"` adapter the SCM calls into. The macro
// (windows-service 0.7) also decodes the raw UTF-16 argument array into a
// `Vec<OsString>` before invoking `service_main` with it, so no unsafe
// pointer walking exists on this side.
define_windows_service!(ffi_service_main, service_main);

/// SCM entry point. `arguments[0]` is the service name the process was
/// started as; the remaining entries are the ImagePath arguments (see
/// the binPath built by `installer.rs`).
fn service_main(arguments: Vec<OsString>) {
    let (service_name, host_args) = match parse_service_main_args(&arguments) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("[mcpgo-service] cannot parse the service arguments: {message}");
            return;
        }
    };

    if let Err(error) = supervise(&service_name, &host_args) {
        let message = format!("service worker failed: {error}");
        worker_log(&host_args, "SYS", &message);
        eprintln!("[mcpgo-service] {message}");
    }
}

/// Decodes the argument vector assembled by `define_windows_service!`:
/// the first element is the service name, the rest are the ImagePath
/// arguments. Conversion is lossy (defensive: a malformed service
/// registration must not abort the process with a panic).
fn parse_service_main_args(arguments: &[OsString]) -> Result<(String, ServiceHostArgs), String> {
    let Some(first) = arguments.first() else {
        return Err("the SCM passed no arguments".into());
    };
    let service_name = first.to_string_lossy().into_owned();
    let image_path_args: Vec<String> = arguments[1..]
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    let host_args = parse_service_host_args(&image_path_args)
        .ok_or_else(|| "missing --service-host / --data-dir flags".to_string())?;
    Ok((service_name, host_args))
}

/// How one supervision cycle of the runtime process ended.
enum CycleOutcome {
    /// The SCM requested a stop: the child tree was terminated.
    StopRequested,
    /// The runtime process exited on its own, carrying the exit code.
    Exited(Option<i32>),
    /// Waiting on the child failed (an OS-level error, not an exit).
    WaitFailed(String),
}

/// The supervision loop - the heart of the worker. Never panics: every
/// failure is logged and turned into a clean Stopped report so the SCM
/// stays consistent.
fn supervise(service_name: &str, host_args: &ServiceHostArgs) -> windows_service::Result<()> {
    // Control channel: the SCM handler signals a stop request, the loop
    // below polls it. mpsc (not an AtomicBool) supports the handler
    // running on the SCM's own thread.
    let (stop_sender, stop_receiver) = mpsc::channel::<()>();
    let status_handle = service_control_handler::register(
        service_name,
        move |event| match event {
            ServiceControl::Stop => {
                let _ = stop_sender.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        },
    )?;

    set_status(&status_handle, ServiceState::StartPending, 1);

    // Load the definition BEFORE reporting Running: a broken definition
    // is a configuration error and the service reports Stopped instead
    // of pretending to run.
    let definition = match load_definition(host_args) {
        Ok(definition) => definition,
        Err(message) => {
            worker_log(host_args, "SYS", &message);
            set_status(&status_handle, ServiceState::Stopped, 0);
            return Ok(());
        }
    };

    let logger = Arc::new(ServiceLogger::new(host_args));
    logger.append(
        "SYS",
        &format!(
            "service worker started (runtime {}, command {}, keep-alive {})",
            definition.runtime.label(),
            definition.command,
            definition.keep_alive
        ),
    );

    set_status(&status_handle, ServiceState::Running, 0);

    let mut restarts: u32 = 0;
    'supervision: loop {
        // --- spawn ---------------------------------------------------------
        let mut child = match spawn_runtime_process(&definition, host_args) {
            Ok(child) => child,
            Err(error) => {
                logger.append("SYS", &format!("cannot start the runtime process: {error}"));
                if !definition.keep_alive || stop_requested(&stop_receiver) {
                    break 'supervision;
                }
                if sleep_unless_stopped(RESTART_DELAY, &stop_receiver) {
                    break 'supervision;
                }
                restarts += 1;
                continue 'supervision;
            }
        };
        let pid = child.id();
        logger.append(
            "SYS",
            &format!("runtime process started (pid {pid}, restart {restarts})"),
        );
        let readers = spawn_log_readers(&mut child, &logger);

        // --- watch ---------------------------------------------------------
        let outcome = loop {
            if stop_requested(&stop_receiver) {
                // Report STOP_PENDING immediately: the SCM shows the
                // wait hint while the child tree is taken down.
                set_status(&status_handle, ServiceState::StopPending, 1);
                kill_process_tree(pid);
                let _ = child.wait();
                break CycleOutcome::StopRequested;
            }
            match child.try_wait() {
                Ok(Some(status)) => break CycleOutcome::Exited(status.code()),
                Ok(None) => {}
                Err(error) => break CycleOutcome::WaitFailed(error.to_string()),
            }
            thread::sleep(POLL_INTERVAL);
        };

        // The pipes are closed now (the process is gone), so the readers
        // drain to EOF and finish immediately.
        for handle in readers {
            let _ = handle.join();
        }

        match outcome {
            CycleOutcome::StopRequested => break 'supervision,
            CycleOutcome::WaitFailed(message) => {
                logger.append("SYS", &format!("cannot wait for the runtime process: {message}"));
                break 'supervision;
            }
            CycleOutcome::Exited(code) => {
                logger.append(
                    "SYS",
                    &format!("runtime process exited (pid {pid}, exit code {code:?})"),
                );
                if !definition.keep_alive || stop_requested(&stop_receiver) {
                    break 'supervision;
                }
                logger.append("SYS", "keep-alive enabled: restarting the runtime process");
                if sleep_unless_stopped(RESTART_DELAY, &stop_receiver) {
                    break 'supervision;
                }
                restarts += 1;
                // next iteration of 'supervision respawns
            }
        }
    }

    set_status(&status_handle, ServiceState::StopPending, 1);
    logger.append("SYS", "service worker stopped");
    set_status(&status_handle, ServiceState::Stopped, 0);
    Ok(())
}

/// Reports a state transition to the SCM. Reporting failures are logged
/// to stderr and swallowed: during system shutdown the SCM may already
/// be gone, and the worker must still finish its cleanup.
fn set_status(handle: &ServiceStatusHandle, state: ServiceState, checkpoint: u32) {
    let status = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        exit_code: ServiceExitCode::NO_ERROR,
        checkpoint,
        wait_hint: if state == ServiceState::Running || state == ServiceState::Stopped {
            Duration::ZERO
        } else {
            TRANSITION_HINT
        },
        // Only the Running state accepts further controls (Stop).
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP
        } else {
            ServiceControlAccept::empty()
        },
        // The process id is a read-only field the SCM fills when a status
        // is QUERIED; a reported status does not carry one (the
        // windows-service 0.7 `ServiceStatus` contract).
        process_id: None,
    };
    if let Err(error) = handle.set_service_status(status) {
        eprintln!("[mcpgo-service] cannot report the {state:?} status: {error}");
    }
}

/// Whether a stop request is pending on the channel.
fn stop_requested(receiver: &Receiver<()>) -> bool {
    receiver.try_recv() == Ok(())
}

/// Sleeps up to `duration`, but returns early (with `true`) as soon as a
/// stop request arrives. Returns `false` when the full duration elapsed
/// without a stop.
fn sleep_unless_stopped(duration: Duration, receiver: &Receiver<()>) -> bool {
    let deadline = std::time::Instant::now() + duration;
    loop {
        if stop_requested(receiver) {
            return true;
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return false;
        }
        thread::sleep(POLL_INTERVAL.min(deadline - now));
    }
}

// ----------------------------------------------------------------------------
// Definition loading and runtime process assembly
// ----------------------------------------------------------------------------

/// Path of the definition file this worker supervises.
fn definition_path(host_args: &ServiceHostArgs) -> PathBuf {
    host_args
        .data_dir
        .join("services")
        .join(format!("{}.json", host_args.id))
}

/// Path of the log file this worker writes to.
fn service_log_path(host_args: &ServiceHostArgs) -> PathBuf {
    host_args
        .data_dir
        .join("services")
        .join("logs")
        .join(format!("{}.log", host_args.id))
}

/// Working directory of the runtime process: the definition's explicit
/// choice, or the services directory as a stable default.
fn default_working_dir(host_args: &ServiceHostArgs) -> PathBuf {
    host_args.data_dir.join("services")
}

/// Reads and parses the service definition. Errors are plain strings:
/// they go straight into the log file and the SCM stops cleanly.
fn load_definition(host_args: &ServiceHostArgs) -> Result<ServiceDefinition, String> {
    let path = definition_path(host_args);
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|error| format!("corrupt service definition {}: {error}", path.display())),
        Err(error) => Err(format!(
            "cannot read the service definition {}: {error}",
            path.display()
        )),
    }
}

/// Resolves the runtime executable:
///
/// 1. the definition's explicit `runtime_path` (absolute) - the reliable
///    choice for services, whose LocalSystem PATH differs from the
///    installing user's;
/// 2. otherwise the standard machine resolution from `paths`.
fn resolve_runtime_exe(definition: &ServiceDefinition) -> Result<PathBuf, String> {
    if let Some(explicit) = definition
        .runtime_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(explicit));
    }
    match definition.runtime {
        RuntimeKind::Node => paths::node_path().ok_or_else(|| {
            "Node.js (node.exe) was not found on the PATH visible to this service; \
             set an absolute runtime path in the service definition"
                .to_string()
        }),
        RuntimeKind::Python => paths::python_path().ok_or_else(|| {
            "Python (python.exe) was not found on the PATH visible to this service; \
             set an absolute runtime path in the service definition"
                .to_string()
        }),
        RuntimeKind::PowerShell => {
            // Windows PowerShell 5.1 is used instead of pwsh: services
            // run as LocalSystem, and a per-user pwsh install is invisible
            // to that account, while 5.1 ships with every Windows.
            let exe = paths::powershell51_path();
            if exe.is_file() {
                Ok(exe)
            } else {
                Err("Windows PowerShell 5.1 (powershell.exe) was not found at its \
                     standard location; set an absolute runtime path in the service \
                     definition"
                    .to_string())
            }
        }
    }
}

/// Argument vector placed after the runtime executable. Script runtimes
/// take the command as their first argument; PowerShell runs it as a
/// command in a fresh, non-interactive process (same flags as the
/// terminal engine in `terminal/mod.rs`).
fn runtime_argv(runtime: RuntimeKind, command: &str, args: &[String]) -> Vec<String> {
    match runtime {
        RuntimeKind::Node | RuntimeKind::Python => {
            let mut argv = vec![command.to_owned()];
            argv.extend(args.iter().cloned());
            argv
        }
        RuntimeKind::PowerShell => {
            let mut argv: Vec<String> = [
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
            ]
            .iter()
            .map(|flag| (*flag).to_owned())
            .collect();
            argv.push(command.to_owned());
            argv.extend(args.iter().cloned());
            argv
        }
    }
}

/// Spawns the runtime process of `definition`: piped stdout/stderr (the
/// readers below stream them into the log), null stdin (services are not
/// interactive), definition environment on top of the inherited (system)
/// environment, and no console window.
fn spawn_runtime_process(
    definition: &ServiceDefinition,
    host_args: &ServiceHostArgs,
) -> Result<Child, String> {
    let exe = resolve_runtime_exe(definition)?;
    let working_dir = definition
        .working_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_working_dir(host_args));

    let mut command = Command::new(exe);
    command.args(runtime_argv(
        definition.runtime,
        &definition.command,
        &definition.args,
    ));
    for (key, value) in &definition.env {
        command.env(key, value);
    }
    command
        .current_dir(&working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
        .spawn()
        .map_err(|error| format!("cannot start the runtime process in {}: {error}", working_dir.display()))
}

// ----------------------------------------------------------------------------
// Logging
// ----------------------------------------------------------------------------

/// Append-only file logger shared by the reader threads. One
/// open-append-write per line (no locks, no handles kept open): NTFS
/// appends of line-sized payloads are atomic, and a crashed writer can
/// never leave the file locked - the same strategy as the job_logs
/// writer in `jobs/mod.rs`.
struct ServiceLogger {
    path: PathBuf,
}

impl ServiceLogger {
    fn new(host_args: &ServiceHostArgs) -> Self {
        let path = service_log_path(host_args);
        if let Some(parent) = path.parent() {
            // Best effort: a failing append reports itself below.
            let _ = fs::create_dir_all(parent);
        }
        Self { path }
    }

    fn append(&self, level: &str, text: &str) {
        let line = format!(
            "[{}] {} {}\n",
            time::rfc3339_utc(time::now_epoch_millis()),
            level,
            text
        );
        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| file.write_all(line.as_bytes()));
        if let Err(error) = result {
            eprintln!(
                "[mcpgo-service] cannot append to {}: {error}",
                self.path.display()
            );
        }
    }
}

/// Logs one record before/without a `ServiceLogger` exists (dispatcher
/// and definition-load failures). Rare by construction.
fn worker_log(host_args: &ServiceHostArgs, level: &str, text: &str) {
    ServiceLogger::new(host_args).append(level, text);
}

// ----------------------------------------------------------------------------
// Process termination
// ----------------------------------------------------------------------------

/// Terminates the whole process tree of `pid` (the runtime process may
/// have spawned children of its own). Output is discarded: taskkill's
/// stdout carries no information the log needs.
fn kill_process_tree(pid: u32) {
    let mut command = Command::new(TASKKILL);
    command
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    if let Err(error) = command.status() {
        eprintln!("[mcpgo-service] cannot terminate the process tree of {pid}: {error}");
    }
}

// ----------------------------------------------------------------------------
// Log readers
// ----------------------------------------------------------------------------

/// Spawns the stdout/stderr reader threads of a freshly started runtime
/// process. Both stream line by line into the shared logger as OUT/ERR
/// records.
fn spawn_log_readers(child: &mut Child, logger: &Arc<ServiceLogger>) -> Vec<JoinHandle<()>> {
    let mut handles = Vec::with_capacity(2);
    if let Some(stdout) = child.stdout.take() {
        handles.push(spawn_log_reader(stdout, logger.clone(), "OUT"));
    }
    if let Some(stderr) = child.stderr.take() {
        handles.push(spawn_log_reader(stderr, logger.clone(), "ERR"));
    }
    handles
}

/// Drains one pipe into the log. A thread-spawn failure is resource
/// exhaustion: panicking lets the worker die, and the SCM failure
/// recovery (installed with keep-alive) restarts it - that beats a
/// silent pipe nobody drains, which would deadlock the runtime on a full
/// 64 KB pipe buffer. A broken-UTF-8 line ends the stream for that pipe
/// (documented limitation, mirroring the job log readers).
fn spawn_log_reader<R: Read + Send + 'static>(
    pipe: R,
    logger: Arc<ServiceLogger>,
    level: &'static str,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name(format!("svc-log-{level}"))
        .spawn(move || {
            for line in BufReader::new(pipe).lines() {
                match line {
                    Ok(text) => logger.append(level, &text),
                    Err(_) => break,
                }
            }
        })
        .expect("service log reader thread spawns")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_a_full_worker_invocation() {
        let parsed = parse_service_host_args(&args(&[
            "--service-host",
            "weather-mcp",
            "--data-dir",
            r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows",
        ]))
        .expect("both flags present");
        assert_eq!(parsed.id, "weather-mcp");
        assert_eq!(
            parsed.data_dir,
            PathBuf::from(r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows")
        );
    }

    #[test]
    fn requires_both_flags_in_any_order() {
        assert!(parse_service_host_args(&args(&["--data-dir", "C:\\data"])).is_none());
        assert!(parse_service_host_args(&args(&["--service-host", "id"])).is_none());
        // Reversed order and unrelated arguments are tolerated.
        let parsed = parse_service_host_args(&args(&[
            "--data-dir",
            "C:\\data",
            "--unrelated",
            "value",
            "--service-host",
            "id",
        ]))
        .expect("order-agnostic parsing");
        assert_eq!(parsed.id, "id");
        assert_eq!(parsed.data_dir, PathBuf::from("C:\\data"));
    }

    #[test]
    fn gui_invocations_are_not_worker_mode() {
        assert!(parse_service_host_args(&[]).is_none());
        assert!(parse_service_host_args(&args(&["--flag", "value"])).is_none());
    }

    #[test]
    fn service_main_arguments_split_into_name_and_flags() {
        // The shape `define_windows_service!` hands over: the service
        // name first, then the ImagePath arguments.
        let arguments: Vec<std::ffi::OsString> = [
            "McpGo-weather-mcp",
            "--service-host",
            "weather-mcp",
            "--data-dir",
            r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows",
        ]
        .iter()
        .map(|value| (*value).to_owned().into())
        .collect();
        let (service_name, host_args) =
            parse_service_main_args(&arguments).expect("parseable service arguments");
        assert_eq!(service_name, "McpGo-weather-mcp");
        assert_eq!(host_args.id, "weather-mcp");
        assert_eq!(
            host_args.data_dir,
            PathBuf::from(r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows")
        );

        // Missing flags and empty argument vectors are plain errors.
        assert!(parse_service_main_args(&[]).is_err());
        let name_only: Vec<std::ffi::OsString> =
            ["McpGo-weather-mcp"].iter().map(|v| (*v).to_owned().into()).collect();
        assert!(parse_service_main_args(&name_only).is_err());
    }

    #[test]
    fn runtime_argv_for_script_runtimes() {
        let extra = args(&["--port", "3000"]);
        // node/python: the command becomes the first argument.
        assert_eq!(
            runtime_argv(RuntimeKind::Node, "server.js", &extra),
            args(&["server.js", "--port", "3000"])
        );
        assert_eq!(
            runtime_argv(RuntimeKind::Python, "server.py", &[]),
            args(&["server.py"])
        );
        // PowerShell: the command runs through the same non-interactive
        // flag set as the terminal engine.
        assert_eq!(
            runtime_argv(RuntimeKind::PowerShell, "Get-Process", &extra),
            args(&[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "Get-Process",
                "--port",
                "3000"
            ])
        );
    }

    #[test]
    fn log_and_definition_paths_live_under_the_data_dir() {
        let host_args = ServiceHostArgs {
            id: "weather-mcp".into(),
            data_dir: PathBuf::from(r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows"),
        };
        assert_eq!(
            definition_path(&host_args),
            PathBuf::from(r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows")
                .join("services")
                .join("weather-mcp.json")
        );
        assert_eq!(
            service_log_path(&host_args),
            PathBuf::from(r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows")
                .join("services")
                .join("logs")
                .join("weather-mcp.log")
        );
        assert_eq!(
            default_working_dir(&host_args),
            PathBuf::from(r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows").join("services")
        );
    }
}
