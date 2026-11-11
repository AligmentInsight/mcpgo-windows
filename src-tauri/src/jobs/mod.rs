//! Background job registry - the Windows port of the macOS job_status /
//! job_logs tracking system.
//!
//! A *job* is one PowerShell command execution (see `crate::terminal` for
//! the spawn mechanics):
//!
//! * `jobs\status\<id>.json` - the job_status file, rewritten atomically
//!   (tmp file + rename) on every state transition;
//! * `jobs\logs\<id>.log` - the job_logs file, append-only, one record per
//!   line in the canonical format `[RFC3339-UTC] OUT|ERR|SYS <text>`.
//!
//! The frontend receives two Tauri events:
//!
//! * `job://status` (payload: the new `JobStatus`) on every transition;
//! * `job://output` (payload: `{ jobId }`) as an *invalidation signal*:
//!   the log file is the single source of truth, the UI re-reads it via
//!   the `read_job_log` command (debounced). This keeps live rendering and
//!   full history consistent without any merge logic on either side.
//!
//! Lifecycle:
//!
//! ```text
//! Running --(exit code 0)------> Completed
//! Running --(exit code != 0)--> Failed
//! Running --(cancel_job)------> Canceled
//! ```
//!
//! Jobs still marked `Running` in their files after an application restart
//! are reconciled to `Canceled` when the registry is created.
//!
//! Concurrency notes:
//!
//! * only RUNNING jobs live in the in-memory map (needed to reach their
//!   child process); finished jobs are file-only;
//! * lock order is always "registry map first, then slot fields" - no
//!   code path locks a slot and then the map, so deadlocks are impossible;
//! * the waiter thread polls `try_wait` every 100 ms, which doubles as
//!   the cancellation checkpoint (`cancel` is an atomic flag).
//!
//! Known limitation (identical in spirit to the macOS original): killing a
//! job terminates the PowerShell process itself; grandchild processes that
//! PowerShell spawned (e.g. `Start-Process notepad`) may outlive it.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::error::{McpGoError, McpGoResult};
use crate::paths;
use crate::terminal;
use crate::time as job_time;

/// Tauri event: a job changed its state. Payload: [`JobStatusEvent`].
pub const EVENT_JOB_STATUS: &str = "job://status";
/// Tauri event: a job appended output. Payload: [`JobOutputEvent`].
pub const EVENT_JOB_OUTPUT: &str = "job://output";

/// Maximum accepted command length (bytes). Generous for scripts, small
/// enough to keep IPC payloads sane.
const MAX_COMMAND_LEN: usize = 65_536;
/// Maximum title length derived from the command's first line.
const MAX_TITLE_CHARS: usize = 60;
/// How many jobs `list_jobs` returns (newest first).
const LIST_LIMIT: usize = 100;
/// Waiter poll interval: process-exit detection and cancellation latency.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

// ----------------------------------------------------------------------------
// Wire types (shared with the frontend; keep src/types/job.ts in sync)
// ----------------------------------------------------------------------------

/// Lifecycle state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Running,
    Completed,
    Failed,
    Canceled,
}

/// Contents of one job_status file, also exchanged over IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    /// Generated id, e.g. `job_1731234567890_3`.
    pub id: String,
    /// Human-readable title (explicit or derived from the command).
    pub title: String,
    /// The full PowerShell command.
    pub command: String,
    /// Shell that executes the job ("pwsh-7" | "powershell-5.1").
    pub shell: String,
    pub state: JobState,
    pub exit_code: Option<i32>,
    /// Set as soon as cancellation is requested.
    #[serde(default)]
    pub cancel_requested: bool,
    /// Creation time, epoch milliseconds.
    pub created_at: u64,
    /// Process spawn time, epoch milliseconds.
    pub started_at: u64,
    /// Termination time, epoch milliseconds.
    pub finished_at: Option<u64>,
    /// Absolute path of the job_logs file.
    pub log_file: String,
    /// Absolute working directory, when one was requested.
    pub working_dir: Option<String>,
    /// Non-fatal infrastructure error recorded for the UI.
    #[serde(default)]
    pub error: Option<String>,
}

/// Output stream a job_logs record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogStream {
    Out,
    Err,
    Sys,
}

