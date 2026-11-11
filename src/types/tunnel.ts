// TypeScript mirrors of the IPC payload types defined in
// src-tauri/src/tunnels/{mod,definition}.rs (TunnelDefinition, TunnelState,
// TunnelSummary, ...).
//
// serde(rename_all = "camelCase") on the Rust side produces camelCase keys
// on the wire. Keep these interfaces in sync when the Rust types change.
//
// Secret contract: NO type here carries API key material - the backend
// stores managed keys in <tunnels>\secrets\<id>.key and never echoes them
// back; the definition carries a reference only (see ApiKeySource).

/** Lifecycle state of one tunnel's runtime. */
export type TunnelState = "stopped" | "starting" | "running" | "failed";

/** Where the local MCP server of the tunnel lives. */
export type TunnelTargetKind = "stdio" | "http";

/** How the runtime API key reaches the generated profile. */
export type ApiKeySourceKind = "managed" | "env" | "file";

/** Control-plane endpoint region (the tunnel-client residency catalog). */
export type TunnelResidency =
  | "global"
  | "us"
  | "eu"
  | "au"
  | "ca"
  | "gb"
  | "in"
  | "jp"
  | "kr"
  | "sg"
  | "ae";

/** Reference to the runtime API key (never key material). */
export interface ApiKeySource {
  kind: ApiKeySourceKind;
  /** Environment variable name (env) or absolute file path (file);
   * null for managed (the path is derived from the definition id). */
  reference: string | null;
}

/** Contents of one tunnel definition file / IPC `TunnelDefinition`. */
export interface TunnelDefinition {
  /** Slug identifying the definition (file name + registry key). */
  id: string;
  /** Human-readable display name. */
  name: string;
  /** Free-form description (may be empty). */
  description: string;
  /** OpenAI tunnel id (`tunnel_<suffix>`) from Platform tunnel settings. */
  tunnelId: string;
  targetKind: TunnelTargetKind;
  /** Stdio target: the MCP server executable (null for HTTP targets). */
  command: string | null;
  /** Stdio target: command-line arguments appended to `command`. */
  args: string[];
  /** Stdio target: absolute working directory of the MCP server. */
  workingDir: string | null;
  /** HTTP target: streamable-HTTP URL of the MCP server. */
  serverUrl: string | null;
  /** Reference to the runtime API key (never the key itself). */
  apiKey: ApiKeySource;
  residency: TunnelResidency;
  /** Whether the runtime starts automatically with McpGo. */
  autostart: boolean;
  /** Whether the supervisor restarts the runtime after unexpected exits. */
  keepAlive: boolean;
  /** Optional absolute tunnel-client.exe path, overriding the resolution
   * order (PATH, then the managed bin directory). */
  clientPath: string | null;
  /** Pinned health-listener port; null allocates the next free port of
   * the McpGo range (18080..=18179) on every start. */
  healthPort: number | null;
  /** Creation time, epoch milliseconds. */
  createdAt: number;
  /** Last modification time, epoch milliseconds. */
  updatedAt: number;
}

/**
 * Create/update payload. Unlike TunnelDefinition it carries no id or
 * timestamps - the backend owns those.
 */
export interface TunnelDefinitionInput {
  name: string;
  description: string;
  tunnelId: string;
  targetKind: TunnelTargetKind;
  command: string | null;
  args: string[];
  workingDir: string | null;
  serverUrl: string | null;
  apiKey: ApiKeySource;
  residency: TunnelResidency;
  autostart: boolean;
  keepAlive: boolean;
  clientPath: string | null;
  healthPort: number | null;
}

/**
 * Live runtime snapshot of one tunnel, exchanged over IPC and through the
 * `tunnel://status` event.
 */
export interface TunnelStatus {
  /** Definition id this runtime belongs to. */
  definitionId: string;
  state: TunnelState;
  /** Process id of the current runtime process. */
  pid: number | null;
  /** Health-listener port of this run. */
  healthPort: number | null;
  /** `http://127.0.0.1:<port>/healthz` (the liveness endpoint). */
  healthUrl: string | null;
  /** `http://127.0.0.1:<port>/ui` (the client's operator UI). */
  uiUrl: string | null;
  /** When the current runtime was spawned, epoch milliseconds. */
  startedAt: number | null;
  /** When the health endpoint first answered, epoch milliseconds. */
  confirmedAt: number | null;
  /** Terminal transition time, epoch milliseconds. */
  stoppedAt: number | null;
  /** Exit code of the last runtime process, once it finished. */
  exitCode: number | null;
  /** Automatic restarts performed by keepAlive during this run. */
  restarts: number;
  /** Set as soon as a stop is requested. */
  stopRequested: boolean;
  /** Non-fatal infrastructure error recorded for the UI. */
  error: string | null;
}

/**
 * A tunnel definition joined with its live runtime state and the machine
 * facts the card renders - one row of the tunnels view.
 */
export interface TunnelSummary {
  definition: TunnelDefinition;
  /** Live runtime state of this application run. */
  status: TunnelStatus;
  /** Whether the tunnel-client executable resolves for this definition. */
  clientAvailable: boolean;
  /** The executable a start would spawn (override, PATH or managed bin). */
  clientPathUsed: string | null;
  /** Whether the configured API key source is ready (secret file exists /
   * variable set / key file exists). */
  apiKeyReady: boolean;
  /** Path of the managed secret file (managed sources only). */
  secretPath: string | null;
  /** Absolute path of the generated profile. */
  profilePath: string;
  /** Absolute path of the runtime log file (ndjson). */
  logPath: string;
  /** Absolute path of the definition file. */
  definitionPath: string;
}

/**
 * Static machine info rendered by the tunnels view header: where the
 * client is expected, where McpGo keeps its files, the managed health
 * port range and the OpenAI links.
 */
export interface TunnelsProfile {
  /** Resolved tunnel-client executable (default resolution), if any. */
  clientPath: string | null;
  clientAvailable: boolean;
  /** Drop-in directory for tunnel-client.exe. */
  managedBinDir: string;
  profilesDir: string;
  secretsDir: string;
  logsDir: string;
  healthPortStart: number;
  healthPortEnd: number;
  /** Platform page that manages tunnels (create/list). */
  platformTunnelsUrl: string;
  /** Public Secure MCP Tunnel guide. */
  tunnelDocsUrl: string;
  /** Repository of the supervised runtime. */
  tunnelClientRepoUrl: string;
}

/**
 * Response of the `read_tunnel_log` command: the newest ndjson lines of
 * the runtime log, oldest first. The lines stay unparsed - the client
 * owns their schema; the UI only renders them.
 */
export interface TunnelLog {
  id: string;
  lines: string[];
}

/** Payload of the `tunnel://status` event. */
export interface TunnelStatusEventPayload {
  tunnel: TunnelStatus;
}
