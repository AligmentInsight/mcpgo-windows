//! Browser management - the port of the macOS Chrome/CDP domain.
//!
//! The macOS original controls Chrome through a Playwright daemon started
//! with the user's profile. This port implements the same capability with
//! the components already shipped by the previous phases:
//!
//! | macOS original                    | This port                                |
//! |-----------------------------------|------------------------------------------|
//! | Playwright daemon per browser     | one `chrome.exe` per session, launched   |
//! |                                   | by `chrome.rs`                           |
//! | daemon waits for the CDP endpoint | supervisor thread confirms the endpoint |
//! |                                   | (`cdp::fetch_version`)                   |
//! | daemon registry of live browsers  | [`BrowserRegistry`], persisted in memory |
//! |                                   | only - endpoints are re-discoverable    |
//! | known-debug-Chrome attach flow    | [`cdp::scan_debug_endpoints`]            |
//! | job_status / job_logs files       | `browser://status` events + the targets |
//! |                                   | panel (Chrome keeps no per-session files |
//! |                                   | McpGo could own)                         |
//!
//! ## Session lifecycle (event `browser://status`, payload [`BrowserStatusEvent`])
//!
//! ```text
//!                --(endpoint answers)------> Running --(exit code 0 / stop)--> Stopped
//! Starting --\
//!                --(no answer in 15 s)-----> Failed
//!                                          Running --(exit code != 0)------> Failed
//! Discovered (a live endpoint found at startup) --(endpoint gone)--> Stopped
//! ```
//!
//! ## Persistence decision (vs. the jobs domain)
//!
//! Sessions are NOT persisted to status files: a debug-enabled Chrome is
//! fully described by its loopback endpoint, so the application restart
//! story is a port scan ([`cdp::scan_debug_endpoints`]) instead of file
//! reconciliation. Launched sessions therefore die with the registry only in
//! the bookkeeping sense - the Chrome processes themselves follow the
//! rules of [`BrowserRegistry::shutdown`].
//!
//! ## Chrome 136 note
//!
//! Chrome ignores `--remote-debugging-port` together with the DEFAULT user
//! data directory. Sessions therefore always run with an explicit
//! `--user-data-dir` (a managed directory under `<data>\browser\profiles`
//! by default), and a launch request for the default directory is rejected
//! with an explanatory message. See `chrome.rs` for the full reasoning.

pub mod cdp;
pub mod chrome;

use std::collections::HashMap;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::error::{McpGoError, McpGoResult};
use crate::paths;
use crate::time as app_time;

pub use cdp::{BrowserTarget, DevToolsVersion, DiscoveredEndpoint};

/// Tauri event: a session changed its state. Payload: [`BrowserStatusEvent`].
pub const EVENT_BROWSER_STATUS: &str = "browser://status";

/// Maximum number of extra Chrome arguments per session.
const MAX_EXTRA_ARGS: usize = 16;
/// Maximum length of one extra Chrome argument (bytes).
const MAX_ARG_LEN: usize = 512;
/// Maximum length of the start URL (bytes).
const MAX_URL_LEN: usize = 2048;

/// Start-URL schemes a session may open. Anything else is rejected: the URL
/// is handed to chrome.exe verbatim, so exotic schemes (e.g. `file://` of a
/// directory) are better opened by the user's explicit command line.
const ALLOWED_URL_SCHEMES: [&str; 5] = ["http://", "https://", "file:///", "about:", "chrome://"];

/// How long a launch waits for the DevTools endpoint to answer before the
/// session is recorded as `Failed`.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval of the endpoint confirmation loop.
const CONFIRM_POLL: Duration = Duration::from_millis(250);
/// Poll interval of the launched-session exit watcher (also the latency of
/// a user-requested stop).
const EXIT_POLL: Duration = Duration::from_millis(500);
/// Poll interval of the discovered-session liveness probe.
const DISCOVERED_POLL: Duration = Duration::from_secs(3);
/// Consecutive failed probes before a discovered session is marked `Stopped`
/// (one hiccup must not flip a healthy endpoint).
const DISCOVERED_FAILURE_LIMIT: u32 = 2;

/// How many sessions `list_browser_sessions` returns (newest first).
const SESSION_LIST_LIMIT: usize = 100;