/// One parsed job_logs record.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLogLine {
    /// RFC 3339 timestamp as written in the file, when parseable.
    pub time: Option<String>,
    pub stream: LogStream,
    pub text: String,
}

/// Response of the `read_job_log` command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLog {
    pub job_id: String,
    pub lines: Vec<JobLogLine>,
}

/// Payload of the `job://status` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatusEvent {
    pub job: JobStatus,
}

/// Payload of the `job://output` event (invalidation signal).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobOutputEvent {
    pub job_id: String,
}

// ----------------------------------------------------------------------------
// Registry
// ----------------------------------------------------------------------------

/// Shared map of running jobs: the registry owns one clone, every waiter
/// thread owns another so it can remove its job after finalization.
type RunningMap = Arc<Mutex<HashMap<String, Arc<JobSlot>>>>;

/// Per-running-job shared state.
struct JobSlot {
    /// Current status; every mutation rewrites the status file and emits
    /// `job://status`.
    status: Mutex<JobStatus>,
    /// Append handle of the job_logs file. `None` once closed (after the
    /// footer record) or after an abort cleanup.
    log: Mutex<Option<File>>,
    /// Cancellation flag; checked by the waiter thread.
    cancel: AtomicBool,
    /// The child process; `None` once waited on or after an abort cleanup.
    child: Mutex<Option<Child>>,
}

/// Owns the job_status/job_logs directories and the running-job map.
/// Managed as Tauri state in `main.rs`.
pub struct JobRegistry {
    app: AppHandle,
    status_dir: PathBuf,
    logs_dir: PathBuf,
    running: RunningMap,
    counter: AtomicU64,
}

impl JobRegistry {
    /// Creates the registry and reconciles jobs left in `Running` state by
    /// a previous application instance (the waiter threads died with the
    /// app, so those processes are gone or detached).
    pub fn new(app: AppHandle, data_dir: PathBuf) -> Self {
        let registry = Self {
            app,
            status_dir: data_dir.join("jobs").join("status"),
            logs_dir: data_dir.join("jobs").join("logs"),
            running: Arc::new(Mutex::new(HashMap::new())),
            counter: AtomicU64::new(0),
        };
        registry.reconcile_orphans();
        registry
    }

    /// Marks job_status files that claim `Running` without a live waiter
    /// as `Canceled`, so the UI never shows ghost jobs after a restart.
    fn reconcile_orphans(&self) {
        let mut reconciled = 0usize;
        let entries = match fs::read_dir(&self.status_dir) {
            Ok(entries) => entries,
            // Missing directory: nothing to reconcile.
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else { continue };
            let Ok(mut status) = serde_json::from_str::<JobStatus>(&raw) else { continue };
            if status.state != JobState::Running {
                continue;
            }
            status.state = JobState::Canceled;
            status.cancel_requested = true;
            status.finished_at = Some(job_time::now_epoch_millis());
            status.error =
                Some("canceled: application was restarted while the job was running".into());
            if Self::write_status_file(&path, &status).is_ok() {
                let _ = self
                    .app
                    .emit(EVENT_JOB_STATUS, JobStatusEvent { job: status });
                reconciled += 1;
            }
        }
        if reconciled > 0 {
            log::warn!("[jobs] reconciled {reconciled} orphaned job(s) from a previous run");
        } else {
            log::info!("[jobs] no orphaned jobs from a previous run");
        }
    }

    /// Starts a new job: validates input, spawns PowerShell, creates the
    /// job_logs/job_status files, wires the reader and waiter threads and
    /// returns the initial `Running` status.
    pub fn start_job(
        &self,
        command: &str,
        working_dir: Option<&str>,
        title: Option<&str>,
    ) -> McpGoResult<JobStatus> {
        let command = command.trim();
        if command.is_empty() {
            return Err(McpGoError::invalid_input("command must not be empty"));
        }
        if command.len() > MAX_COMMAND_LEN {
            return Err(McpGoError::invalid_input(format!(
                "command is too long ({} bytes, limit {MAX_COMMAND_LEN} bytes)",
                command.len()
            )));
        }

        // Validate the working directory when provided.
        let working_dir_path = match working_dir.map(str::trim) {
            Some(dir) if !dir.is_empty() => {
                let path = PathBuf::from(dir);
                if !path.is_absolute() {
                    return Err(McpGoError::invalid_input(
                        "working directory must be an absolute path",
                    ));
                }
                if !path.is_dir() {
                    return Err(McpGoError::invalid_input(format!(
                        "working directory does not exist: {dir}"
                    )));
                }
                Some(path)
            }
            _ => None,
        };

        let shell = paths::preferred_shell().ok_or_else(|| McpGoError::ComponentMissing {
            component: "powershell".into(),
            detail: "neither PowerShell 7 nor Windows PowerShell 5.1 was found".into(),
        })?;

        let now = job_time::now_epoch_millis();
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        let id = format!("job_{now}_{seq}");
        let title = title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| truncate_title(command));
        let log_path = self.logs_dir.join(format!("{id}.log"));
        let status_path = self.status_dir.join(format!("{id}.json"));

