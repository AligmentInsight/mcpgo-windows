//! OpenAI Secure MCP Tunnels - the port of the macOS roadmap's tunnel
//! domain (stage 5), implemented as the supervision domain of the official
//! `tunnel-client` runtime.
//!
//! Secure MCP Tunnel connects a private (localhost or LAN) MCP server to
//! ChatGPT, Codex and the Responses API without exposing it to the public
//! internet: the runtime polls the OpenAI-hosted control plane
//! (`https://api.openai.com/v1/tunnels/...`), forwards requests to the
//! local MCP server (stdio or streamable HTTP) and delivers the responses
//! back. Applications reach the server through the OpenAI-hosted
//! `/v1/mcp/{tunnel_id}` endpoint.
//!
//! | Piece                             | Lives in                          |
//! |-----------------------------------|-----------------------------------|
//! | persisted definitions             | [`definition`] (`<tunnels>\*.json`)|
//! | generated `tunnel-client` profile | [`profile`] (`profiles\<id>.yaml`) |
//! | executable resolution + spawn     | [`client`]                        |
//! | loopback `/healthz` probing       | [`health`] (ports 18080..=18179)   |
//! | lifecycle + events                | this module ([`TunnelRegistry`])  |
//!
//! ## Runtime lifecycle (event `tunnel://status`, payload
//! ## [`TunnelStatusEvent`])
//!
//! ```text
//!             --(health answers)------> Running --(stop / clean exit 1)--> Stopped
//! Stopped --\                             |
//!   ^        --(no answer in 20 s)--> Failed(exit / timeout)
//!   |                                          |
//!   +----(keep_alive: automatic restart)-------+
//! ```
//!
//! * `Running` additionally requires the health endpoint to KEEP answering:
//!   three consecutive misses (15 s) mark the tunnel `Failed` and terminate
//!   the process - a wedged runtime must not linger invisibly.
//! * `keep_alive` restarts the runtime after unexpected exits (crash or
//!   timeout). Consecutive start failures are capped at
//!   [`START_FAILURE_LIMIT`] so a permanently broken configuration (a bad
//!   key, a dead endpoint) fails visibly instead of spinning; the counter
//!   resets once a runtime confirmed health.
//! * One runtime per tunnel is a protocol-level requirement (the stdio
//!   deployment limit of the client): `start_tunnel` refuses while a
//!   runtime is active, and definition edits/deletes are blocked until it
//!   is stopped.
//!
//! ## Persistence decision
//!
//! Runtime state lives in memory only: after an application restart every
//! tunnel reads as `Stopped` (a McpGo crash can orphan a runtime - its
//! pinned health port then answers, the next start attempt fails with a
//! precise "port already serves a health listener" message, and the user
//! resolves it by stopping the orphan or rebooting). Definitions, profiles,
//! secrets and logs are files, created by [`definition`] and [`profile`].

pub mod client;
pub mod definition;
pub mod health;
pub mod profile;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::{McpGoError, McpGoResult};
use crate::time as app_time;

pub use definition::{
    ApiKeySource, ApiKeySourceKind, Residency, TunnelDefinition, TunnelDefinitionInput,
    TunnelStore, TunnelTargetKind,
};

/// Tauri event: a tunnel changed its state. Payload: [`TunnelStatusEvent`].
pub const EVENT_TUNNEL_STATUS: &str = "tunnel://status";

/// How long a start waits for the health endpoint to answer before the
/// tunnel records `Failed` (the client binds its listener before the first
/// control-plane poll, so this covers slow machines, not slow networks).
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(20);
/// Poll interval of the health confirmation loop (Phase A).
const CONFIRM_POLL: Duration = Duration::from_millis(250);
/// Poll interval of the process watcher (also the latency of a requested
/// stop, Phase B).
const EXIT_POLL: Duration = Duration::from_millis(500);
/// Interval between two health probes while running (Phase B).
const HEALTH_POLL: Duration = Duration::from_secs(5);
/// Consecutive failed health probes before a running tunnel is marked
/// `Failed` (one hiccup must not flip a healthy runtime).
const HEALTH_MISS_LIMIT: u32 = 3;
/// Pause before an automatic (keep_alive) restart.
const RESTART_DELAY: Duration = Duration::from_secs(3);
/// Consecutive start failures (runtime exits or never confirms health)
/// before keep_alive gives up and leaves the tunnel `Failed`.
const START_FAILURE_LIMIT: u32 = 5;
/// Upper bound of log lines returned by `read_tunnel_log` (newest kept).
const LOG_LINES_LIMIT: usize = 2000;

/// OpenAI Platform page that lists and creates tunnels (the management
/// surface McpGo deliberately does not duplicate).
pub const PLATFORM_TUNNELS_URL: &str =
    "https://platform.openai.com/settings/organization/tunnels";
/// Public Secure MCP Tunnel guide.
pub const TUNNEL_DOCS_URL: &str =
    "https://developers.openai.com/api/docs/guides/secure-mcp-tunnels";
/// Repository of the supervised runtime (release downloads).
pub const TUNNEL_CLIENT_REPO_URL: &str = "https://github.com/openai/tunnel-client";

// ----------------------------------------------------------------------------
// Wire types (shared with the frontend; keep src/types/tunnel.ts in sync)
// ----------------------------------------------------------------------------

/// Lifecycle state of one tunnel's runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TunnelState {
    /// No runtime of this application run (initial state).
    Stopped,
    /// The runtime was spawned; its health endpoint has not answered yet.
    Starting,
    /// The health endpoint answers; the tunnel is live.
    Running,
    /// Terminal: the runtime exited unexpectedly, never came up, or its
    /// health endpoint stopped answering.
    Failed,
}

