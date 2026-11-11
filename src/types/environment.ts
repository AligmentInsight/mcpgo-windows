// TypeScript mirrors of the IPC payload types defined in
// src-tauri/src/main.rs (ComponentCheck, EnvironmentReport).
//
// serde(rename_all = "camelCase") on the Rust side produces camelCase keys
// on the wire. Keep these interfaces in sync when the Rust types change.

/** Result of a single Windows component check. */
export interface ComponentCheck {
  /** Short machine-readable id, e.g. "powershell-5.1". */
  id: string;
  /** Human-readable label rendered in the UI. */
  label: string;
  /** True when the component was found on this machine. */
  available: boolean;
  /** Absolute path of the component, or null when not found. */
  path: string | null;
  /** Explanatory note shown under the label. */
  note: string;
  /** Whether McpGo refuses to operate without this component. */
  required: boolean;
}

/** Full environment snapshot cached by the backend at startup. */
export interface EnvironmentReport {
  powershell51: ComponentCheck;
  powershell7: ComponentCheck;
  chromeExe: ComponentCheck;
  chromeUserData: ComponentCheck;
  nodeJs: ComponentCheck;
  python: ComponentCheck;
  /** Directory for job_status files (one JSON per background job). */
  jobsStatusDir: string;
  /** Directory for job_logs files (one log file per background job). */
  jobsLogsDir: string;
  /** Directory with service definitions and generated PowerShell scripts. */
  servicesDir: string;
}