        // 1. Create the job_logs file and write the header record. Doing
        //    this before the spawn keeps failure handling simple: no child,
        //    no status file - just delete the log again.
        let mut log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| McpGoError::io("create job_logs file", &e))?;
        let status = JobStatus {
            id: id.clone(),
            title: title.clone(),
            command: command.to_owned(),
            shell: shell.name.to_owned(),
            state: JobState::Running,
            exit_code: None,
            cancel_requested: false,
            created_at: now,
            started_at: job_time::now_epoch_millis(),
            finished_at: None,
            log_file: log_path.to_string_lossy().into_owned(),
            working_dir: working_dir_path
                .as_deref()
                .map(|p| p.to_string_lossy().into_owned()),
            error: None,
        };
        let header_dir = status
            .working_dir
            .clone()
            .unwrap_or_else(|| "(inherited working directory)".into());
        let _ = writeln!(
            log_file,
            "[{}] SYS job {} started on {} in {}",
            job_time::rfc3339_utc(status.started_at),
            status.id,
            shell.name,
            header_dir
        );
        let _ = log_file.flush();

        // 2. Spawn PowerShell. On failure the log file is removed so no
        //    orphan artefacts remain.
        let mut child = match terminal::spawn_shell(&shell, command, working_dir_path.as_deref())
        {
            Ok(child) => child,
            Err(e) => {
                let _ = fs::remove_file(&log_path);
                return Err(McpGoError::io("spawn powershell", &e));
            }
        };
        let stdout = child.stdout.take().expect("stdout must be piped");
        let stderr = child.stderr.take().expect("stderr must be piped");

        // 3. Persist the initial job_status file. A failure here (e.g. a
        //    read-only disk) must not leak the already-spawned process: the
        //    child is killed and reaped, and the log file removed - the same
        //    cleanup as for a spawn failure above.
        if let Err(error) = Self::write_status_file(&status_path, &status) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(&log_path);
            return Err(error);
        }

        // 4. Register the slot and spawn the worker threads.
        let slot = Arc::new(JobSlot {
            status: Mutex::new(status.clone()),
            log: Mutex::new(Some(log_file)),
            cancel: AtomicBool::new(false),
            child: Mutex::new(Some(child)),
        });
        self.running
            .lock()
            .expect("job registry map lock poisoned")
            .insert(id.clone(), Arc::clone(&slot));

        let out_handle = thread::Builder::new()
            .name(format!("{id}-stdout"))
            .spawn({
                let app = self.app.clone();
                let slot = Arc::clone(&slot);
                let job_id = id.clone();
                move || stream_output(app, slot, job_id, Box::new(stdout), LogStream::Out)
            });
        let err_handle = thread::Builder::new()
            .name(format!("{id}-stderr"))
            .spawn({
                let app = self.app.clone();
                let slot = Arc::clone(&slot);
                let job_id = id.clone();
                move || stream_output(app, slot, job_id, Box::new(stderr), LogStream::Err)
            });
        let readers = match (out_handle, err_handle) {
            (Ok(out), Err(e)) | (Err(e), Ok(out)) => {
                // One reader started and is detached. Kill the child FIRST
                // (the reader then drains to EOF and exits), join it, and
                // report the failure.
                let error = self.abort_job(
                    &slot,
                    &id,
                    &log_path,
                    &status_path,
                    &format!("failed to spawn a job reader thread: {e}"),
                );
                let _ = out.join();
                return Err(error);
            }
            (Ok(out), Ok(err)) => vec![out, err],
            (Err(out_err), Err(err_err)) => {
                return Err(self.abort_job(
                    &slot,
                    &id,
                    &log_path,
                    &status_path,
                    &format!(
                        "failed to spawn job reader threads: {out_err}; {err_err}"
                    ),
                ));
            }
        };

        let waiter = thread::Builder::new()
            .name(format!("{id}-waiter"))
            .spawn({
                let app = self.app.clone();
                let slot = Arc::clone(&slot);
                let job_id = id.clone();
                let running = Arc::clone(&self.running);
                let status_path = status_path.clone();
                move || wait_and_finalize(app, slot, running, status_path, job_id, readers)
            });
        if let Err(e) = waiter {
            // The closure (and the readers vector) was dropped; readers are
            // detached and will drain to EOF once the child is killed.
            return Err(self.abort_job(
                &slot,
                &id,
                &log_path,
                &status_path,
                &format!("failed to spawn the job waiter thread: {e}"),
            ));
        }

        // 5. Everything is wired up: announce the job to the frontend.
        let _ = self
            .app
            .emit(EVENT_JOB_STATUS, JobStatusEvent { job: status.clone() });
        log::info!(
            "[jobs] {id} started on {} ({}), log: {}",
            shell.name,
            status.title,
            status.log_file
        );
        Ok(status)
    }

    /// Requests cancellation of a running job. The waiter thread terminates
    /// the process within one poll interval; the final `Canceled` status
    /// arrives through `job://status`.
    pub fn cancel_job(&self, job_id: &str) -> McpGoResult<JobStatus> {
        let slot = self
            .running
            .lock()
            .expect("job registry map lock poisoned")
            .get(job_id)
            .cloned()
            .ok_or_else(|| McpGoError::job_not_found(job_id))?;

        let status = {
            let mut status_guard = slot.status.lock().expect("job status lock poisoned");
            if status_guard.state != JobState::Running {
                return Ok(status_guard.clone());
            }
            status_guard.cancel_requested = true;
            let status = status_guard.clone();
            let status_path = self.status_dir.join(format!("{}.json", job_id));
            let _ = Self::write_status_file(&status_path, &status_guard);
            status
        };
        slot.cancel.store(true, Ordering::Relaxed);
        let _ = self
            .app
            .emit(EVENT_JOB_STATUS, JobStatusEvent { job: status.clone() });
        log::info!("[jobs] {job_id} cancellation requested");
        Ok(status)
    }

    /// Terminates every running job. Called from the `RunEvent::Exit`
    /// handler in `main.rs`: Windows does NOT terminate child processes
    /// together with their parent, so without this the PowerShell jobs
    /// would linger as orphan processes after the GUI closes.
    ///
    /// The killed jobs are finalized by their waiter threads when the
    /// process lives long enough for one more 100 ms poll; otherwise their
    /// job_status files still say `Running` and are reconciled to `Canceled`
    /// by [`JobRegistry::new`] on the next application start.
    pub fn shutdown(&self) {
        let slots: Vec<(String, Arc<JobSlot>)> = {
            let map = self
                .running
                .lock()
                .expect("job registry map lock poisoned");
            map.iter().map(|(id, slot)| (id.clone(), Arc::clone(slot))).collect()
        };
        if slots.is_empty() {
            log::info!("[jobs] shutdown: no running jobs");
            return;
        }
        for (id, slot) in &slots {
            // Signal cancellation first so a surviving waiter thread reports
            // `Canceled` rather than `Failed` after the kill.
            slot.cancel.store(true, Ordering::Relaxed);
            if let Some(mut child) = slot
                .child
                .lock()
                .expect("job child lock poisoned")
                .take()
            {
                let _ = child.kill();
            }
            log::info!("[jobs] shutdown: terminated {id}");
        }
        log::warn!(
            "[jobs] shutdown: terminated {} running job(s)",
            slots.len()
        );
    }

    /// Returns the status of one job: running jobs from memory (freshest),
    /// everything else from the job_status file.
    pub fn get_job(&self, job_id: &str) -> McpGoResult<JobStatus> {
        if let Some(slot) = self
            .running
            .lock()
            .expect("job registry map lock poisoned")
            .get(job_id)
        {
            return Ok(slot.status.lock().expect("job status lock poisoned").clone());
        }
        self.read_status_file(job_id)
    }

    /// Lists known jobs, newest first: everything found on disk (current
    /// and previous sessions), with in-memory status preferred for running
    /// jobs. Capped at [`LIST_LIMIT`] entries.
    pub fn list_jobs(&self) -> Vec<JobStatus> {
        let mut jobs: Vec<JobStatus> = Vec::new();
        let entries = match fs::read_dir(&self.status_dir) {
            Ok(entries) => entries,
            Err(_) => return jobs,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else { continue };
            let Ok(status) = serde_json::from_str::<JobStatus>(&raw) else { continue };
            jobs.push(status);
        }
        // Overlay: in-memory status is fresher than the last file write.
        for (id, slot) in self
            .running
            .lock()
            .expect("job registry map lock poisoned")
            .iter()
        {
            let current = slot.status.lock().expect("job status lock poisoned").clone();
            match jobs.iter_mut().find(|status| &status.id == id) {
                Some(existing) => *existing = current,
                None => jobs.push(current),
            }
        }
        jobs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.started_at.cmp(&a.started_at))
                .then_with(|| b.id.cmp(&a.id))
        });
        jobs.truncate(LIST_LIMIT);
        jobs
    }

    /// Returns the parsed job_logs records of one job, oldest first.
    pub fn read_job_log(&self, job_id: &str) -> McpGoResult<JobLog> {
        // Validate the job exists (also guards against fabricated ids).
        self.get_job(job_id)?;
        let path = self.logs_dir.join(format!("{job_id}.log"));
        let raw = fs::read_to_string(&path).unwrap_or_default();
        let lines = raw.lines().map(parse_log_line).collect();
        Ok(JobLog { job_id: job_id.to_owned(), lines })
    }

    /// Reads one job_status file from disk.
    fn read_status_file(&self, job_id: &str) -> McpGoResult<JobStatus> {
        if !is_valid_job_id(job_id) {
            return Err(McpGoError::invalid_input("invalid job id"));
        }
        let path = self.status_dir.join(format!("{job_id}.json"));
        match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw)
                .map_err(|e| McpGoError::internal(format!("corrupt job_status file for {job_id}: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(McpGoError::job_not_found(job_id))
            }
            Err(e) => Err(McpGoError::io("read job_status file", &e)),
        }
    }

    /// Writes a job_status file atomically: serialize, write to a `.tmp`
    /// sibling, then rename over the target (POSIX-style replace).
    fn write_status_file(path: &Path, status: &JobStatus) -> McpGoResult<()> {
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(status)
            .map_err(|e| McpGoError::internal(format!("serialize job_status: {e}")))?;
        fs::write(&tmp, json).map_err(|e| McpGoError::io("write job_status file", &e))?;
        fs::rename(&tmp, path).map_err(|e| McpGoError::io("replace job_status file", &e))?;
        Ok(())
    }

    /// Cleanup for the (practically unreachable) case of a worker thread
    /// failing to spawn: kills the child, closes the log, unregisters the
    /// job and removes its files so no orphan artefacts remain.
    fn abort_job(
        &self,
        slot: &Arc<JobSlot>,
        id: &str,
        log_path: &Path,
        status_path: &Path,
        detail: &str,
    ) -> McpGoError {
        slot.cancel.store(true, Ordering::Relaxed);
        if let Some(mut child) = slot
            .child
            .lock()
            .expect("job child lock poisoned")
            .take()
        {
            let _ = child.kill();
        }
        // Closing the log handle lets remove_file succeed on Windows;
        // a detached reader may still write one last line into the
        // unlinked handle, which is harmless.
        slot.log.lock().expect("job log lock poisoned").take();
        self.running
            .lock()
            .expect("job registry map lock poisoned")
            .remove(id);
        let _ = fs::remove_file(log_path);
        let _ = fs::remove_file(status_path);
        log::error!("[jobs] {id} aborted: {detail}");
        McpGoError::internal(detail)
    }
}