impl std::fmt::Display for TunnelState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            TunnelState::Stopped => "stopped",
            TunnelState::Starting => "starting",
            TunnelState::Running => "running",
            TunnelState::Failed => "failed",
        };
        f.write_str(name)
    }
}

/// Live runtime snapshot of one tunnel, exchanged over IPC and through the
/// `tunnel://status` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    /// Definition id this runtime belongs to.
    pub definition_id: String,
    pub state: TunnelState,
    /// Process id of the current runtime process.
    pub pid: Option<u32>,
    /// Health-listener port of this run.
    pub health_port: Option<u16>,
    /// `http://127.0.0.1:<port>/healthz` (the liveness endpoint).
    pub health_url: Option<String>,
    /// `http://127.0.0.1:<port>/ui` (the client's operator UI).
    pub ui_url: Option<String>,
    /// When the current runtime was spawned, epoch milliseconds.
    pub started_at: Option<u64>,
    /// When the health endpoint first answered, epoch milliseconds.
    pub confirmed_at: Option<u64>,
    /// Terminal transition time, epoch milliseconds.
    pub stopped_at: Option<u64>,
    /// Exit code of the last runtime process, once it finished.
    pub exit_code: Option<i32>,
    /// Automatic restarts performed by keep_alive during this run.
    pub restarts: u32,
    /// Set as soon as a stop is requested (mirrors the job flag).
    pub stop_requested: bool,
    /// Non-fatal infrastructure error recorded for the UI.
    pub error: Option<String>,
}

impl TunnelStatus {
    /// The `Stopped` snapshot of a tunnel that never ran in this
    /// application run.
    fn dormant(definition_id: &str) -> Self {
        TunnelStatus {
            definition_id: definition_id.to_owned(),
            state: TunnelState::Stopped,
            pid: None,
            health_port: None,
            health_url: None,
            ui_url: None,
            started_at: None,
            confirmed_at: None,
            stopped_at: None,
            exit_code: None,
            restarts: 0,
            stop_requested: false,
            error: None,
        }
    }
}

/// Payload of the `tunnel://status` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatusEvent {
    pub tunnel: TunnelStatus,
}

/// A tunnel definition joined with its live runtime state and the machine
/// facts the card renders - one row of the tunnels view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelSummary {
    /// The persisted definition (never contains key material).
    pub definition: TunnelDefinition,
    /// Live runtime state of this application run.
    pub status: TunnelStatus,
    /// Whether the tunnel-client executable resolves for this definition.
    pub client_available: bool,
    /// The executable a start would spawn (override, PATH or managed bin).
    pub client_path_used: Option<String>,
    /// Whether the configured API key source is ready (secret file exists /
    /// variable set / key file exists).
    pub api_key_ready: bool,
    /// Path of the managed secret file (Managed sources only).
    pub secret_path: Option<String>,
    /// Absolute path of the generated profile.
    pub profile_path: String,
    /// Absolute path of the runtime log file (ndjson).
    pub log_path: String,
    /// Absolute path of the definition file.
    pub definition_path: String,
}

/// Static machine info rendered by the tunnels view header: where the
/// client is expected, where McpGo keeps its files and which health port
/// range it manages.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelsProfile {
    /// Resolved `tunnel-client` executable (default resolution), if any.
    pub client_path: Option<String>,
    /// Whether a tunnel-client executable was found.
    pub client_available: bool,
    /// Drop-in directory for `tunnel-client.exe`
    /// (`<data>\tunnels\bin`).
    pub managed_bin_dir: String,
    /// Directory of the generated profiles.
    pub profiles_dir: String,
    /// Directory of the managed key files.
    pub secrets_dir: String,
    /// Directory of the runtime logs.
    pub logs_dir: String,
    /// First health port McpGo allocates.
    pub health_port_start: u16,
    /// Last health port McpGo allocates.
    pub health_port_end: u16,
    /// Platform page that manages tunnels (create/list).
    pub platform_tunnels_url: String,
    /// Public Secure MCP Tunnel guide.
    pub tunnel_docs_url: String,
    /// Repository of the supervised runtime.
    pub tunnel_client_repo_url: String,
}

/// Response of the `read_tunnel_log` command: the newest ndjson lines of
/// the runtime log, oldest first. The lines stay unparsed - the client owns
/// their schema; McpGo only renders them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelLog {
    /// Definition id whose log was read.
    pub id: String,
    /// Raw log lines, oldest first, capped at [`LOG_LINES_LIMIT`].
    pub lines: Vec<String>,
}

// ----------------------------------------------------------------------------
// Registry
// ----------------------------------------------------------------------------

/// Shared map of the live slots, keyed by definition id.
type SlotMap = Arc<Mutex<HashMap<String, Arc<TunnelSlot>>>>;

/// Per-tunnel shared state.
struct TunnelSlot {
    /// Current runtime snapshot; every mutation goes through
    /// [`apply_transition`] (terminal guard + event emission).
    status: Mutex<TunnelStatus>,
    /// The current runtime process; `None` between restarts and after the
    /// final `take()` of a finished or stopped child.
    child: Mutex<Option<Child>>,
    /// Stop request flag, checked by the supervisor loops.
    stop_requested: AtomicBool,
}

/// Owns the definition store and the live runtime slots. Managed as Tauri
/// state in `main.rs`.
pub struct TunnelRegistry {
    app: AppHandle,
    store: TunnelStore,
    slots: SlotMap,
}