// ----------------------------------------------------------------------------
// Wire types (shared with the frontend; keep src/types/browser.ts in sync)
// ----------------------------------------------------------------------------

/// Lifecycle state of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserSessionState {
    /// Chrome was spawned; the DevTools endpoint has not answered yet.
    Starting,
    /// The endpoint answers; targets can be listed.
    Running,
    /// Terminal: exited cleanly or stopped from McpGo, or the endpoint of a
    /// discovered session vanished.
    Stopped,
    /// Terminal: the launch never came up, or Chrome exited non-zero.
    Failed,
}

impl std::fmt::Display for BrowserSessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            BrowserSessionState::Starting => "starting",
            BrowserSessionState::Running => "running",
            BrowserSessionState::Stopped => "stopped",
            BrowserSessionState::Failed => "failed",
        };
        f.write_str(name)
    }
}

/// How a session came into the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionOrigin {
    /// McpGo spawned the Chrome of this session and owns its process.
    Launched,
    /// A live DevTools endpoint was found by the startup scan; McpGo does
    /// NOT own the Chrome process behind it.
    Discovered,
}

/// One managed browser session, exchanged over IPC.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSession {
    /// Generated id, e.g. `br_1731234567890_3`.
    pub id: String,
    pub state: BrowserSessionState,
    pub origin: SessionOrigin,
    /// Loopback port of the DevTools endpoint.
    pub port: u16,
    /// HTTP base URL of the DevTools endpoint, e.g. `http://127.0.0.1:9222`.
    pub debug_http: String,
    /// Process id of the launched Chrome (`None` for discovered sessions).
    pub pid: Option<u32>,
    /// Product/version string reported by `/json/version` once confirmed.
    pub chrome_version: Option<String>,
    /// Whether the session runs headless (launched sessions only).
    pub headless: bool,
    /// URL the session opened at launch, when one was given.
    pub start_url: Option<String>,
    /// User data directory of the session (`None` for discovered sessions,
    /// whose Chrome was started outside McpGo).
    pub user_data_dir: Option<String>,
    /// Set as soon as a stop is requested (mirrors the job flag).
    pub stop_requested: bool,
    /// Session creation time, epoch milliseconds.
    pub created_at: u64,
    /// When the DevTools endpoint first answered, epoch milliseconds.
    pub confirmed_at: Option<u64>,
    /// Terminal transition time, epoch milliseconds.
    pub stopped_at: Option<u64>,
    /// Chrome exit code, once the process finished.
    pub exit_code: Option<i32>,
    /// Non-fatal infrastructure error recorded for the UI.
    pub error: Option<String>,
}

/// Launch request of the `launch_browser` command. All fields are optional;
/// `port: None` allocates the next free port of the McpGo range and
/// `user_data_dir: None` uses a managed profile directory.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOptions {
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub headless: bool,
    #[serde(default)]
    pub start_url: Option<String>,
    #[serde(default)]
    pub user_data_dir: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Static machine info rendered by the browser view header: where Chrome
/// lives, where managed profiles are stored and which port range McpGo
/// manages.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfile {
    /// Absolute path of the detected `chrome.exe`, when found.
    pub chrome_exe: Option<String>,
    /// Whether a Chrome installation was found.
    pub chrome_available: bool,
    /// The machine's default Chrome user data directory, when it exists
    /// (informational: launching debug sessions against it is rejected).
    pub default_user_data_dir: Option<String>,
    /// Directory of the managed session profiles
    /// (`<data>\browser\profiles`).
    pub managed_profiles_dir: String,
    /// First port McpGo allocates or scans.
    pub port_range_start: u16,
    /// Last port McpGo allocates or scans.
    pub port_range_end: u16,
}

/// Response of the `read_browser_targets` command: the live target list of
/// one running session joined with the endpoint's version string.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTargetList {
    pub session_id: String,
    /// Version string of the moment of the read (fresher than the cached
    /// session field if Chrome updated in place).
    pub chrome_version: String,
    /// Live targets, pages first.
    pub targets: Vec<BrowserTarget>,
}

/// Payload of the `browser://status` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatusEvent {
    pub session: BrowserSession,
}

// ----------------------------------------------------------------------------
// Registry
// ----------------------------------------------------------------------------

