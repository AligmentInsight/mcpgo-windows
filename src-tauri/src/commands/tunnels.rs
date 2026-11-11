//! Tunnels IPC surface: CRUD for tunnel definitions, the joined overview
//! of the tunnels view, the managed API key of a tunnel, the runtime
//! start/stop lifecycle and the runtime log.
//!
//! Mapping to the OpenAI Secure MCP Tunnel workflow:
//!
//! | Workflow step                            | Command here               |
//! |------------------------------------------|----------------------------|
//! | create a tunnel in the Platform settings | (link in the view header - |
//! |                                          | McpGo does not duplicate   |
//! |                                          | the management surface)    |
//! | describe it locally (target + key)       | [`create_tunnel_definition`]|
//! | store the runtime API key                | [`set_tunnel_api_key`]     |
//! | run / stop the runtime                   | [`start_tunnel`],          |
//! |                                          | [`stop_tunnel`]            |
//! | read the runtime's ndjson log            | [`read_tunnel_log`]        |
//!
//! Handlers stay thin adapters, exactly like the service commands; the
//! guards (one runtime per tunnel, no edits while active, no key material
//! over IPC) live in the registry.

use tauri::State;

use crate::error::McpGoResult;
use crate::tunnels::{
    TunnelDefinition, TunnelDefinitionInput, TunnelLog, TunnelRegistry, TunnelSummary,
    TunnelsProfile,
};

/// Returns the machine profile of the tunnels view header: where the
/// tunnel-client executable is expected (PATH and the managed bin
/// directory), where profiles, secrets and logs live, the managed health
/// port range and the OpenAI links (tunnel management, docs, releases).
#[tauri::command]
pub async fn tunnels_profile(registry: State<'_, TunnelRegistry>) -> TunnelsProfile {
    registry.profile_info()
}

/// Lists every tunnel definition joined with its live runtime state and
/// the machine facts of the card, sorted by display name.
#[tauri::command]
pub async fn list_tunnels(
    registry: State<'_, TunnelRegistry>,
) -> McpGoResult<Vec<TunnelSummary>> {
    Ok(registry.list_summaries())
}

/// Loads one tunnel definition by id (never contains key material).
#[tauri::command]
pub async fn get_tunnel_definition(
    registry: State<'_, TunnelRegistry>,
    id: String,
) -> McpGoResult<TunnelDefinition> {
    registry.definition(&id)
}

/// Creates a definition. The `id` argument is optional: when omitted
/// (null), the id is derived from the slugified name; when given it must
/// satisfy the id rules and be unused.
#[tauri::command]
pub async fn create_tunnel_definition(
    registry: State<'_, TunnelRegistry>,
    id: Option<String>,
    input: TunnelDefinitionInput,
) -> McpGoResult<TunnelDefinition> {
    registry.create_definition(id.as_deref(), &input)
}

/// Updates an existing definition. Refuses while the runtime is active
/// (the running process keeps the profile generated at its start).
#[tauri::command]
pub async fn update_tunnel_definition(
    registry: State<'_, TunnelRegistry>,
    id: String,
    input: TunnelDefinitionInput,
) -> McpGoResult<TunnelDefinition> {
    registry.update_definition(&id, &input)
}

/// Deletes one definition. Refuses while the runtime is active; removes
/// the definition file, the generated profile and the managed secret (the
/// log file stays behind as evidence).
#[tauri::command]
pub async fn delete_tunnel_definition(
    registry: State<'_, TunnelRegistry>,
    id: String,
) -> McpGoResult<()> {
    registry.delete_definition(&id)
}

/// Stores the managed API key of definition `id` in
/// `<tunnels>\secrets\<id>.key`. The key is written, never logged and
/// never returned; refuses for non-managed key sources and while the
/// runtime is active.
#[tauri::command]
pub async fn set_tunnel_api_key(
    registry: State<'_, TunnelRegistry>,
    id: String,
    key: String,
) -> McpGoResult<()> {
    registry.set_api_key(&id, &key)
}

/// Starts the runtime of definition `id`: generates the profile, spawns
/// `tunnel-client run --profile-file ...` and supervises it. Resolves
/// with the initial "starting" summary; the "running" (or "failed")
/// status arrives through `tunnel://status` events.
#[tauri::command]
pub async fn start_tunnel(
    registry: State<'_, TunnelRegistry>,
    id: String,
) -> McpGoResult<TunnelSummary> {
    registry.start(&id)
}

/// Requests the stop of the running runtime. Resolves with the current
/// snapshot (stopRequested: true); the final "stopped" status arrives
/// through a `tunnel://status` event. Idempotent.
#[tauri::command]
pub async fn stop_tunnel(
    registry: State<'_, TunnelRegistry>,
    id: String,
) -> McpGoResult<TunnelSummary> {
    registry.stop(&id)
}

/// Returns the newest ndjson lines of the runtime log of definition `id`
/// (written by tunnel-client into `<tunnels>\logs\<id>.log`). A tunnel
/// that never ran reads as an empty log.
#[tauri::command]
pub async fn read_tunnel_log(
    registry: State<'_, TunnelRegistry>,
    id: String,
) -> McpGoResult<TunnelLog> {
    registry.read_log(&id)
}
