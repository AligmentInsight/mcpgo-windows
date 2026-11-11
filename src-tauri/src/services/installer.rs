//! The write side of the Service Control Manager: install / uninstall /
//! start / stop / restart of McpGo services through elevated PowerShell
//! management scripts.
//!
//! # Flow of one operation
//!
//! ```text
//! IPC command (commands/services.rs)
//!   -> installer::install_service / control_service / uninstall_service
//!     -> run_script: materialize the .ps1, spawn PowerShell with -File
//!     -> the script self-elevates (UAC prompt) when needed and prints
//!        one JSON result line
//!     -> parse_script_result turns stdout into ok/detail
//!   -> ServiceStore::summary_with_live_scm confirms the new state
//! ```
//!
//! # Why scripts, and why `-File`
//!
//! The scripts (services/scripts/*.ps1) carry the UAC self-elevation
//! dance (`Start-Process -Verb RunAs`) and the SCM interaction rules
//! (exit-code tolerances, stop-wait polling). They are embedded with
//! `include_str!` - the installed application needs no extra resources -
//! and materialized into `<data>\services\scripts\` before every run.
//! The re-write on every run is deliberate: it is idempotent, cheap
//! (three small files) and self-heals manual edits that would otherwise
//! drift away from the Rust expectations.
//!
//! Parameters are passed as separate process arguments (never string-
//! interpolated into the command line), so paths with spaces, quotes or
//! non-ASCII characters travel safely: `std::process::Command` performs
//! the native Windows argument quoting.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::error::{McpGoError, McpGoResult};
use crate::paths::{self, ShellKind};
use crate::services::definition::ServiceStore;
use crate::services::ServiceSummary;
use crate::terminal::{self, CapturedOutput};

/// Embedded install script (see module docs).
const SCRIPT_INSTALL: &str = include_str!("scripts/Install-McpGoService.ps1");
/// Embedded uninstall script (see module docs).
const SCRIPT_UNINSTALL: &str = include_str!("scripts/Uninstall-McpGoService.ps1");
/// Embedded start/stop/restart script (see module docs).
const SCRIPT_CONTROL: &str = include_str!("scripts/Control-McpGoService.ps1");

/// Budget for one elevated operation. The UAC consent prompt may sit on
/// screen for a while before the user reacts; the margin only catches a
/// genuinely hung elevated instance.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(180);

/// One management action, executed by the control script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

impl ServiceAction {
    /// Parameter value of Control-McpGoService.ps1.
    fn script_value(self) -> &'static str {
        match self {
            ServiceAction::Start => "Start",
            ServiceAction::Stop => "Stop",
            ServiceAction::Restart => "Restart",
        }
    }

    /// Human-readable name for log records and error messages.
    fn label(self) -> &'static str {
        match self {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Restart => "restart",
        }
    }
}

/// Parsed result of one script run.
#[derive(Debug, Clone)]
struct ScriptResult {
    /// Whether the operation succeeded (the script's `ok` field, ANDed
    /// with a zero process exit code for robustness).
    ok: bool,
    /// The script's detail line (error text or confirmation note).
    detail: String,
}

// ----------------------------------------------------------------------------
// Script materialization and execution
// ----------------------------------------------------------------------------

/// Writes every embedded script into `<services>\scripts\`. See the
/// module docs for why this happens before every run.
fn materialize_scripts(services_dir: &Path) -> McpGoResult<PathBuf> {
    let scripts_dir = services_dir.join("scripts");
    fs::create_dir_all(&scripts_dir)
        .map_err(|e| McpGoError::io("create the services scripts directory", &e))?;
    let scripts = [
        ("Install-McpGoService.ps1", SCRIPT_INSTALL),
        ("Uninstall-McpGoService.ps1", SCRIPT_UNINSTALL),
        ("Control-McpGoService.ps1", SCRIPT_CONTROL),
    ];
    for (file_name, content) in scripts {
        fs::write(scripts_dir.join(file_name), content)
            .map_err(|e| McpGoError::io("materialize the service management scripts", &e))?;
    }
    Ok(scripts_dir)
}