/// Shared map of all sessions (running and terminal). The registry owns one
/// clone, every supervisor thread owns another so it can finalize its
/// session after the map itself is unlocked.
type SessionMap = Arc<Mutex<HashMap<String, Arc<SessionSlot>>>>;

/// Per-session shared state.
struct SessionSlot {
    /// Current session snapshot; every mutation goes through
    /// [`apply_transition`] (terminal guard + event emission).
    status: Mutex<BrowserSession>,
    /// The launched Chrome; `None` for discovered sessions and after the
    /// final `take()` of a finished or stopped child.
    child: Mutex<Option<Child>>,
    /// Stop request flag, checked by the supervisor threads.
    stop_requested: AtomicBool,
}

/// Owns the session map and the managed profile directory. Managed as
/// Tauri state in `main.rs`.
pub struct BrowserRegistry {
    app: AppHandle,
    profiles_dir: PathBuf,
    sessions: SessionMap,
    counter: Arc<AtomicU64>,
}

impl BrowserRegistry {
    /// Creates the registry and starts the endpoint discovery scan in the
    /// background. Discovery runs asynchronously because probing 100 ports
    /// must not delay the application window; discovered endpoints appear
    /// through `browser://status` events as they register.
    pub fn new(app: AppHandle, profiles_dir: PathBuf) -> Self {
        // Idempotent; `main.rs::ensure_app_directories` created the parent
        // tree already, this also covers a manually cleaned directory.
        if let Err(e) = fs::create_dir_all(&profiles_dir) {
            log::warn!(
                "[browser] cannot create the managed profiles directory {}: {e}",
                profiles_dir.display()
            );
        }

        let registry = Self {
            app,
            profiles_dir,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            counter: Arc::new(AtomicU64::new(0)),
        };

        let discovery = thread::Builder::new()
            .name("browser-discovery".into())
            .spawn({
                let app = registry.app.clone();
                let sessions = Arc::clone(&registry.sessions);
                let counter = Arc::clone(&registry.counter);
                move || discover_and_register(app, sessions, counter)
            });
        if let Err(e) = discovery {
            log::warn!("[browser] discovery thread failed to start: {e}");
        }
        registry
    }

    /// Returns the machine's browser profile information for the view
    /// header. Cheap synchronous path probes, recomputed on every call so
    /// a Chrome installed while the app runs is picked up.
    pub fn profile(&self) -> BrowserProfile {
        let exe = paths::chrome_executable_path();
        let default_dir = paths::chrome_user_data_path();
        BrowserProfile {
            chrome_exe: exe.as_deref().and_then(Path::to_str).map(str::to_owned),
            chrome_available: exe.is_some(),
            default_user_data_dir: default_dir
                .as_deref()
                .and_then(Path::to_str)
                .map(str::to_owned),
            managed_profiles_dir: self.profiles_dir.to_string_lossy().into_owned(),
            port_range_start: cdp::PORT_RANGE_START,
            port_range_end: cdp::PORT_RANGE_END,
        }
    }