// ----------------------------------------------------------------------------
// Worker threads
// ----------------------------------------------------------------------------

/// Drains one output pipe line-by-line: appends every line to the job_logs
/// file (best effort - a failing log write never stops the stream) and
/// emits a `job://output` invalidation event.
fn stream_output(
    app: AppHandle,
    slot: Arc<JobSlot>,
    job_id: String,
    reader: Box<dyn Read + Send>,
    stream: LogStream,
) {
    let level = match stream {
        LogStream::Out => "OUT",
        LogStream::Err => "ERR",
        LogStream::Sys => "SYS",
    };
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            // EOF: the pipe is closed (process exited or was killed).
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // PowerShell writes CRLF; normalise to a bare line.
        let mut line = String::from_utf8_lossy(&buffer).into_owned();
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        let timestamp = job_time::rfc3339_utc(job_time::now_epoch_millis());
        {
            let mut log_guard = slot.log.lock().expect("job log lock poisoned");
            if let Some(file) = log_guard.as_mut() {
                let _ = writeln!(file, "[{timestamp}] {level} {line}");
                let _ = file.flush();
            }
        }
        let _ = app.emit(EVENT_JOB_OUTPUT, JobOutputEvent { job_id: job_id.clone() });
    }
}

/// Waits for the process to exit (polling, cancellation-aware), joins the
/// reader threads, writes the footer record, finalizes the job_status
/// file, emits the final `job://status` event and unregisters the job.
fn wait_and_finalize(
    app: AppHandle,
    slot: Arc<JobSlot>,
    running: RunningMap,
    status_path: PathBuf,
    job_id: String,
    readers: Vec<JoinHandle<()>>,
) {
    // Phase 1: wait for the process, killing it once cancellation is
    // requested. A race window of one poll interval exists where a job
    // exits naturally just as the user cancels; such a job is reported as
    // Canceled, which is the honest view of the user's intent.
    let mut canceled = false;
    let mut exit_code: Option<i32> = None;
    loop {
        let mut finished = false;
        {
            let mut child_guard = slot.child.lock().expect("job child lock poisoned");
            if let Some(child) = child_guard.as_mut() {
                if slot.cancel.load(Ordering::Relaxed) && !canceled {
                    canceled = true;
                    let _ = child.kill();
                }
                match child.try_wait() {
                    Ok(Some(status)) => {
                        exit_code = status.code();
                        finished = true;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::error!("[jobs] {job_id} wait failed: {e}");
                        finished = true;
                    }
                }
            } else {
                // Defensive: child handle already taken.
                finished = true;
            }
        }
        if finished {
            break;
        }
        thread::sleep(WAIT_POLL_INTERVAL);
    }
    // The process is reaped; drop the handle.
    slot.child.lock().expect("job child lock poisoned").take();

    // Phase 2: let the readers drain the remaining pipe content so the
    // footer record is guaranteed to be the last line in the file.
    for handle in readers {
        let _ = handle.join();
    }

    // Phase 3: footer record + final status.
    let finished_at = job_time::now_epoch_millis();
    let (state, final_error) = if canceled {
        (JobState::Canceled, None)
    } else {
        match exit_code {
            Some(0) => (JobState::Completed, None),
            // Non-zero exit code: the exact number is recorded in the
            // status below and in the footer summary.
            Some(_) => (JobState::Failed, None),
            None => (
                JobState::Failed,
                Some("process exited without an exit code".into()),
            ),
        }
    };
    {
        let mut log_guard = slot.log.lock().expect("job log lock poisoned");
        if let Some(file) = log_guard.as_mut() {
            let summary = match (state, exit_code) {
                (JobState::Completed, _) => "job finished: exit code 0".to_string(),
                (JobState::Failed, Some(code)) => format!("job failed: exit code {code}"),
                (JobState::Failed, None) => "job failed: no exit code".to_string(),
                (JobState::Canceled, _) => "job canceled by user".to_string(),
                (JobState::Running, _) => "job still running".to_string(),
            };
            let _ = writeln!(file, "[{}] SYS {summary}", job_time::rfc3339_utc(finished_at));
            let _ = file.flush();
        }
        log_guard.take(); // close the file
    }

    {
        let mut status_guard = slot.status.lock().expect("job status lock poisoned");
        status_guard.state = state;
        status_guard.exit_code = exit_code;
        status_guard.finished_at = Some(finished_at);
        if let Some(error) = final_error {
            status_guard.error = Some(error);
        }
        let _ = JobRegistry::write_status_file(&status_path, &status_guard);
        let _ = app.emit(
            EVENT_JOB_STATUS,
            JobStatusEvent { job: status_guard.clone() },
        );
        log::info!(
            "[jobs] {job_id} finished: {state:?} (exit code: {})",
            exit_code.map(|code| code.to_string()).unwrap_or_else(|| "none".into())
        );
    }

    // Phase 4: the job is finished - remove it from the running map.
    running.lock().expect("job registry map lock poisoned").remove(&job_id);
}

