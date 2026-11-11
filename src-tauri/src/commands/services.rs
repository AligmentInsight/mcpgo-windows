//! Services IPC surface: CRUD for service definitions, the joined
//! overview rendered by the services view, and the elevated SCM write
//! side (install / uninstall / start / stop / restart).
//!
//! Mapping to the macOS original:
//!
//! | macOS concept              | Command here                          |
//! |----------------------------|---------------------------------------|
//! | list LaunchAgents          | [`list_services`] (files + SCM state) |
//! | write / edit a plist       | [`create_service_definition`],        |
//! |                            | [`update_service_definition`]         |
//! | unload and delete a plist  | [`delete_service_definition`]         |
//! | launchctl load             | [`install_service`] (+ start)         |
//! | launchctl unload / bootout | [`uninstall_service`]                 |
//! | launchctl kickstart -k     | [`start_service`], [`stop_service`],  |
//! |                            | [`restart_service`]                   |
//!
//! The write side runs through `services::installer` (self-elevating
//! PowerShell scripts, UAC prompt included); handlers stay thin
//! adapters, exactly like the terminal commands.

use tauri::State;

use crate::error::McpGoResult;
use crate::services::installer::{self, ServiceAction};
use crate::services::{
    ServiceDefinition, ServiceDefinitionInput, ServiceLog, ServiceStore, ServiceSummary,
};

/// Lists every service definition joined with its live SCM state, sorted by
/// display name. One PowerShell invocation covers the whole list.
#[tauri::command]
pub async fn list_services(
    store: State<'_, ServiceStore>,
) -> McpGoResult<Vec<ServiceSummary>> {
    store.list_summaries()
}

/// Loads one service definition by id.
#[tauri::command]
pub async fn get_service_definition(
    store: State<'_, ServiceStore>,
    id: String,
) -> McpGoResult<ServiceDefinition> {
    store.get(&id)
}

/// Creates a definition. The `id` argument is optional: when omitted (null),
/// the id is derived from the slugified name; when given it must satisfy
/// the id rules and be unused.
#[tauri::command]
pub async fn create_service_definition(
    store: State<'_, ServiceStore>,
    id: Option<String>,
    input: ServiceDefinitionInput,
) -> McpGoResult<ServiceDefinition> {
    store.create(id.as_deref(), &input)
}

/// Updates an existing definition. The id is immutable (it names both the
/// definition file and the SCM service).
#[tauri::command]
pub async fn update_service_definition(
    store: State<'_, ServiceStore>,
    id: String,
    input: ServiceDefinitionInput,
) -> McpGoResult<ServiceDefinition> {
    store.update(&id, &input)
}

/// Deletes one definition. Refuses while the Windows service is still
/// installed in the SCM (see `ServiceStore::delete_definition`).
#[tauri::command]
pub async fn delete_service_definition(
    store: State<'_, ServiceStore>,
    id: String,
) -> McpGoResult<()> {
    store.delete_definition(&id)
}

/// Installs the service of definition `id` in the Service Control
/// Manager (the `launchctl load` of the port). A UAC prompt appears
/// unless the application already runs elevated.
///
/// Returns the refreshed summary: the service exists and reports its
/// live SCM state (Stopped for a manual service, Running when the
/// install started it).
#[tauri::command]
pub async fn install_service(
    store: State<'_, ServiceStore>,
    id: String,
) -> McpGoResult<ServiceSummary> {
    installer::install_service(store.inner(), &id)
}

/// Uninstalls (stops and deletes) the service of definition `id`. The
/// definition file stays and can be reinstalled at any time. A UAC
/// prompt appears unless the application already runs elevated.
#[tauri::command]
pub async fn uninstall_service(
    store: State<'_, ServiceStore>,
    id: String,
) -> McpGoResult<ServiceSummary> {
    installer::uninstall_service(store.inner(), &id)
}

/// Starts the installed service of definition `id` (UAC prompt).
#[tauri::command]
pub async fn start_service(
    store: State<'_, ServiceStore>,
    id: String,
) -> McpGoResult<ServiceSummary> {
    installer::control_service(store.inner(), &id, ServiceAction::Start)
}

/// Stops the running service of definition `id` (UAC prompt). The
/// service worker terminates the runtime process tree first.
#[tauri::command]
pub async fn stop_service(
    store: State<'_, ServiceStore>,
    id: String,
) -> McpGoResult<ServiceSummary> {
    installer::control_service(store.inner(), &id, ServiceAction::Stop)
}

/// Restarts the service of definition `id` (UAC prompt).
#[tauri::command]
pub async fn restart_service(
    store: State<'_, ServiceStore>,
    id: String,
) -> McpGoResult<ServiceSummary> {
    installer::control_service(store.inner(), &id, ServiceAction::Restart)
}

/// Returns the parsed log records of definition `id`, oldest first. The
/// service worker appends runtime output and its own system records to
/// `<data>\services\logs\<id>.log` (the same canonical grammar as the
/// job logs); a service that never ran reads as an empty log.
#[tauri::command]
pub async fn read_service_log(
    store: State<'_, ServiceStore>,
    id: String,
) -> McpGoResult<ServiceLog> {
    store.read_log(&id)
}