impl TunnelRegistry {
    /// Creates the registry (its store materialises the `tunnels`
    /// subdirectories) and starts the autostart sweep in the background:
    /// tunnels marked `autostart` are started sequentially, after the
    /// window is up, so a slow runtime never delays the GUI.
    pub fn new(app: AppHandle, dir: PathBuf) -> Self {
        let registry = Self {
            app,
            store: TunnelStore::new(dir),
            slots: Arc::new(Mutex::new(HashMap::new())),
        };

        let sweep = thread::Builder::new()
            .name("tunnel-autostart".into())
            .spawn({
                let app = registry.app.clone();
                let store = registry.store.clone();
                let slots = Arc::clone(&registry.slots);
                move || autostart_marked(&app, &store, &slots)
            });
        if let Err(e) = sweep {
            log::warn!("[tunnels] autostart thread failed to start: {e}");
        }
        registry
    }

    /// The machine profile rendered by the view header.
    pub fn profile_info(&self) -> TunnelsProfile {
        let client = client::resolve_default(&self.store.bin_dir());
        TunnelsProfile {
            client_available: client.is_some(),
            client_path: client
                .as_deref()
                .and_then(std::path::Path::to_str)
                .map(str::to_owned),
            managed_bin_dir: self.store.bin_dir().to_string_lossy().into_owned(),
            profiles_dir: self.store.dir().join("profiles").to_string_lossy().into_owned(),
            secrets_dir: self.store.dir().join("secrets").to_string_lossy().into_owned(),
            logs_dir: self.store.dir().join("logs").to_string_lossy().into_owned(),
            health_port_start: health::PORT_RANGE_START,
            health_port_end: health::PORT_RANGE_END,
            platform_tunnels_url: PLATFORM_TUNNELS_URL.to_owned(),
            tunnel_docs_url: TUNNEL_DOCS_URL.to_owned(),
            tunnel_client_repo_url: TUNNEL_CLIENT_REPO_URL.to_owned(),
        }
    }

    /// Loads one definition by id.
    pub fn definition(&self, id: &str) -> McpGoResult<TunnelDefinition> {
        self.store.get(id)
    }

    /// Creates a definition (the store owns validation and id derivation).
    pub fn create_definition(
        &self,
        requested_id: Option<&str>,
        input: &TunnelDefinitionInput,
    ) -> McpGoResult<TunnelDefinition> {
        self.store.create(requested_id, input)
    }

    /// Updates a definition. Refuses while the runtime is active: the
    /// running process keeps using the profile generated at its start, so
    /// an edit would silently diverge from what the UI shows.
    pub fn update_definition(
        &self,
        id: &str,
        input: &TunnelDefinitionInput,
    ) -> McpGoResult<TunnelDefinition> {
        self.ensure_inactive(id, "saving the definition (the running runtime keeps the previous profile)")?;
        self.store.update(id, input)
    }

    /// Deletes a definition. Refuses while the runtime is active (removing
    /// the files under a live process would leave an orphan runtime whose
    /// profile and secret have vanished).
    pub fn delete_definition(&self, id: &str) -> McpGoResult<()> {
        self.ensure_inactive(id, "deleting the definition")?;
        self.store.delete(id)
    }

    /// Stores the managed API key of a definition. Refuses while the
    /// runtime is active and never echoes the key back.
    pub fn set_api_key(&self, id: &str, key: &str) -> McpGoResult<()> {
        let definition = self.store.get(id)?;
        if definition.api_key.kind != ApiKeySourceKind::Managed {
            return Err(McpGoError::invalid_input(
                "this tunnel reads its key from an environment variable or file - \
                 change the key source to \"managed\" to store it with McpGo",
            ));
        }
        self.ensure_inactive(id, "changing the API key")?;
        self.store.set_managed_key(id, key)
    }

    /// Lists every definition joined with its runtime state and the machine
    /// facts of the card, sorted by display name.
    pub fn list_summaries(&self) -> Vec<TunnelSummary> {
        let slots = self.slots.lock().expect("tunnel registry lock poisoned");
        let mut summaries: Vec<TunnelSummary> = self
            .store
            .list()
            .into_iter()
            .map(|definition| {
                let status = match slots.get(&definition.id) {
                    Some(slot) => slot
                        .status
                        .lock()
                        .expect("tunnel status lock poisoned")
                        .clone(),
                    None => TunnelStatus::dormant(&definition.id),
                };
                build_summary(&self.store, definition, status)
            })
            .collect();
        summaries.sort_by(|a, b| {
            a.definition
                .name
                .to_lowercase()
                .cmp(&b.definition.name.to_lowercase())
                .then_with(|| a.definition.id.cmp(&b.definition.id))
        });
        summaries
    }

    /// Starts the runtime of one definition: validates, resolves the
    /// executable, writes the profile, spawns and supervises.
    pub fn start(&self, id: &str) -> McpGoResult<TunnelSummary> {
        start_runtime(&self.app, &self.store, &self.slots, id)
    }

