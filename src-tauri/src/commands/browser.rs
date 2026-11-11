//! Browser IPC surface: the Chrome/CDP session lifecycle used by the
//! frontend.
//!
//! Mapping to the macOS original:
//!
//! | macOS concept                   | Command here                      |
//! |---------------------------------|-----------------------------------|
//! | Playwright daemon browser info  | [`browser_profile`]                |
//! | daemon registry of live Chromes| [`list_browser_sessions`]          |
//! | daemon launches Chrome          | [`launch_browser`]                 |
//! | daemon closes the browser       | [`stop_browser_session`]           |
//! | daemon reports open tabs       | [`read_browser_targets`]           |
//!
//! Trust model: identical to the terminal domain. Launch parameters are
//! entered by the local user in the McpGo UI; the Chrome they start runs
//! with the user's own privileges on their own machine. The only outbound
//! traffic is loopback DevTools probes.

use tauri::State;

use crate::browser::{BrowserProfile, BrowserRegistry, BrowserSession, BrowserTargetList, LaunchOptions};
use crate::error::McpGoResult;

/// Returns the machine's browser profile information (Chrome path, default
/// and managed profile directories, port range). Cheap path probes,
/// recomputed on every call.
#[tauri::command]
pub async fn browser_profile(
    registry: State<'_, BrowserRegistry>,
) -> McpGoResult<BrowserProfile> {
    Ok(registry.profile())
}

/// Lists the sessions of this application run, newest first: McpGo-launched
/// Chromes plus the DevTools endpoints discovered at startup.
#[tauri::command]
pub async fn list_browser_sessions(
    registry: State<'_, BrowserRegistry>,
) -> McpGoResult<Vec<BrowserSession>> {
    Ok(registry.list_sessions())
}

/// Returns the current snapshot of one session.
#[tauri::command]
pub async fn get_browser_session(
    registry: State<'_, BrowserRegistry>,
    session_id: String,
) -> McpGoResult<BrowserSession> {
    registry.get_session(&session_id)
}

/// Launches one debug-enabled Chrome session and returns its initial
/// `Starting` status. The transition to `Running` (or `Failed`) is announced
/// through the `browser://status` event.
#[tauri::command]
pub async fn launch_browser(
    registry: State<'_, BrowserRegistry>,
    options: LaunchOptions,
) -> McpGoResult<BrowserSession> {
    registry.launch(options)
}

/// Requests the stop of one launched session. Returns the current snapshot
/// (`stopRequested: true`); the final `Stopped` status arrives through the
/// `browser://status` event. Discovered sessions cannot be stopped here.
#[tauri::command]
pub async fn stop_browser_session(
    registry: State<'_, BrowserRegistry>,
    session_id: String,
) -> McpGoResult<BrowserSession> {
    registry.stop_session(&session_id)
}

/// Reads the live DevTools targets (tabs, workers, background pages) of
/// one RUNNING session, joined with the endpoint's version string. The
/// targets panel polls this while it is open.
#[tauri::command]
pub async fn read_browser_targets(
    registry: State<'_, BrowserRegistry>,
    session_id: String,
) -> McpGoResult<BrowserTargetList> {
    registry.read_targets(&session_id)
}
