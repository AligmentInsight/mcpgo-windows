// TypeScript mirrors of the IPC payload types defined in
// src-tauri/src/services/{mod,definition,scm}.rs (ServiceDefinition,
// RuntimeKind, ServiceSummary, ...).
//
// serde(rename_all = "camelCase") on the Rust side produces camelCase keys
// on the wire. Keep these interfaces in sync when the Rust types change.

import type { JobLogLine } from "./job";

/** Runtime that executes a service command. */
export type RuntimeKind = "node" | "python" | "powershell";

/** Contents of one definition file / IPC `ServiceDefinition`. */
export interface ServiceDefinition {
  /** Slug identifying the definition (SCM name: `McpGo-<id>`). */
  id: string;
  /** Human-readable display name. */
  name: string;
  /** Free-form description (may be empty). */
  description: string;
  runtime: RuntimeKind;
  /** Entry command: a script path (node/python) or a PowerShell command. */
  command: string;
  /** Command-line arguments appended after the command. */
  args: string[];
  /** Absolute working directory, when one is set. */
  workingDir: string | null;
  /** Extra environment variables for the runtime process. */
  env: Record<string, string>;
  /** Whether the service starts automatically at system boot. */
  autostart: boolean;
  /** Restart the runtime process automatically when it fails (the
   * LaunchAgent `KeepAlive` flag). */
  keepAlive: boolean;
  /** Optional absolute runtime executable path, overriding the PATH
   * lookup (recommended for per-user runtime installs: services run as
   * LocalSystem, whose PATH differs from the installing user's). */
  runtimePath: string | null;
  /** Creation time, epoch milliseconds. */
  createdAt: number;
  /** Last modification time, epoch milliseconds. */
  updatedAt: number;
}

/**
 * Create/update payload. Unlike ServiceDefinition it carries no id or
 * timestamps - the backend owns those.
 */
export interface ServiceDefinitionInput {
  name: string;
  description: string;
  runtime: RuntimeKind;
  command: string;
  args: string[];
  workingDir: string | null;
  env: Record<string, string>;
  autostart: boolean;
  keepAlive: boolean;
  runtimePath: string | null;
}

/**
 * A definition joined with its live SCM state - one row of the services
 * view.
 */
export interface ServiceSummary {
  definition: ServiceDefinition;
  /** SCM name of the service (`McpGo-<id>`). */
  serviceName: string;
  /** Whether the SCM knows a service under that name. */
  installed: boolean;
  /** ServiceControllerStatus name when installed ("Running", ...). */
  scmStatus: string | null;
  /** ServiceStartMode name when installed ("Automatic", ...). */
  startType: string | null;
  /** Whether the definition's runtime is resolvable on this machine. */
  runtimeAvailable: boolean;
  /** Absolute path of the definition file. */
  definitionPath: string;
  /** Absolute path of the log file the service worker writes
   * (`<data>\services\logs\<id>.log`); shown by the log panel footer. */
  logPath: string;
}

/**
 * Response of the `read_service_log` command. The lines reuse the job log
 * record type: the service worker writes the same canonical
 * `[RFC3339] OUT|ERR|SYS <text>` grammar as the job logs.
 */
export interface ServiceLog {
  id: string;
  lines: JobLogLine[];
}