    /// Requests the stop of one running runtime: the process is terminated
    /// and the supervisor finalizes the tunnel as `Stopped` within one
    /// poll interval. Idempotent for tunnels that already stopped.
    pub fn stop(&self, id: &str) -> McpGoResult<TunnelSummary> {
        let slot = {
            let slots = self.slots.lock().expect("tunnel registry lock poisoned");
            slots.get(id).cloned()
        };
        let Some(slot) = slot else {
            return Err(McpGoError::invalid_input(
                "this tunnel is not running (no runtime was started in this application run)",
            ));
        };

        let snapshot = {
            let mut status = slot.status.lock().expect("tunnel status lock poisoned");
            if is_terminal(status.state) {
                return Ok(build_summary(
                    &self.store,
                    self.store.get(id)?,
                    status.clone(),
                ));
            }
            status.stop_requested = true;
            status.clone()
        };

        slot.stop_requested.store(true, Ordering::Relaxed);
        let _ = self.app.emit(
            EVENT_TUNNEL_STATUS,
            TunnelStatusEvent {
                tunnel: snapshot.clone(),
            },
        );
        if let Some(mut child) = slot
            .child
            .lock()
            .expect("tunnel child lock poisoned")
            .take()
        {
            let _ = child.kill();
            // Reap immediately: the supervisor's exit watcher sees the
            // taken child plus the stop flag and finalizes the tunnel.
            let _ = child.wait();
        }
        log::info!("[tunnels] {id} stop requested");
        Ok(build_summary(
            &self.store,
            self.store.get(id)?,
            snapshot,
        ))
    }

    /// Returns the newest ndjson lines of one runtime log (the definition
    /// must exist; a tunnel that never ran reads as an empty log).
    pub fn read_log(&self, id: &str) -> McpGoResult<TunnelLog> {
        self.store.get(id)?; // guards against fabricated ids
        let raw = fs::read_to_string(self.store.log_path(id)).unwrap_or_default();
        let mut lines: Vec<String> = raw.lines().map(str::to_owned).collect();
        if lines.len() > LOG_LINES_LIMIT {
            let split = lines.len() - LOG_LINES_LIMIT;
            lines.drain(..split);
        }
        Ok(TunnelLog {
            id: id.to_owned(),
            lines,
        })
    }

    /// Resolves one slot by id.
    fn slot(&self, id: &str) -> Option<Arc<TunnelSlot>> {
        self.slots
            .lock()
            .expect("tunnel registry lock poisoned")
            .get(id)
            .cloned()
    }

    /// Fails while the runtime of `id` is active; `action` names what is
    /// being refused ("deleting the definition", ...).
    fn ensure_inactive(&self, id: &str, action: &str) -> McpGoResult<()> {
        let active = self
            .slot(id)
            .map(|slot| {
                let state = slot.status.lock().expect("tunnel status lock poisoned").state;
                state == TunnelState::Starting || state == TunnelState::Running
            })
            .unwrap_or(false);
        if active {
            return Err(McpGoError::invalid_input(format!(
                "the tunnel runtime is active - stop it before {action}"
            )));
        }
        Ok(())
    }

    /// Exit cleanup, called from the `RunEvent::Exit` handler in `main.rs`.
    /// Every non-terminal runtime is terminated: they are invisible
    /// background daemons, and letting them run would leak one orphan
    /// tunnel-client (plus its stdio MCP child) per tunnel.
    pub fn shutdown(&self) {
        let slots: Vec<(String, Arc<TunnelSlot>)> = self
            .slots
            .lock()
            .expect("tunnel registry lock poisoned")
            .iter()
            .map(|(id, slot)| (id.clone(), Arc::clone(slot)))
            .collect();

        let mut terminated = 0usize;
        for (id, slot) in &slots {
            let state = slot.status.lock().expect("tunnel status lock poisoned").state;
            if is_terminal(state) {
                continue;
            }
            slot.stop_requested.store(true, Ordering::Relaxed);
            if let Some(mut child) = slot
                .child
                .lock()
                .expect("tunnel child lock poisoned")
                .take()
            {
                let _ = child.kill();
                let _ = child.wait();
            }
            apply_transition(&self.app, slot, |status| {
                status.state = TunnelState::Stopped;
                status.stopped_at = Some(app_time::now_epoch_millis());
            });
            log::info!("[tunnels] shutdown: terminated {id}");
            terminated += 1;
        }
        if terminated > 0 {
            log::info!("[tunnels] shutdown: terminated {terminated} runtime(s)");
        }
    }
}

// ----------------------------------------------------------------------------
// Start / autostart (free functions so threads need no &Registry borrow)
// ----------------------------------------------------------------------------