    /// Resolves one session slot by id.
    fn slot(&self, id: &str) -> McpGoResult<Arc<SessionSlot>> {
        self.sessions
            .lock()
            .expect("browser registry lock poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| McpGoError::item_not_found("browser session", id))
    }

    /// Returns the current snapshot of one session.
    pub fn get_session(&self, id: &str) -> McpGoResult<BrowserSession> {
        Ok(self
            .slot(id)?
            .status
            .lock()
            .expect("session status lock poisoned")
            .clone())
    }

    /// Lists all sessions of this application run, newest first.
    pub fn list_sessions(&self) -> Vec<BrowserSession> {
        let mut sessions: Vec<BrowserSession> = self
            .sessions
            .lock()
            .expect("browser registry lock poisoned")
            .values()
            .map(|slot| slot.status.lock().expect("session status lock poisoned").clone())
            .collect();
        sessions.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        sessions.truncate(SESSION_LIST_LIMIT);
        sessions
    }

    /// Launches one debug-enabled Chrome session. Validation, spawning,
    /// registration and supervision are sequenced so every failure path
    /// leaves no partial registration behind.
    pub fn launch(&self, options: LaunchOptions) -> McpGoResult<BrowserSession> {
        // --- 1. Validate the user input --------------------------------------
        let extra_args = chrome::sanitize_extra_args(&options.extra_args);
        if extra_args.len() > MAX_EXTRA_ARGS {
            return Err(McpGoError::invalid_input(format!(
                "too many extra arguments ({} > {MAX_EXTRA_ARGS})",
                extra_args.len()
            )));
        }
        if let Some(arg) = extra_args.iter().find(|arg| arg.len() > MAX_ARG_LEN) {
            return Err(McpGoError::invalid_input(format!(
                "extra argument is too long ({} bytes, limit {MAX_ARG_LEN} bytes)",
                arg.len()
            )));
        }

        let start_url = options
            .start_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty());
        if let Some(url) = start_url {
            if url.len() > MAX_URL_LEN {
                return Err(McpGoError::invalid_input(format!(
                    "start URL is too long ({} bytes, limit {MAX_URL_LEN} bytes)",
                    url.len()
                )));
            }
            let lowered = url.to_lowercase();
            if !ALLOWED_URL_SCHEMES.iter().any(|scheme| lowered.starts_with(scheme)) {
                return Err(McpGoError::invalid_input(format!(
                    "start URL scheme is not allowed (supported: {})",
                    ALLOWED_URL_SCHEMES.join(", ")
                )));
            }
        }

        let user_data_dir: Option<PathBuf> = match options
            .user_data_dir
            .as_deref()
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
        {
            Some(dir) => {
                let path = PathBuf::from(dir);
                if !path.is_absolute() {
                    return Err(McpGoError::invalid_input(
                        "user data directory must be an absolute path",
                    ));
                }
                if chrome::is_default_chrome_profile(&path) {
                    return Err(McpGoError::invalid_input(
                        "the default Chrome user data directory cannot host a debug session: \
                         since Chrome 136 the --remote-debugging-port switch is ignored \
                         with the default profile. Leave the field empty to use a managed \
                         profile, or pick a non-default directory.",
                    ));
                }
                Some(path)
            }
            None => None,
        };

        if let Some(port) = options.port {
            if port < 1024 {
                return Err(McpGoError::invalid_input(
                    "port must be between 1024 and 65535",
                ));
            }
            if cdp::endpoint_alive(port) {
                return Err(McpGoError::invalid_input(format!(
                    "port {port} already serves a DevTools endpoint"
                )));
            }
            if TcpListener::bind((cdp::HOST, port)).is_err() {
                return Err(McpGoError::invalid_input(format!(
                    "port {port} is already used by another process"
                )));
            }
        }

        // --- 2. Resolve the executable ---------------------------------------
        let exe = paths::chrome_executable_path().ok_or_else(|| McpGoError::ComponentMissing {
            component: "chrome".into(),
            detail: "no Google Chrome installation was found in the standard locations".into(),
        })?;

        // --- 3. Identify the session (the id names the managed profile) ------
        let now = app_time::now_epoch_millis();
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        let id = format!("br_{now}_{seq}");

        let data_dir = match &user_data_dir {
            Some(explicit) => explicit.clone(),
            None => {
                let managed = chrome::managed_profile_dir(&self.profiles_dir, &id);
                fs::create_dir_all(&managed)
                    .map_err(|e| McpGoError::io("create managed profile directory", &e))?;
                managed
            }
        };

        let port = match options.port {
            Some(port) => port,
            None => chrome::allocate_debug_port().ok_or_else(|| {
                McpGoError::operation(
                    "allocating a debugging port",
                    &format!(
                        "no free port in the range {}..={}",
                        cdp::PORT_RANGE_START,
                        cdp::PORT_RANGE_END
                    ),
                )
            })?,
        };

        // --- 4. Spawn Chrome --------------------------------------------------
        let args = chrome::build_launch_args(
            port,
            &data_dir,
            options.headless,
            start_url,
            &extra_args,
        );
        let child = chrome::launch(&exe, &args)
            .map_err(|e| McpGoError::io("launch chrome", &e))?;
        let pid = child.id();

        // --- 5. Register and supervise ---------------------------------------
        let status = BrowserSession {
            id: id.clone(),
            state: BrowserSessionState::Starting,
            origin: SessionOrigin::Launched,
            port,
            debug_http: format!("http://{}:{port}", cdp::HOST),
            pid: Some(pid),
            chrome_version: None,
            headless: options.headless,
            start_url: start_url.map(str::to_owned),
            user_data_dir: Some(data_dir.to_string_lossy().into_owned()),
            stop_requested: false,
            created_at: now,
            confirmed_at: None,
            stopped_at: None,
            exit_code: None,
            error: None,
        };
        let slot = Arc::new(SessionSlot {
            status: Mutex::new(status.clone()),
            child: Mutex::new(Some(child)),
            stop_requested: AtomicBool::new(false),
        });
        self.sessions
            .lock()
            .expect("browser registry lock poisoned")
            .insert(id.clone(), Arc::clone(&slot));

        let supervisor = thread::Builder::new()
            .name(format!("{id}-supervisor"))
            .spawn({
                let app = self.app.clone();
                let slot = Arc::clone(&slot);
                let id = id.clone();
                move || supervise_launched(app, slot, id)
            });
        if supervisor.is_err() {
            // Undo the registration and terminate the freshly spawned
            // Chrome: a registry entry without a supervisor would never
            // observe the process exit and keep state "starting" forever.
            self.sessions
                .lock()
                .expect("browser registry lock poisoned")
                .remove(&id);
            if let Some(mut taken) = slot
                .child
                .lock()
                .expect("session child lock poisoned")
                .take()
            {
                let _ = taken.kill();
            }
            return Err(McpGoError::internal(
                "failed to spawn the session supervisor thread",
            ));
        }

        // --- 6. Announce and return ------------------------------------------
        let _ = self.app.emit(
            EVENT_BROWSER_STATUS,
            BrowserStatusEvent {
                session: status.clone(),
            },
        );
        log::info!(
            "[browser] {id} launching chrome on port {port} (pid {pid}, {})",
            exe.display()
        );
        Ok(status)
    }

