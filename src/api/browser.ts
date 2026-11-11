// IPC wrappers for the browser commands implemented in
// src-tauri/src/commands/browser.rs and for the `browser://status` event
// emitted by the session registry (src-tauri/src/browser/mod.rs).
//
// Event names MUST stay in sync with the EVENT_BROWSER_STATUS constant
// there.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  BrowserLaunchOptions,
  BrowserProfile,
  BrowserSession,
  BrowserStatusEventPayload,
  BrowserTargetList,
} from "../types/browser";

/**
 * Returns the machine's browser profile information (Chrome path, default
 * and managed profile directories, port range).
 */
export function fetchBrowserProfile(): Promise<BrowserProfile> {
  return invoke<BrowserProfile>("browser_profile");
}

/**
 * Lists the sessions of this application run, newest first: McpGo-launched
 * Chromes plus the DevTools endpoints discovered at startup.
 */
export function listBrowserSessions(): Promise<BrowserSession[]> {
  return invoke<BrowserSession[]>("list_browser_sessions");
}

/** Returns the current snapshot of one session. */
export function getBrowserSession(sessionId: string): Promise<BrowserSession> {
  return invoke<BrowserSession>("get_browser_session", { sessionId });
}

/**
 * Launches one debug-enabled Chrome session; resolves with its initial
 * "starting" status. The "running" (or "failed") status arrives through a
 * browser://status event.
 */
export function launchBrowser(
  options: BrowserLaunchOptions,
): Promise<BrowserSession> {
  return invoke<BrowserSession>("launch_browser", { options });
}

/**
 * Requests the stop of one launched session. Resolves with the current
 * snapshot (still "running", stopRequested: true); the final "stopped"
 * status arrives through a browser://status event. Discovered sessions
 * cannot be stopped here.
 */
export function stopBrowserSession(sessionId: string): Promise<BrowserSession> {
  return invoke<BrowserSession>("stop_browser_session", { sessionId });
}

/**
 * Reads the live DevTools targets of one RUNNING session (tabs, workers,
 * background pages) joined with the endpoint's version string. The targets
 * panel polls this while it is open.
 */
export function readBrowserTargets(sessionId: string): Promise<BrowserTargetList> {
  return invoke<BrowserTargetList>("read_browser_targets", { sessionId });
}

/** Subscribes to `browser://status` (every session state transition). */
export function onBrowserStatusEvent(
  handler: (session: BrowserSession) => void,
): Promise<() => void> {
  return listen<BrowserStatusEventPayload>("browser://status", (event) => {
    handler(event.payload.session);
  });
}