/// The engine of [`TunnelRegistry::start`]: validation, executable
/// resolution, profile generation, spawn and supervision registration.
/// Every failure path leaves no partial registration behind.
fn start_runtime(
    app: &AppHandle,
    store: &TunnelStore,
    slots: &SlotMap,
    id: &str,
) -> McpGoResult<TunnelSummary> {
    let definition = store.get(id)?;

    // One runtime per tunnel is the protocol-level deployment limit - a
    // second instance on the same tunnel id is not supported (the client
    // documentation is explicit about stdio targets).
    {
        let registry = slots.lock().expect("tunnel registry lock poisoned");
        if let Some(slot) = registry.get(id) {
            let state = slot.status.lock().expect("tunnel status lock poisoned").state;
            if state == TunnelState::Starting || state == TunnelState::Running {
                return Err(McpGoError::invalid_input(
                    "this tunnel is already running - stop it first (one runtime \
                     per tunnel is a protocol requirement)",
                ));
            }
        }
    }

    // Resolve the executable (override -> PATH -> managed bin directory).
    let exe = client::resolve(definition.client_path.as_deref(), &store.bin_dir())?;

    // Fail fast when the API key source is not ready: the client would
    // reject its config at startup, but that error would only surface in
    // the log file.
    require_api_key_ready(store, &definition)?;

    // Health port: pinned by the definition or allocated from the McpGo
    // range. A pinned port that already answers belongs to a leftover
    // runtime of a crashed session - refuse with the recovery hint.
    let port = match definition.health_port {
        Some(pinned) => {
            if health::health_alive(pinned) {
                return Err(McpGoError::operation(
                    "starting the tunnel runtime",
                    &format!(
                        "health port {pinned} already serves a listener (a leftover \
                         runtime of a previous session?) - stop the process or pick \
                         another port"
                    ),
                ));
            }
            pinned
        }
        None => health::allocate_health_port().ok_or_else(|| {
            McpGoError::operation(
                "allocating a health port",
                &format!(
                    "no free port in the range {}..={}",
                    health::PORT_RANGE_START,
                    health::PORT_RANGE_END
                ),
            )
        })?,
    };

    // Generate and write the profile (rewritten on every start - the
    // definition may have changed since the last run).
    let secret_path = store.secret_path(&definition.id);
    let api_key_reference = profile::api_key_reference(&definition, &secret_path);
    let profile_path = store.profile_path(&definition.id);
    profile::write(
        &definition,
        &api_key_reference,
        port,
        &profile_path,
        &store.log_path(&definition.id),
    )?;

    // Spawn the runtime.
    let args = client::build_run_args(&profile_path);
    let child = client::launch(&exe, &args, store.dir())
        .map_err(|e| McpGoError::io("launch tunnel-client", &e))?;
    let pid = child.id();

    // Register the slot and announce the initial state.
    let now = app_time::now_epoch_millis();
    let status = TunnelStatus {
        definition_id: definition.id.clone(),
        state: TunnelState::Starting,
        pid: Some(pid),
        health_port: Some(port),
        health_url: Some(format!("http://{}:{port}{}", health::HOST, health::HEALTHZ_PATH)),
        ui_url: Some(format!("http://{}:{port}/ui", health::HOST)),
        started_at: Some(now),
        confirmed_at: None,
        stopped_at: None,
        exit_code: None,
        restarts: 0,
        stop_requested: false,
        error: None,
    };
    let slot = Arc::new(TunnelSlot {
        status: Mutex::new(status.clone()),
        child: Mutex::new(Some(child)),
        stop_requested: AtomicBool::new(false),
    });
    slots
        .lock()
        .expect("tunnel registry lock poisoned")
        .insert(definition.id.clone(), Arc::clone(&slot));

    let supervisor = thread::Builder::new()
        .name(format!("{id}-supervisor"))
        .spawn({
            let app = app.clone();
            let store = store.clone();
            let slot = Arc::clone(&slot);
            let id = definition.id.clone();
            let exe = exe.clone();
            let profile_path = profile_path.clone();
            move || supervise_runtime(app, store, slot, id, exe, profile_path, port, definition.keep_alive)
        });
    if supervisor.is_err() {
        // Undo the registration and terminate the freshly spawned runtime:
        // a slot without a supervisor would never observe the process exit.
        slots
            .lock()
            .expect("tunnel registry lock poisoned")
            .remove(&definition.id);
        if let Some(mut taken) = slot
            .child
            .lock()
            .expect("tunnel child lock poisoned")
            .take()
        {
            let _ = taken.kill();
        }
        return Err(McpGoError::internal(
            "failed to spawn the tunnel supervisor thread",
        ));
    }

    let _ = app.emit(
        EVENT_TUNNEL_STATUS,
        TunnelStatusEvent {
            tunnel: status.clone(),
        },
    );
    log::info!(
        "[tunnels] {id} starting on health port {port} (pid {pid}, profile {})",
        profile_path.display()
    );
    Ok(build_summary(store, definition, status))
}

/// Starts every definition marked `autostart`, sequentially, logging the
/// outcome of each. Runs on the background sweep thread of
/// [`TunnelRegistry::new`].
fn autostart_marked(app: &AppHandle, store: &TunnelStore, slots: &SlotMap) {
    let marked: Vec<String> = store
        .list()
        .into_iter()
        .filter(|definition| definition.autostart)
        .map(|definition| definition.id)
        .collect();
    if marked.is_empty() {
        return;
    }
    log::info!("[tunnels] autostart: {} tunnel(s) marked", marked.len());
    for id in marked {
        match start_runtime(app, store, slots, &id) {
            Ok(_) => log::info!("[tunnels] autostart: {id} started"),
            Err(e) => log::warn!("[tunnels] autostart: {id} failed: {e}"),
        }
    }
}

// ----------------------------------------------------------------------------
// Supervision (free functions so the registry stays lock-agnostic)
// ----------------------------------------------------------------------------

/// `true` for terminal states - transitions into them are final.
fn is_terminal(state: TunnelState) -> bool {
    matches!(state, TunnelState::Stopped | TunnelState::Failed)
}

/// Applies one state mutation under the status lock, emits the
/// `tunnel://status` event. Terminal states are immutable: a late
/// supervisor poll can never resurrect a tunnel.
fn apply_transition(
    app: &AppHandle,
    slot: &TunnelSlot,
    mutate: impl FnOnce(&mut TunnelStatus),
) -> TunnelStatus {
    let mut status = slot.status.lock().expect("tunnel status lock poisoned");
    if is_terminal(status.state) {
        return status.clone();
    }
    mutate(&mut status);
    let snapshot = status.clone();
    drop(status);
    let _ = app.emit(
        EVENT_TUNNEL_STATUS,
        TunnelStatusEvent {
            tunnel: snapshot.clone(),
        },
    );
    snapshot
}