/// Spawns `shell` running `script_path` with `-File` and the given
/// parameter arguments, capturing the output under `timeout`. Argument
/// passing is native (one `Command::arg` per token), so no quoting
/// hazards exist (see module docs).
fn run_powershell_script(
    shell: &ShellKind,
    script_path: &Path,
    args: &[String],
    timeout: Duration,
) -> McpGoResult<CapturedOutput> {
    let mut command = Command::new(&shell.exe);
    command
        // Same deterministic host flags as the terminal engine
        // (terminal/mod.rs), so script behaviour never depends on user
        // profiles.
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(script_path);
    for argument in args {
        command.arg(argument);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // The GUI must stay foreground: the script runs consoleless. (The
        // script itself relaunches an elevated instance when needed.)
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let child = command
        .spawn()
        .map_err(|e| McpGoError::io("start the service management script", &e))?;
    terminal::capture_child(child, timeout)
        .map_err(|e| McpGoError::io("run the service management script", &e))
}

/// Runs one materialized script and parses its JSON result line. The
/// file name is used for logging and error messages only.
fn run_script(
    store: &ServiceStore,
    file_name: &str,
    args: &[String],
) -> McpGoResult<ScriptResult> {
    let shell = paths::preferred_shell().ok_or_else(|| McpGoError::ComponentMissing {
        component: "powershell".into(),
        detail: "neither PowerShell 7 nor Windows PowerShell 5.1 was found".into(),
    })?;
    let scripts_dir = materialize_scripts(store.dir())?;
    let captured = run_powershell_script(&shell, &scripts_dir.join(file_name), args, OPERATION_TIMEOUT)?;

    if captured.timed_out {
        return Err(McpGoError::operation(
            format!("running {file_name}"),
            "the script did not finish in time (is the UAC prompt waiting?)",
        ));
    }

    // The script prints exactly one JSON line: either its own result
    // (already-elevated run) or the relayed result of the elevated
    // instance. Taking the LAST non-empty line is tolerant of stray
    // warnings without ever reading past the result.
    let result_line = captured
        .stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty());
    let Some(result_line) = result_line else {
        return Err(McpGoError::operation(
            format!("running {file_name}"),
            format!(
                "the script produced no result (exit code {:?}): {}",
                captured.exit_code,
                captured.stderr.trim()
            ),
        ));
    };

    let value: serde_json::Value = serde_json::from_str(result_line.trim()).map_err(|e| {
        McpGoError::internal(format!("{file_name} returned invalid JSON: {e}"))
    })?;
    // The scripts exit 0 on success and 1 on failure; honouring both
    // channels makes the parse robust against a script bug.
    let ok = value.get("ok").and_then(serde_json::Value::as_bool) == Some(true)
        && captured.exit_code == Some(0);
    let detail = value
        .get("detail")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(ScriptResult { ok, detail })
}

/// `'true'`/`'false'` parameter token (the scripts parse these strings
/// explicitly - see ConvertTo-McpGoFlag in Install-McpGoService.ps1).
fn flag_token(value: bool) -> String {
    if value { "true" } else { "false" }.to_owned()
}

/// Display texts travelling through the elevated relaunch (its
/// `-ArgumentList` string cannot carry embedded double quotes): quotes
/// become apostrophes, mirroring the sanitising inside the script itself.
fn safe_display(text: &str) -> String {
    text.replace('"', "'")
}

// ----------------------------------------------------------------------------
// Public operations (one per IPC command)
// ----------------------------------------------------------------------------

/// Installs the service described by definition `id`: registers the SCM
/// service `McpGo-<id>` whose ImagePath starts this executable in
/// service-worker mode. Returns the refreshed summary (the service
/// exists and is Stopped unless it was installed with autostart).
pub fn install_service(store: &ServiceStore, id: &str) -> McpGoResult<ServiceSummary> {
    let definition = store.get(id)?;
    let worker = std::env::current_exe()
        .map_err(|e| McpGoError::io("resolve the McpGo executable path", &e))?;
    let args = vec![
        "-Id".into(),
        definition.id.clone(),
        "-WorkerPath".into(),
        worker.to_string_lossy().into_owned(),
        "-DataDir".into(),
        store.data_dir().to_string_lossy().into_owned(),
        "-DisplayName".into(),
        safe_display(&definition.name),
        "-Description".into(),
        safe_display(&definition.description),
        "-Autostart".into(),
        flag_token(definition.autostart),
        "-KeepAlive".into(),
        flag_token(definition.keep_alive),
    ];

    log::info!(
        "[services] installing service {} (autostart {}, keep-alive {})",
        definition.service_name(),
        definition.autostart,
        definition.keep_alive
    );
    let result = run_script(store, "Install-McpGoService.ps1", &args)?;
    if !result.ok {
        return Err(McpGoError::operation(
            format!("installing service {}", definition.service_name()),
            result.detail,
        ));
    }
    log::info!("[services] service installed: {}", result.detail);
    store.summary_with_live_scm(id)
}