// ----------------------------------------------------------------------------
// Parsing helpers
// ----------------------------------------------------------------------------

/// Parses one job_logs record. Canonical format:
/// `[RFC3339-UTC] OUT|ERR|SYS <text>`.
/// A raw output line that happens to look like a record but carries an
/// unknown level token falls back to `stream: "out"` with the raw text.
///
/// Shared with the services domain: the service worker
/// (`services/worker.rs`) writes the same canonical grammar into
/// `<data>\services\logs\<id>.log`, and `ServiceStore::read_log` parses
/// those files with this parser - one grammar, one parser.
pub(crate) fn parse_log_line(line: &str) -> JobLogLine {
    if let Some(rest) = line.strip_prefix('[') {
        if let Some((time, tail)) = rest.split_once("] ") {
            let (level, text) = match tail.split_once(' ') {
                Some((level, text)) => (level, text),
                None => (tail, ""),
            };
            let stream = match level {
                "OUT" => Some(LogStream::Out),
                "ERR" => Some(LogStream::Err),
                "SYS" => Some(LogStream::Sys),
                _ => None,
            };
            if let Some(stream) = stream {
                return JobLogLine {
                    time: Some(time.to_owned()),
                    stream,
                    text: text.to_owned(),
                };
            }
        }
    }
    JobLogLine {
        time: None,
        stream: LogStream::Out,
        text: line.to_owned(),
    }
}