/// Finalizes a tunnel as `Stopped` after killing any child still held in
/// the slot (covers every finalize path - user stop, clean exit, shutdown).
fn finalize_stopped(app: &AppHandle, slot: &TunnelSlot, id: &str, exit_code: Option<i32>) {
    if let Some(mut child) = slot
        .child
        .lock()
        .expect("tunnel child lock poisoned")
        .take()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
    apply_transition(app, slot, |status| {
        status.state = TunnelState::Stopped;
        status.exit_code = exit_code;
        status.stopped_at = Some(app_time::now_epoch_millis());
        status.error = None;
    });
    log::info!("[tunnels] {id} stopped");
}

/// Records a `Failed` state after killing any child still held in the slot.
fn finalize_failed(app: &AppHandle, slot: &TunnelSlot, id: &str, exit_code: Option<i32>, error: &str) {
    if let Some(mut child) = slot
        .child
        .lock()
        .expect("tunnel child lock poisoned")
        .take()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
    apply_transition(app, slot, |status| {
        status.state = TunnelState::Failed;
        status.exit_code = exit_code;
        status.stopped_at = Some(app_time::now_epoch_millis());
        status.error = Some(error.to_owned());
    });
    log::warn!("[tunnels] {id} failed: {error}");
}

/// Respawns the runtime for an automatic (keep_alive) restart. Returns the
/// new pid, or `None` when the spawn itself failed (the caller gives up).
fn respawn(store: &TunnelStore, slot: &TunnelSlot, exe: &std::path::Path, profile_path: &std::path::Path) -> Option<u32> {
    let args = client::build_run_args(profile_path);
    match client::launch(exe, &args, store.dir()) {
        Ok(child) => {
            let pid = child.id();
            *slot.child.lock().expect("tunnel child lock poisoned") = Some(child);
            Some(pid)
        }
        Err(e) => {
            log::error!("[tunnels] automatic restart failed to spawn: {e}");
            None
        }
    }
}

/// Supervises one tunnel's runtime:
///
/// * Phase A - confirm the health endpoint answers within
///   [`CONFIRM_TIMEOUT`]; a runtime that dies early or never comes up
///   counts as a start failure.
/// * Phase B - watch the process (polls every [`EXIT_POLL`]) and keep
///   probing the health endpoint every [`HEALTH_POLL`]; clean exits become
///   `Stopped`, unexpected exits and health blackouts become `Failed`
///   (with the keep_alive restart instead).
///
/// keep_alive restarts are bounded by [`START_FAILURE_LIMIT`] consecutive
/// start failures; the counter resets once a runtime confirmed health.
fn supervise_runtime(
    app: AppHandle,
    store: TunnelStore,
    slot: Arc<TunnelSlot>,
    id: String,
    exe: PathBuf,
    profile_path: PathBuf,
    port: u16,
    keep_alive: bool,
) {
    let mut consecutive_start_failures: u32 = 0;

    'runtime: loop {
        // --- Phase A: wait for the health endpoint ----------------------
        let deadline = Instant::now() + CONFIRM_TIMEOUT;
        let mut early_exit: Option<Option<i32>> = None; // Some(code) = process died
        let mut confirmed = false;
        while Instant::now() < deadline {
            if slot.stop_requested.load(Ordering::Relaxed) {
                // stop() already took and killed the child.
                finalize_stopped(&app, &slot, &id, None);
                return;
            }
            {
                let mut child = slot.child.lock().expect("tunnel child lock poisoned");
                match child.as_mut().map(Child::try_wait) {
                    Some(Ok(Some(exit))) => {
                        let code = exit.code();
                        child.take();
                        drop(child);
                        early_exit = Some(code);
                        break;
                    }
                    Some(Err(e)) => {
                        log::warn!("[tunnels] {id}: waiting for the child failed: {e}");
                    }
                    Some(Ok(None)) | None => {}
                }
            }
            if health::health_alive(port) {
                confirmed = true;
                break;
            }
            thread::sleep(CONFIRM_POLL);
        }

        if !confirmed {
            // A start failure: the runtime exited early or its health
            // endpoint never answered (bad key, dead control plane, stolen
            // port - the log file carries the client's own diagnosis).
            let code = early_exit.flatten();
            let error = match code {
                Some(code) => format!(
                    "the runtime exited before the health endpoint answered (code {code})"
                ),
                None => format!(
                    "the health endpoint did not answer within {} seconds",
                    CONFIRM_TIMEOUT.as_secs()
                ),
            };

            if keep_alive {
                consecutive_start_failures += 1;
                if consecutive_start_failures < START_FAILURE_LIMIT {
                    let delay = RESTART_DELAY;
                    apply_transition(&app, &slot, |status| {
                        status.restarts += 1;
                        status.exit_code = code;
                    });
                    log::warn!(
                        "[tunnels] {id} start failure {consecutive_start_failures}/\
                         {START_FAILURE_LIMIT} - restarting in {} ms",
                        delay.as_millis()
                    );
                    thread::sleep(delay);
                    if slot.stop_requested.load(Ordering::Relaxed) {
                        finalize_stopped(&app, &slot, &id, code);
                        return;
                    }
                    if let Some(pid) = respawn(&store, &slot, &exe, &profile_path) {
                        apply_transition(&app, &slot, |status| {
                            status.pid = Some(pid);
                            status.started_at = Some(app_time::now_epoch_millis());
                        });
                        continue 'runtime;
                    }
                    finalize_failed(&app, &slot, &id, code, "the automatic restart could not spawn the runtime");
                    return;
                }
                // else: start-failure budget exhausted -> fall through to
                // the terminal Failed transition below.
            }

            finalize_failed(&app, &slot, &id, code, &error);
            return;
        }

        consecutive_start_failures = 0;
        apply_transition(&app, &slot, |status| {
            status.state = TunnelState::Running;
            status.confirmed_at = Some(app_time::now_epoch_millis());
            status.error = None;
        });
        log::info!("[tunnels] {id} running on health port {port}");

        // --- Phase B: watch the process and the health endpoint ---------
        let mut health_misses: u32 = 0;
        let mut last_probe = Instant::now();
        loop {
            if slot.stop_requested.load(Ordering::Relaxed) {
                // The exit code (if the kill already landed) is recorded
                // by reading whatever the child left behind.
                let code = take_exit_code(&slot);
                finalize_stopped(&app, &slot, &id, code);
                return;
            }
            {
                let mut child = slot.child.lock().expect("tunnel child lock poisoned");
                match child.as_mut().map(Child::try_wait) {
                    Some(Ok(Some(exit))) => {
                        let code = exit.code();
                        child.take();
                        drop(child);
                        if slot.stop_requested.load(Ordering::Relaxed) {
                            finalize_stopped(&app, &slot, &id, code);
                        } else if keep_alive {
                            // Any exit of a daemon that should run forever
                            // is unexpected; flip back to Starting (the
                            // restart window must not read as running) and
                            // restart with the recorded code.
                            apply_transition(&app, &slot, |status| {
                                status.state = TunnelState::Starting;
                                status.restarts += 1;
                                status.exit_code = code;
                            });
                            log::warn!(
                                "[tunnels] {id} exited with code {code:?} - restarting"
                            );
                            thread::sleep(RESTART_DELAY);
                            if slot.stop_requested.load(Ordering::Relaxed) {
                                finalize_stopped(&app, &slot, &id, code);
                                return;
                            }
                            if let Some(pid) = respawn(&store, &slot, &exe, &profile_path) {
                                apply_transition(&app, &slot, |status| {
                                    status.pid = Some(pid);
                                    status.started_at = Some(app_time::now_epoch_millis());
                                });
                                continue 'runtime;
                            }
                            finalize_failed(
                                &app,
                                &slot,
                                &id,
                                code,
                                "the automatic restart could not spawn the runtime",
                            );
                        } else if code.map(|c| c != 0).unwrap_or(false) {
                            finalize_failed(
                                &app,
                                &slot,
                                &id,
                                code,
                                &format!("the runtime exited with code {}", code.unwrap_or_default()),
                            );
                        } else {
                            finalize_stopped(&app, &slot, &id, code);
                        }
                        return;
                    }
                    Some(Err(e)) => {
                        log::warn!("[tunnels] {id}: waiting for the child failed: {e}");
                    }
                    Some(Ok(None)) | None => {}
                }
            }
            if last_probe.elapsed() >= HEALTH_POLL {
                last_probe = Instant::now();
                if health::health_alive(port) {
                    health_misses = 0;
                } else {
                    health_misses += 1;
                    if health_misses >= HEALTH_MISS_LIMIT {
                        finalize_failed(
                            &app,
                            &slot,
                            &id,
                            None,
                            "the health endpoint stopped answering (runtime wedged)",
                        );
                        return;
                    }
                }
            }
            thread::sleep(EXIT_POLL);
        }
    }
}

