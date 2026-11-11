// TypeScript mirrors of the IPC payload types defined in
// src-tauri/src/browser/{mod,cdp}.rs (BrowserSession, LaunchOptions,
// BrowserProfile, BrowserTarget, event payloads).
//
// serde(rename_all = "camelCase") on the Rust side produces camelCase keys
// on the wire. Keep these interfaces in sync when the Rust types change.

/** Lifecycle state of a browser session. */
export type BrowserSessionState = "starting" | "running" | "stopped" | "failed";

/** How a session came into the registry. */
export type BrowserSessionOrigin = "launched" | "discovered";

/**
 * Launch request of the `launch_browser` command. Every field is optional:
 * a null port allocates the next free port of the McpGo range and a null
 * user data directory uses a managed profile.
 */
export interface BrowserLaunchOptions {
  /** Explicit debugging port (null = allocate from the McpGo range). */
  port: number | null;
  /** Start Chrome headless (no window, terminated on app exit). */
  headless: boolean;
  /** URL opened at launch (null = blank start page). */
  startUrl: string | null;
  /**
   * Explicit non-default user data directory (null = managed profile).
   * The default Chrome profile directory is rejected - since Chrome 136
   * --remote-debugging-port is ignored with it.
   */
  userDataDir: string | null;
  /** Extra Chrome arguments (reserved flags are dropped by the backend). */
  extraArgs: string[];
}

/** One managed browser session, exchanged over IPC. */
export interface BrowserSession {
  /** Generated id, e.g. "br_1731234567890_3". */
  id: string;
  state: BrowserSessionState;
  origin: BrowserSessionOrigin;
  /** Loopback port of the DevTools endpoint. */
  port: number;
  /** HTTP base URL of the DevTools endpoint, e.g. "http://127.0.0.1:9222". */
  debugHttp: string;
  /** Process id of the launched Chrome (null for discovered sessions). */
  pid: number | null;
  /** Product/version string once the endpoint answered. */
  chromeVersion: string | null;
  /** Whether the session runs headless (launched sessions only). */
  headless: boolean;
  /** URL the session opened at launch, when one was given. */
  startUrl: string | null;
  /** User data directory of the session (null for discovered sessions). */
  userDataDir: string | null;
  /** True as soon as a stop has been requested. */
  stopRequested: boolean;
  /** Session creation time, epoch milliseconds. */
  createdAt: number;
  /** When the DevTools endpoint first answered, epoch milliseconds. */
  confirmedAt: number | null;
  /** Terminal transition time, epoch milliseconds. */
  stoppedAt: number | null;
  /** Chrome exit code, once the process finished. */
  exitCode: number | null;
  /** Non-fatal infrastructure error recorded by the backend. */
  error: string | null;
}

/** Static machine info rendered by the browser view header. */
export interface BrowserProfile {
  /** Absolute path of the detected chrome.exe, when found. */
  chromeExe: string | null;
  /** Whether a Chrome installation was found. */
  chromeAvailable: boolean;
  /** The machine's default Chrome user data directory, when it exists. */
  defaultUserDataDir: string | null;
  /** Directory of the managed session profiles. */
  managedProfilesDir: string;
  /** First port McpGo allocates or scans. */
  portRangeStart: number;
  /** Last port McpGo allocates or scans. */
  portRangeEnd: number;
}

/** One debuggable target (tab, worker, background page). */
export interface BrowserTarget {
  /** Target id assigned by Chrome. */
  targetId: string;
  /** Target kind ("page", "service_worker", "background_page", ...). */
  kind: string;
  /** Page title (empty for non-document targets). */
  title: string;
  /** Current URL of the target. */
  url: string;
  /** CDP WebSocket URL of the target; absent for some target kinds. */
  webSocketDebuggerUrl: string | null;
}

/** Response of the `read_browser_targets` command. */
export interface BrowserTargetList {
  sessionId: string;
  /** Version string of the moment of the read. */
  chromeVersion: string;
  /** Live targets, pages first. */
  targets: BrowserTarget[];
}

/** Payload of the `browser://status` event. */
export interface BrowserStatusEventPayload {
  session: BrowserSession;
}