/// Derives a short title from the command's first line.
fn truncate_title(command: &str) -> String {
    let first_line = command.lines().next().unwrap_or("");
    let mut title: String = first_line.chars().take(MAX_TITLE_CHARS).collect();
    if first_line.chars().count() > MAX_TITLE_CHARS {
        title.push_str("...");
    }
    title
}

/// Job ids reaching `get_job`/`read_job_log` come from the frontend, so
/// they are validated before being turned into file paths: only
/// alphanumerics and underscores, non-empty (generated ids always match).
fn is_valid_job_id(job_id: &str) -> bool {
    !job_id.is_empty() && job_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_records() {
        let line = parse_log_line("[2026-11-10T09:30:12.500Z] OUT Handles  NPM(K)");
        assert_eq!(line.stream, LogStream::Out);
        assert_eq!(line.time.as_deref(), Some("2026-11-10T09:30:12.500Z"));
        assert_eq!(line.text, "Handles  NPM(K)");

        let line = parse_log_line("[2026-11-10T09:30:13.100Z] ERR Get-Process: failed");
        assert_eq!(line.stream, LogStream::Err);
        assert_eq!(line.text, "Get-Process: failed");

        let line = parse_log_line("[2026-11-10T09:30:13.200Z] SYS job finished: exit code 0");
        assert_eq!(line.stream, LogStream::Sys);
    }

    #[test]
    fn unknown_levels_and_raw_lines_fall_back_to_out() {
        // A raw output line that coincidentally starts like a record but
        // carries an unknown level token.
        let line = parse_log_line("[today] VERBOSE something");
        assert_eq!(line.stream, LogStream::Out);
        assert_eq!(line.time, None);
        assert_eq!(line.text, "[today] VERBOSE something");

        // Completely plain line.
        let line = parse_log_line("just output");
        assert_eq!(line.stream, LogStream::Out);
        assert_eq!(line.text, "just output");

        // Level without text.
        let line = parse_log_line("[t] ERR");
        assert_eq!(line.stream, LogStream::Err);
        assert_eq!(line.text, "");
    }

    #[test]
    fn titles_are_truncated_at_60_chars() {
        let long = "x".repeat(80);
        let title = truncate_title(&long);
        assert_eq!(title.len(), 60 + 3);
        assert!(title.ends_with("..."));
    }

    #[test]
    fn job_ids_are_validated() {
        assert!(is_valid_job_id("job_1731234567890_3"));
        assert!(!is_valid_job_id(""));
        assert!(!is_valid_job_id("../secret"));
        assert!(!is_valid_job_id("job 1"));
    }
}