/// Reads (and clears) the child of a slot after a stop request, returning
/// its exit code when the kill already landed and was reaped.
fn take_exit_code(slot: &TunnelSlot) -> Option<i32> {
    match slot
        .child
        .lock()
        .expect("tunnel child lock poisoned")
        .as_mut()
        .map(Child::try_wait)
    {
        // The child is still there (stop() did not reap it yet) - poll it
        // once; the kill of stop() makes this immediate in practice.
        Some(Ok(Some(exit))) => exit.code(),
        _ => None,
    }
}

// ----------------------------------------------------------------------------
// Summary assembly (free function: unit-testable without a registry)
// ----------------------------------------------------------------------------

/// Assembles one row of the tunnels view: the definition, its live status
/// and the machine facts (client resolution, key readiness, paths).
pub fn build_summary(
    store: &TunnelStore,
    definition: TunnelDefinition,
    status: TunnelStatus,
) -> TunnelSummary {
    let resolved = client::resolve(definition.client_path.as_deref(), &store.bin_dir()).ok();
    let secret_path = if definition.api_key.kind == ApiKeySourceKind::Managed {
        Some(
            store
                .secret_path(&definition.id)
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    };
    TunnelSummary {
        client_available: resolved.is_some(),
        client_path_used: resolved
            .as_deref()
            .and_then(std::path::Path::to_str)
            .map(str::to_owned),
        api_key_ready: api_key_ready(store, &definition),
        secret_path,
        profile_path: store
            .profile_path(&definition.id)
            .to_string_lossy()
            .into_owned(),
        log_path: store
            .log_path(&definition.id)
            .to_string_lossy()
            .into_owned(),
        definition_path: store
            .definition_path(&definition.id)
            .to_string_lossy()
            .into_owned(),
        definition,
        status,
    }
}

/// `true` when the definition's API key source is usable right now:
/// the managed secret file exists / the environment variable is set /
/// the referenced key file exists.
fn api_key_ready(store: &TunnelStore, definition: &TunnelDefinition) -> bool {
    match &definition.api_key {
        ApiKeySource {
            kind: ApiKeySourceKind::Managed,
            ..
        } => store.secret_path(&definition.id).is_file(),
        ApiKeySource {
            kind: ApiKeySourceKind::Env,
            reference: Some(name),
        } => std::env::var_os(name).is_some(),
        ApiKeySource {
            kind: ApiKeySourceKind::File,
            reference: Some(path),
        } => std::path::Path::new(path).is_file(),
        _ => false,
    }
}

/// The fail-fast gate of [`start_runtime`]: an unusable key source stops
/// the start with an actionable message naming the source.
fn require_api_key_ready(store: &TunnelStore, definition: &TunnelDefinition) -> McpGoResult<()> {
    match (&definition.api_key.kind, api_key_ready(store, definition)) {
        (_, true) => Ok(()),
        (ApiKeySourceKind::Managed, _) => Err(McpGoError::invalid_input(format!(
            "the managed API key file {} does not exist yet - use the Set key action \
             on the tunnel card",
            store.secret_path(&definition.id).display()
        ))),
        (ApiKeySourceKind::Env, _) => Err(McpGoError::invalid_input(format!(
            "the environment variable {} is not set in this application's \
             environment (set it before launching McpGo, or switch to a managed \
             or file key)",
            definition.api_key.reference.as_deref().unwrap_or_default()
        ))),
        (ApiKeySourceKind::File, _) => Err(McpGoError::invalid_input(format!(
            "the key file {} does not exist",
            definition.api_key.reference.as_deref().unwrap_or_default()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states_are_stopped_and_failed() {
        assert!(is_terminal(TunnelState::Stopped));
        assert!(is_terminal(TunnelState::Failed));
        assert!(!is_terminal(TunnelState::Starting));
        assert!(!is_terminal(TunnelState::Running));
    }

    #[test]
    fn states_serialize_lowercase() {
        assert_eq!(
            serde_json::to_string(&TunnelState::Starting).expect("serialize"),
            "\"starting\""
        );
        assert_eq!(
            serde_json::to_string(&TunnelState::Failed).expect("serialize"),
            "\"failed\""
        );
        assert_eq!(TunnelState::Running.to_string(), "running");
    }

    #[test]
    fn dormant_status_is_stopped_with_no_runtime_facts() {
        let status = TunnelStatus::dormant("files");
        assert_eq!(status.state, TunnelState::Stopped);
        assert_eq!(status.pid, None);
        assert_eq!(status.health_port, None);
        assert_eq!(status.restarts, 0);
        assert_eq!(status.definition_id, "files");
    }

    #[test]
    fn the_health_urls_follow_the_documented_paths() {
        let status = TunnelStatus {
            definition_id: "files".into(),
            state: TunnelState::Starting,
            pid: Some(4242),
            health_port: Some(18123),
            health_url: Some(format!(
                "http://{}:18123{}",
                health::HOST,
                health::HEALTHZ_PATH
            )),
            ui_url: Some(format!("http://{}:18123/ui", health::HOST)),
            started_at: Some(1),
            confirmed_at: None,
            stopped_at: None,
            exit_code: None,
            restarts: 0,
            stop_requested: false,
            error: None,
        };
        assert_eq!(status.health_url.as_deref(), Some("http://127.0.0.1:18123/healthz"));
        assert_eq!(status.ui_url.as_deref(), Some("http://127.0.0.1:18123/ui"));
    }

    #[test]
    fn managed_key_readiness_follows_the_secret_file() {
        // A throwaway store directory in the system temp dir (the sandbox
        // runs the tests on the build host; the FS semantics are identical).
        let base = std::env::temp_dir().join(format!(
            "mcpgo-tunnels-test-{}",
            app_time::now_epoch_millis()
        ));
        let store = TunnelStore::new(base.clone());

        // The definition must exist before its managed key can be written
        // (set_managed_key validates the id against the store).
        let input = TunnelDefinitionInput {
            name: "Files".into(),
            description: String::new(),
            tunnel_id: "tunnel_0123456789abcdef0123456789abcdef".into(),
            target_kind: TunnelTargetKind::Http,
            command: None,
            args: Vec::new(),
            working_dir: None,
            server_url: Some("http://127.0.0.1:3000/mcp".into()),
            api_key: ApiKeySource {
                kind: ApiKeySourceKind::Managed,
                reference: None,
            },
            residency: Residency::Global,
            autostart: false,
            keep_alive: false,
            client_path: None,
            health_port: None,
        };
        let created = store.create(Some("files"), &input).expect("create definition");

        // Without the secret file the summary flags the key as not ready
        // and the start gate refuses.
        assert!(!api_key_ready(&store, &created));
        assert!(require_api_key_ready(&store, &created).is_err());

        // ... and the readiness error names the Set key action.
        let message = require_api_key_ready(&store, &created)
            .unwrap_err()
            .to_string();
        assert!(message.contains("Set key"), "unexpected: {message}");

        // With the secret file present the same checks pass.
        store
            .set_managed_key("files", "sk-test-value-not-a-real-key")
            .expect("write the managed key");
        assert!(api_key_ready(&store, &created));
        assert!(require_api_key_ready(&store, &created).is_ok());

        // The summary carries the secret path and no key material.
        let summary = build_summary(&store, created, TunnelStatus::dormant("files"));
        assert!(summary.api_key_ready);
        assert_eq!(
            summary.secret_path.as_deref(),
            Some(
                store
                    .secret_path("files")
                    .to_string_lossy()
                    .into_owned()
                    .as_str()
            )
        );

        let _ = std::fs::remove_dir_all(base);
    }
}
