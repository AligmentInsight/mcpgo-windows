// IPC wrappers for the tunnel commands implemented in
// src-tauri/src/commands/tunnels.rs and for the `tunnel://status` event
// emitted by the tunnel registry (src-tauri/src/tunnels/mod.rs).
//
// Event names MUST stay in sync with the EVENT_TUNNEL_STATUS constant
// there.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  TunnelDefinition,
  TunnelDefinitionInput,
  TunnelLog,
  TunnelStatusEventPayload,
  TunnelSummary,
  TunnelsProfile,
} from "../types/tunnel";

/**
 * Returns the machine profile of the tunnels view header (client path and
 * managed directories, health port range, OpenAI links).
 */
export function fetchTunnelsProfile(): Promise<TunnelsProfile> {
  return invoke<TunnelsProfile>("tunnels_profile");
}

/**
 * Lists every tunnel definition joined with its live runtime state and the
 * machine facts of the card, sorted by display name.
 */
export function listTunnels(): Promise<TunnelSummary[]> {
  return invoke<TunnelSummary[]>("list_tunnels");
}

/** Loads one tunnel definition by id (never contains key material). */
export function getTunnelDefinition(id: string): Promise<TunnelDefinition> {
  return invoke<TunnelDefinition>("get_tunnel_definition", { id });
}

/**
 * Creates a definition. `id` is optional: when omitted the backend derives
 * it from the slugified name.
 */
export function createTunnelDefinition(
  input: TunnelDefinitionInput,
  id?: string,
): Promise<TunnelDefinition> {
  return invoke<TunnelDefinition>("create_tunnel_definition", {
    input,
    id: id ?? null,
  });
}

/**
 * Updates an existing definition. Refuses while the runtime is active
 * (stop it first - the running process keeps the previous profile).
 */
export function updateTunnelDefinition(
  id: string,
  input: TunnelDefinitionInput,
): Promise<TunnelDefinition> {
  return invoke<TunnelDefinition>("update_tunnel_definition", { id, input });
}

/**
 * Deletes one definition. Refuses while the runtime is active; removes the
 * definition file, the generated profile and the managed secret (the log
 * file stays behind as evidence).
 */
export function deleteTunnelDefinition(id: string): Promise<void> {
  return invoke<void>("delete_tunnel_definition", { id });
}

/**
 * Stores the managed API key of definition `id` in
 * `<tunnels>\secrets\<id>.key`. The key is written, never logged and never
 * returned; refuses for non-managed key sources and while the runtime is
 * active.
 */
export function setTunnelApiKey(id: string, key: string): Promise<void> {
  return invoke<void>("set_tunnel_api_key", { id, key });
}

/**
 * Starts the runtime of definition `id`: the backend generates the
 * profile, spawns `tunnel-client run --profile-file ...` and supervises
 * it. Resolves with the initial "starting" summary; the "running" (or
 * "failed") status arrives through tunnel://status events.
 */
export function startTunnel(id: string): Promise<TunnelSummary> {
  return invoke<TunnelSummary>("start_tunnel", { id });
}

/**
 * Requests the stop of the running runtime. Resolves with the current
 * snapshot (stopRequested: true); the final "stopped" status arrives
 * through a tunnel://status event. Idempotent.
 */
export function stopTunnel(id: string): Promise<TunnelSummary> {
  return invoke<TunnelSummary>("stop_tunnel", { id });
}

/**
 * Returns the newest ndjson lines of the runtime log of definition `id`.
 * A tunnel that never ran reads as an empty log.
 */
export function readTunnelLog(id: string): Promise<TunnelLog> {
  return invoke<TunnelLog>("read_tunnel_log", { id });
}

/** Subscribes to `tunnel://status` (every runtime state transition). */
export function onTunnelStatusEvent(
  handler: (status: TunnelStatusEventPayload["tunnel"]) => void,
): Promise<() => void> {
  return listen<TunnelStatusEventPayload>("tunnel://status", (event) => {
    handler(event.payload.tunnel);
  });
}
