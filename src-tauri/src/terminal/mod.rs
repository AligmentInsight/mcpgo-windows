//! PowerShell process launcher - the Windows replacement of the macOS
//! Bash/Zsh backend.
//!
//! Every job (see `crate::jobs`) runs one command in a fresh PowerShell
//! process with:
//!
//! * `-NoLogo -NoProfile -NonInteractive` - deterministic, script-friendly
//!   behaviour with no profile side effects;
//! * `-ExecutionPolicy Bypass` - user commands may invoke local scripts;
//! * piped stdout/stderr (drained line-by-line by the job's reader
//!   threads) and a null stdin;
//! * `CREATE_NO_WINDOW` so no console window flashes behind the GUI.
//!
//! Trust model: identical to the macOS original. The commands executed
//! here are entered by the local user in the McpGo UI, i.e. they run with
//! the user's own privileges on their own machine. No untrusted input
//! reaches this module.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::paths::ShellKind;

/// Windows process creation flag CREATE_NO_WINDOW (0x0800_0000): keeps the
/// child consoleless so nothing flashes behind the GUI window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Poll interval of [`run_capture`]'s exit watcher; also the granularity of
/// its timeout.
const CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Spawns `shell` executing `command`, optionally in `working_dir`.
///
/// The child is configured with piped stdout/stderr (the caller must drain
/// both to avoid the 64 KB pipe buffer blocking the process) and a null
/// stdin (jobs are one-shot commands, not interactive sessions).
pub fn spawn_shell(
    shell: &ShellKind,
    command: &str,
    working_dir: Option<&Path>,
) -> io::Result<Child> {
    let mut builder = Command::new(&shell.exe);
    builder
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(command);
    if let Some(dir) = working_dir {
        builder.current_dir(dir);
    }
    builder
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        builder.creation_flags(CREATE_NO_WINDOW);
    }

    builder.spawn()
}

// ----------------------------------------------------------------------------
// Synchronous capture runs
// ----------------------------------------------------------------------------

/// Output collected by [`run_capture`].
#[derive(Debug, Clone)]
pub struct CapturedOutput {
    /// stdout of the child, UTF-8 lossily decoded.
    pub stdout: String,
    /// stderr of the child, UTF-8 lossily decoded.
    pub stderr: String,
    /// Exit code of the finished child; `None` when the platform reports
    /// none (a killed process, or a termination without a code).
    pub exit_code: Option<i32>,
    /// `true` when the child exceeded the timeout and was terminated by
    /// this side.
    pub timed_out: bool,
}

/// Spawns one reader thread that drains `pipe` to EOF. Returning the
/// handle (instead of panicking on failure) keeps [`run_capture`]
/// failure-safe: a failed reader is reported as an ordinary io error.
fn spawn_capture_reader<R: Read + Send + 'static>(
    pipe: R,
) -> io::Result<JoinHandle<io::Result<Vec<u8>>>> {
    thread::Builder::new()
        .name("capture-reader".into())
        .spawn(move || {
            let mut buffer = Vec::new();
            let mut reader = pipe;
            reader.read_to_end(&mut buffer)?;
            Ok(buffer)
        })
}

/// Runs `command` in `shell` to completion, capturing stdout and stderr.
/// Timeout-bounded sibling of [`spawn_shell`] for internal callers (e.g.
/// the SCM status query in `crate::services::scm`) that need the OUTPUT of
/// a PowerShell invocation synchronously.
///
/// Design notes:
///
/// * both pipes are drained by dedicated threads, so a chatty child can
///   never deadlock on a full pipe buffer while we poll for its exit;
/// * `Child::wait_with_output` is NOT used: it cannot be interrupted, so a
///   hung child would hang the caller forever;
/// * on timeout the child is terminated and `timed_out` is set - callers
///   decide whether that is an error;
/// * a wait failure kills the child, drains the (now detached) readers
///   implicitly and surfaces the io error.
pub fn run_capture(
    shell: &ShellKind,
    command: &str,
    working_dir: Option<&Path>,
    timeout: Duration,
) -> io::Result<CapturedOutput> {
    let child = spawn_shell(shell, command, working_dir)?;
    capture_child(child, timeout)
}

/// Waits for an ALREADY SPAWNED child with piped stdout/stderr, draining
/// both pipes and enforcing `timeout`. The engine behind [`run_capture`],
/// extracted so callers that spawn their PowerShell differently (e.g. the
/// service management scripts run with `-File` by
/// `crate::services::installer`) reuse the identical drain/poll logic.
///
/// The child MUST have piped stdout/stderr (they are taken from the
/// `Child`); stdin is left as the caller configured it.
pub fn capture_child(mut child: Child, timeout: Duration) -> io::Result<CapturedOutput> {
    let stdout_pipe = child.stdout.take().expect("stdout must be piped");
    let stderr_pipe = child.stderr.take().expect("stderr must be piped");

    // Reader threads keep the pipes empty while the poll loop watches the
    // child. A failure to spawn one leaves the process undrainable: the
    // child is killed immediately (a started reader then drains to EOF and
    // exits) and the error is reported.
    let (stdout_reader, stderr_reader) =
        match (spawn_capture_reader(stdout_pipe), spawn_capture_reader(stderr_pipe)) {
            (Ok(out), Ok(err)) => (out, err),
            (out, err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(out
                    .err()
                    .or_else(|| err.err())
                    .expect("at least one capture reader failed"));
            }
        };

    let started = Instant::now();
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok().and_then(|status| status.code());
                }
                thread::sleep(CAPTURE_POLL_INTERVAL);
            }
            Err(e) => {
                // Unrecoverable wait failure: kill, let the detached
                // readers finish on their own and surface the error.
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        }
    };

    // Join the readers. A panicked thread yields the default (empty) io
    // result, an io failure inside a reader yields an empty buffer - in
    // both cases partial output beats failing the whole run.
    let stdout = String::from_utf8_lossy(
        &stdout_reader.join().unwrap_or_default().unwrap_or_default(),
    )
    .into_owned();
    let stderr = String::from_utf8_lossy(
        &stderr_reader.join().unwrap_or_default().unwrap_or_default(),
    )
    .into_owned();

    Ok(CapturedOutput {
        stdout,
        stderr,
        exit_code,
        timed_out,
    })
}
