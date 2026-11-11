//! Terminal IPC surface: the PowerShell job lifecycle used by the frontend.
//!
//! Mapping to the macOS original:
//!
//! | macOS concept              | Command here                       |
//! |----------------------------|------------------------------------|
//! | run a command in Bash/Zsh  | [`start_job`]                       |
//! | job_status file            | status of the returned `JobStatus`  |
//! | job_logs file              | [`read_job_log`]                    |
//! | cancel a background task   | [`cancel_job`]                      |
//!
//! Trust model: identical to the macOS original - the commands executed
//! here are entered by the local user in the McpGo UI and run with the
//! user's own privileges. No untrusted input reaches these handlers.

use tauri::State;

use crate::error::McpGoResult;
use crate::jobs::{JobLog, JobRegistry, JobStatus};

/// Starts a new PowerShell job.
///
/// Returns the initial `Running` status. Output arrives through
/// `job://output` events (invalidation signals - the log file is the
/// source of truth) and is read back with [`read_job_log`]. State
/// transitions are announced through `job://status` events.
#[tauri::command]
pub async fn start_job(
    registry: State<'_, JobRegistry>,
    command: String,
    working_dir: Option<String>,
    title: Option<String>,
) -> McpGoResult<JobStatus> {
    registry.start_job(&command, working_dir.as_deref(), title.as_deref())
}

/// Requests cancellation of a running job.
///
/// Returns the current status (still `Running`, but with
/// `cancelRequested: true`); the final `Canceled` status is delivered
/// through a `job://status` event once the process has terminated.
#[tauri::command]
pub async fn cancel_job(
    registry: State<'_, JobRegistry>,
    job_id: String,
) -> McpGoResult<JobStatus> {
    registry.cancel_job(&job_id)
}

/// Returns the status of one job (running jobs from memory, finished or
/// previous-session jobs from their job_status file).
#[tauri::command]
pub async fn get_job(
    registry: State<'_, JobRegistry>,
    job_id: String,
) -> McpGoResult<JobStatus> {
    registry.get_job(&job_id)
}

/// Lists known jobs, newest first (current session and previous sessions,
/// capped by the registry).
///
/// Returns `McpGoResult` (like every async command borrowing `State`):
/// Tauri 2 requires async commands with borrowed arguments to return a
/// `Result`.
#[tauri::command]
pub async fn list_jobs(registry: State<'_, JobRegistry>) -> McpGoResult<Vec<JobStatus>> {
    Ok(registry.list_jobs())
}

/// Returns the parsed job_logs records of one job (header, output lines and
/// footer), oldest first.
#[tauri::command]
pub async fn read_job_log(
    registry: State<'_, JobRegistry>,
    job_id: String,
) -> McpGoResult<JobLog> {
    registry.read_job_log(&job_id)
}