    /// Requests the stop of one launched session: the Chrome process is
    /// terminated and the supervisor finalizes the session as `Stopped`
    /// within one poll interval. Returns the current snapshot (still
    /// `Running`, with `stopRequested: true`) - the final status arrives
    /// through `browser://status`, exactly like the job cancellation flow.
    pub fn stop_session(&self, id: &str) -> McpGoResult<BrowserSession> {
        let slot = self.slot(id)?;
        let snapshot = {
            let mut status = slot.status.lock().expect("session status lock poisoned");
            if status.origin == SessionOrigin::Discovered {
                return Err(McpGoError::invalid_input(
                    "discovered sessions are not owned by McpGo; close the Chrome \
                     that exposes the endpoint instead",
                ));
            }
            if is_terminal(status.state) {
                return Ok(status.clone()); // idempotent stop
            }
            status.stop_requested = true;
            status.clone()
        };

        slot.stop_requested.store(true, Ordering::Relaxed);
        let _ = self.app.emit(
            EVENT_BROWSER_STATUS,
            BrowserStatusEvent {
                session: snapshot.clone(),
            },
        );
        if let Some(mut child) = slot
            .child
            .lock()
            .expect("session child lock poisoned")
            .take()
        {
            let _ = child.kill();
            // Reap immediately: the supervisor's exit watcher sees the
            // taken child plus the stop flag and finalizes the session.
            let _ = child.wait();
        }
        log::info!("[browser] {id} stop requested");
        Ok(snapshot)
    }

    /// Reads the live targets of one RUNNING session (two endpoint probes:
    /// `/json/version` + `/json/list`). The session's cached version field
    /// is refreshed opportunistically.
    pub fn read_targets(&self, id: &str) -> McpGoResult<BrowserTargetList> {
        let slot = self.slot(id)?;
        let (state, port) = {
            let status = slot.status.lock().expect("session status lock poisoned");
            (status.state, status.port)
        };
        if state != BrowserSessionState::Running {
            return Err(McpGoError::invalid_input(format!(
                "browser session {id} is not running (state: {state})"
            )));
        }

        let version = cdp::try_fetch_version(port)?;
        let targets = cdp::fetch_targets(port)
            .map_err(|detail| McpGoError::operation("querying the DevTools endpoint", &detail))?;

        // Refresh the cached version without emitting an event: this read
        // runs every few seconds while the targets panel is open.
        {
            let mut status = slot.status.lock().expect("session status lock poisoned");
            if !is_terminal(status.state) {
                status.chrome_version = Some(version.browser.clone());
            }
        }

        Ok(BrowserTargetList {
            session_id: id.to_owned(),
            chrome_version: version.browser,
            targets,
        })
    }