/// Uninstalls the service of definition `id` (stop + delete). The
/// definition file itself stays - a later reinstall reuses it.
pub fn uninstall_service(store: &ServiceStore, id: &str) -> McpGoResult<ServiceSummary> {
    let definition = store.get(id)?;
    let args = vec!["-Id".into(), definition.id.clone()];

    log::info!("[services] uninstalling service {}", definition.service_name());
    let result = run_script(store, "Uninstall-McpGoService.ps1", &args)?;
    if !result.ok {
        return Err(McpGoError::operation(
            format!("uninstalling service {}", definition.service_name()),
            result.detail,
        ));
    }
    log::info!("[services] service uninstalled: {}", result.detail);
    store.summary_with_live_scm(id)
}

/// Starts, stops or restarts the service of definition `id`.
pub fn control_service(
    store: &ServiceStore,
    id: &str,
    action: ServiceAction,
) -> McpGoResult<ServiceSummary> {
    let definition = store.get(id)?;
    let args = vec![
        "-Id".into(),
        definition.id.clone(),
        "-Action".into(),
        action.script_value().to_owned(),
    ];

    log::info!(
        "[services] {} service {}",
        action.label(),
        definition.service_name()
    );
    let result = run_script(store, "Control-McpGoService.ps1", &args)?;
    if !result.ok {
        return Err(McpGoError::operation(
            format!("{}ing service {}", action.label(), definition.service_name()),
            result.detail,
        ));
    }
    log::info!("[services] service {}: {}", action.label(), result.detail);
    store.summary_with_live_scm(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_tokens_are_explicit_strings() {
        // The scripts refuse implicit boolean coercion ('false' as a
        // non-empty string would bind as $true); the tokens must match
        // their ConvertTo-McpGoFlag whitelist exactly.
        assert_eq!(flag_token(true), "true");
        assert_eq!(flag_token(false), "false");
    }

    #[test]
    fn display_texts_lose_their_embedded_quotes() {
        // Quotes cannot survive the elevated -ArgumentList relay; they are
        // replaced by apostrophes before the script ever sees the value.
        assert_eq!(safe_display("My \"best\" server"), "My 'best' server");
        assert_eq!(safe_display("plain"), "plain");
    }

    #[test]
    fn action_values_match_the_script_parameter_set() {
        assert_eq!(ServiceAction::Start.script_value(), "Start");
        assert_eq!(ServiceAction::Stop.script_value(), "Stop");
        assert_eq!(ServiceAction::Restart.script_value(), "Restart");
    }

    #[test]
    fn embedded_scripts_are_nonempty_and_named() {
        // Guards against an accidentally empty script file slipping
        // through a merge: the installer would materialize nothing and
        // every operation would fail cryptically on the user's machine.
        for (name, content) in [
            ("Install-McpGoService.ps1", SCRIPT_INSTALL),
            ("Uninstall-McpGoService.ps1", SCRIPT_UNINSTALL),
            ("Control-McpGoService.ps1", SCRIPT_CONTROL),
        ] {
            assert!(content.len() > 500, "{name} looks truncated");
            assert!(content.contains("Write-McpGoResult"), "{name} lost its result helper");
        }
    }

    #[test]
    fn install_arguments_carry_both_flags() {
        // Mirrors the argument assembly of install_service without
        // touching the file system or PowerShell.
        let args = vec![
            "-Id".to_owned(),
            "weather".to_owned(),
            "-WorkerPath".to_owned(),
            r"C:\Program Files\McpGo\McpGo.exe".to_owned(),
            "-DataDir".to_owned(),
            r"C:\Users\dev\AppData\Roaming\com.mcpgo.windows".to_owned(),
            "-DisplayName".to_owned(),
            "Weather MCP".to_owned(),
            "-Description".to_owned(),
            String::new(),
            "-Autostart".to_owned(),
            flag_token(true),
            "-KeepAlive".to_owned(),
            flag_token(true),
        ];
        // Parameter/value pairs stay aligned.
        assert_eq!(args.len() % 2, 0);
        for pair in args.chunks(2) {
            assert!(pair[0].starts_with('-'), "misaligned pair: {pair:?}");
        }
        assert!(args.contains(&"true".to_owned()));
    }
}
