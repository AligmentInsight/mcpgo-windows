// IPC wrappers for the service commands implemented in
// src-tauri/src/commands/services.rs: definition CRUD, the joined
// overview and the elevated SCM write side (install / uninstall / start /
// stop / restart - each shows a UAC prompt unless the app already runs
// elevated; the call blocks until the elevated script finishes).

import { invoke } from "@tauri-apps/api/core";
import type {
  ServiceDefinition,
  ServiceDefinitionInput,
  ServiceLog,
  ServiceSummary,
} from "../types/service";

/**
 * Lists every service definition joined with its live SCM state, sorted by
 * display name.
 */
export function listServices(): Promise<ServiceSummary[]> {
  return invoke<ServiceSummary[]>("list_services");
}

/** Loads one service definition by id. */
export function getServiceDefinition(id: string): Promise<ServiceDefinition> {
  return invoke<ServiceDefinition>("get_service_definition", { id });
}

/**
 * Creates a definition. `id` is optional: when omitted the backend derives
 * it from the slugified name.
 */
export function createServiceDefinition(
  input: ServiceDefinitionInput,
  id?: string,
): Promise<ServiceDefinition> {
  return invoke<ServiceDefinition>("create_service_definition", {
    input,
    id: id ?? null,
  });
}

/**
 * Updates an existing definition. The id is immutable (it names both the
 * definition file and the SCM service).
 */
export function updateServiceDefinition(
  id: string,
  input: ServiceDefinitionInput,
): Promise<ServiceDefinition> {
  return invoke<ServiceDefinition>("update_service_definition", { id, input });
}

/**
 * Deletes one definition. The backend refuses while the Windows service is
 * still installed in the SCM.
 */
export function deleteServiceDefinition(id: string): Promise<void> {
  return invoke<void>("delete_service_definition", { id });
}

/**
 * Installs the definition as a Windows service (SCM). Returns the summary
 * refreshed from the live SCM state.
 */
export function installService(id: string): Promise<ServiceSummary> {
  return invoke<ServiceSummary>("install_service", { id });
}

/**
 * Stops and deletes the installed Windows service. The definition file
 * stays and can be reinstalled at any time.
 */
export function uninstallService(id: string): Promise<ServiceSummary> {
  return invoke<ServiceSummary>("uninstall_service", { id });
}

/** Starts the installed service. */
export function startService(id: string): Promise<ServiceSummary> {
  return invoke<ServiceSummary>("start_service", { id });
}

/** Stops the running service (its runtime process tree goes first). */
export function stopService(id: string): Promise<ServiceSummary> {
  return invoke<ServiceSummary>("stop_service", { id });
}

/** Stops and then starts the installed service. */
export function restartService(id: string): Promise<ServiceSummary> {
  return invoke<ServiceSummary>("restart_service", { id });
}

/**
 * Returns the parsed log records of one service, oldest first (written by
 * the service worker under `<data>\services\logs\<id>.log`). A service that
 * never ran reads as an empty log.
 */
export function readServiceLog(id: string): Promise<ServiceLog> {
  return invoke<ServiceLog>("read_service_log", { id });
}