    /// Exit cleanup, called from the `RunEvent::Exit` handler in `main.rs`.
    ///
    /// * Headless launched sessions are terminated: they are invisible, so
    ///   letting them run would leak one orphan Chrome per session.
    /// * Headful (windowed) launched sessions keep running: their windows
    ///   are user-visible, and the next application start re-attaches to
    ///   them through the endpoint scan (the same "services outlive the
    ///   GUI" reasoning as the services domain).
    /// * Discovered sessions are never owned by McpGo; their endpoint
    ///   supervisor simply dies with the process.
    pub fn shutdown(&self) {
        let slots: Vec<(String, Arc<SessionSlot>)> = self
            .sessions
            .lock()
            .expect("browser registry lock poisoned")
            .iter()
            .map(|(id, slot)| (id.clone(), Arc::clone(slot)))
            .collect();

        let mut terminated = 0usize;
        let mut left_running = 0usize;
        for (id, slot) in &slots {
            let (origin, headless, state, stop_requested) = {
                let status = slot.status.lock().expect("session status lock poisoned");
                (
                    status.origin,
                    status.headless,
                    status.state,
                    status.stop_requested,
                )
            };
            if origin != SessionOrigin::Launched
                || stop_requested
                || is_terminal(state)
            {
                continue; // not ours to kill, or already dying/finished
            }
            if headless {
                slot.stop_requested.store(true, Ordering::Relaxed);
                if let Some(mut child) = slot
                    .child
                    .lock()
                    .expect("session child lock poisoned")
                    .take()
                {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                log::info!("[browser] shutdown: terminated headless {id}");
                terminated += 1;
            } else {
                log::info!("[browser] shutdown: leaving windowed {id} running");
                left_running += 1;
            }
        }
        if terminated > 0 || left_running > 0 {
            log::info!(
                "[browser] shutdown: terminated {terminated} headless session(s), \
                 left {left_running} windowed session(s) running"
            );
        }
    }
}

// ----------------------------------------------------------------------------
// Supervisors (free functions so the registry stays lock-agnostic)
// ----------------------------------------------------------------------------

/// `true` for terminal states - transitions into them are final.
fn is_terminal(state: BrowserSessionState) -> bool {
    matches!(
        state,
        BrowserSessionState::Stopped | BrowserSessionState::Failed
    )
}

/// Applies one state mutation under the session lock, emits the
/// `browser://status` event and returns the new snapshot. Terminal states
/// are immutable: a late supervisor poll can never resurrect a session.
fn apply_transition(
    app: &AppHandle,
    slot: &SessionSlot,
    mutate: impl FnOnce(&mut BrowserSession),
) -> BrowserSession {
    let mut status = slot.status.lock().expect("session status lock poisoned");
    if is_terminal(status.state) {
        return status.clone();
    }
    mutate(&mut status);
    let snapshot = status.clone();
    drop(status);
    let _ = app.emit(
        EVENT_BROWSER_STATUS,
        BrowserStatusEvent {
            session: snapshot.clone(),
        },
    );
    snapshot
}

/// Finalizes a session as `Stopped` (clean exit / user stop / vanished
/// endpoint). The reason text goes to the log only - the state itself says
/// everything the UI needs.
fn finalize_stopped(app: &AppHandle, slot: &SessionSlot, id: &str, exit_code: Option<i32>) {
    apply_transition(app, slot, |status| {
        status.state = BrowserSessionState::Stopped;
        status.exit_code = exit_code;
        status.stopped_at = Some(app_time::now_epoch_millis());
        status.error = None;
    });
    log::info!("[browser] {id} stopped");
}

/// Supervises one launched session:
///
/// * Phase A - confirm the DevTools endpoint answers within
///   [`CONFIRM_TIMEOUT`] (polls every [`CONFIRM_POLL`]); a Chrome that dies
///   early or never opens the endpoint records a `Failed` session.
/// * Phase B - watch the Chrome process (polls every [`EXIT_POLL`]); clean
///   exits become `Stopped`, non-zero exits `Failed`, and the stop flag of
///   [`BrowserRegistry::stop_session`] finalizes as `Stopped` with the
///   recorded exit code.
fn supervise_launched(app: AppHandle, slot: Arc<SessionSlot>, id: String) {
    let port = slot.status.lock().expect("session status lock poisoned").port;

    // --- Phase A: wait for the endpoint -----------------------------------
    let deadline = Instant::now() + CONFIRM_TIMEOUT;
    let mut version: Option<String> = None;
    while Instant::now() < deadline {
        if slot.stop_requested.load(Ordering::Relaxed) {
            // stop_session already took and killed the child.
            finalize_stopped(&app, &slot, &id, None);
            return;
        }
        {
            let mut child = slot.child.lock().expect("session child lock poisoned");
            match child.as_mut().map(Child::try_wait) {
                Some(Ok(Some(exit))) => {
                    let code = exit.code();
                    child.take();
                    drop(child);
                    apply_transition(&app, &slot, |status| {
                        status.state = BrowserSessionState::Failed;
                        status.exit_code = code;
                        status.stopped_at = Some(app_time::now_epoch_millis());
                        status.error = Some(
                            "chrome exited before the DevTools endpoint answered".into(),
                        );
                    });
                    log::warn!("[browser] {id} exited before the endpoint answered");
                    return;
                }
                Some(Err(e)) => {
                    log::warn!("[browser] {id}: waiting for the child failed: {e}");
                }
                Some(Ok(None)) | None => {}
            }
        }
        if let Ok(answer) = cdp::fetch_version(port) {
            version = Some(answer.browser);
            break;
        }
        thread::sleep(CONFIRM_POLL);
    }

    let Some(version) = version else {
        // The endpoint never came up: kill the leftover process and record
        // the failure (a port stolen between allocation and Chrome's bind
        // lands here as well).
        if let Some(mut child) = slot
            .child
            .lock()
            .expect("session child lock poisoned")
            .take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        apply_transition(&app, &slot, |status| {
            status.state = BrowserSessionState::Failed;
            status.stopped_at = Some(app_time::now_epoch_millis());
            status.error = Some(format!(
                "the DevTools endpoint did not answer within {} seconds",
                CONFIRM_TIMEOUT.as_secs()
            ));
        });
        log::warn!("[browser] {id} never answered on port {port}");
        return;
    };

    apply_transition(&app, &slot, |status| {
        status.state = BrowserSessionState::Running;
        status.confirmed_at = Some(app_time::now_epoch_millis());
        status.chrome_version = Some(version.clone());
    });
    log::info!("[browser] {id} running on port {port} ({version})");

    // --- Phase B: watch the process ----------------------------------------
    loop {
        if slot.stop_requested.load(Ordering::Relaxed) {
            finalize_stopped(&app, &slot, &id, None);
            return;
        }
        let mut child = slot.child.lock().expect("session child lock poisoned");
        match child.as_mut().map(Child::try_wait) {
            Some(Ok(Some(exit))) => {
                let code = exit.code();
                child.take();
                drop(child);
                if slot.stop_requested.load(Ordering::Relaxed) {
                    finalize_stopped(&app, &slot, &id, code);
                } else if code.map(|c| c != 0).unwrap_or(false) {
                    apply_transition(&app, &slot, |status| {
                        status.state = BrowserSessionState::Failed;
                        status.exit_code = code;
                        status.stopped_at = Some(app_time::now_epoch_millis());
                        status.error = Some(format!(
                            "chrome exited with code {}",
                            code.unwrap_or_default()
                        ));
                    });
                    log::warn!("[browser] {id} crashed with exit code {code:?}");
                } else {
                    finalize_stopped(&app, &slot, &id, code);
                }
                return;
            }
            Some(Err(e)) => {
                log::warn!("[browser] {id}: waiting for the child failed: {e}");
            }
            Some(Ok(None)) | None => {}
        }
        drop(child);
        thread::sleep(EXIT_POLL);
    }
}

/// Supervises one discovered session: probes the endpoint until it stops
/// answering ([`DISCOVERED_FAILURE_LIMIT`] consecutive misses), then
/// records the session as `Stopped` (a closed Chrome is a fact, not an
/// error).
fn supervise_discovered(app: AppHandle, slot: Arc<SessionSlot>, id: String) {
    let port = slot.status.lock().expect("session status lock poisoned").port;
    let mut misses: u32 = 0;
    loop {
        thread::sleep(DISCOVERED_POLL);
        if is_terminal(
            slot.status
                .lock()
                .expect("session status lock poisoned")
                .state,
        ) {
            return; // already finalized by another path
        }
        if cdp::endpoint_alive(port) {
            misses = 0;
        } else {
            misses += 1;
            if misses >= DISCOVERED_FAILURE_LIMIT {
                finalize_stopped(&app, &slot, &id, None);
                log::info!("[browser] {id} endpoint on port {port} is no longer reachable");
                return;
            }
        }
    }
}

/// Startup discovery: scans the McpGo port range and registers every live
/// DevTools endpoint as a `Discovered` session. Runs on its own thread so
/// application startup is never blocked by the probes.
fn discover_and_register(app: AppHandle, sessions: SessionMap, counter: Arc<AtomicU64>) {
    let endpoints = cdp::scan_debug_endpoints();
    for endpoint in endpoints {
        let now = app_time::now_epoch_millis();
        let id = format!(
            "br_{now}_{}",
            counter.fetch_add(1, Ordering::Relaxed)
        );
        let status = BrowserSession {
            id: id.clone(),
            state: BrowserSessionState::Running,
            origin: SessionOrigin::Discovered,
            port: endpoint.port,
            debug_http: format!("http://{}:{}", cdp::HOST, endpoint.port),
            pid: None,
            chrome_version: Some(endpoint.browser),
            headless: false,
            start_url: None,
            user_data_dir: None,
            stop_requested: false,
            created_at: now,
            confirmed_at: Some(now),
            stopped_at: None,
            exit_code: None,
            error: None,
        };
        let slot = Arc::new(SessionSlot {
            status: Mutex::new(status.clone()),
            child: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
        });
        sessions
            .lock()
            .expect("browser registry lock poisoned")
            .insert(id.clone(), Arc::clone(&slot));
        let _ = app.emit(
            EVENT_BROWSER_STATUS,
            BrowserStatusEvent {
                session: status,
            },
        );

        let supervisor = thread::Builder::new()
            .name(format!("{id}-supervisor"))
            .spawn({
                let app = app.clone();
                let slot = Arc::clone(&slot);
                let id = id.clone();
                move || supervise_discovered(app, slot, id)
            });
        if let Err(e) = supervisor {
            log::warn!("[browser] {id} supervisor thread failed to start: {e}");
        }
    }
    log::info!("[browser] discovery finished");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states_are_stopped_and_failed() {
        assert!(!is_terminal(BrowserSessionState::Starting));
        assert!(!is_terminal(BrowserSessionState::Running));
        assert!(is_terminal(BrowserSessionState::Stopped));
        assert!(is_terminal(BrowserSessionState::Failed));
    }

    #[test]
    fn states_serialize_lowercase_camel_case() {
        assert_eq!(
            serde_json::to_string(&BrowserSessionState::Starting).expect("serialize"),
            "\"starting\""
        );
        assert_eq!(
            serde_json::to_string(&SessionOrigin::Discovered).expect("serialize"),
            "\"discovered\""
        );
    }

    #[test]
    fn display_matches_the_serialized_names() {
        assert_eq!(BrowserSessionState::Running.to_string(), "running");
        assert_eq!(BrowserSessionState::Failed.to_string(), "failed");
    }

    #[test]
    fn launch_options_default_to_a_managed_headful_session() {
        // The frontend always sends the full object, but a missing key must
        // never break the deserialization (serde defaults).
        let options: LaunchOptions = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(options.port, None);
        assert!(!options.headless);
        assert_eq!(options.start_url, None);
        assert_eq!(options.user_data_dir, None);
        assert!(options.extra_args.is_empty());
    }

    #[test]
    fn launch_options_read_camel_case_keys() {
        let options: LaunchOptions = serde_json::from_str(
            r#"{
                "port": 9333,
                "headless": true,
                "startUrl": "https://example.com/",
                "userDataDir": "C:\\Data\\SomeProfile",
                "extraArgs": ["--incognito"]
            }"#,
        )
        .expect("deserialize");
        assert_eq!(options.port, Some(9333));
        assert!(options.headless);
        assert_eq!(options.start_url.as_deref(), Some("https://example.com/"));
        assert_eq!(options.user_data_dir.as_deref(), Some(r"C:\Data\SomeProfile"));
        assert_eq!(options.extra_args, vec!["--incognito"]);
    }
}
