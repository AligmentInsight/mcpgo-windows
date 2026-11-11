// Shared presentation metadata for the tunnels view: target chips, runtime
// state badges, key-source labels and the residency options (used by
// TunnelCard, TunnelDefinitionForm and TunnelsView).

import type { BadgeTone } from "../common/Badge";
import type {
  ApiKeySourceKind,
  TunnelResidency,
  TunnelState,
  TunnelTargetKind,
} from "../../types/tunnel";

export interface TargetMeta {
  /** Human-readable target label. */
  label: string;
  /** Tailwind classes of the target chip. */
  chipClass: string;
  /** Hint shown under the command field in the definition form. */
  commandHint: string;
}

export const TARGET_META: Record<TunnelTargetKind, TargetMeta> = {
  stdio: {
    label: "stdio",
    chipClass: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/30",
    commandHint: "full MCP server command line, e.g. node C:\\servers\\mcp-server.js",
  },
  http: {
    label: "HTTP",
    chipClass: "bg-cyan-500/15 text-cyan-300 ring-cyan-500/30",
    commandHint: "streamable-HTTP URL, e.g. http://127.0.0.1:3000/mcp",
  },
};

export interface TunnelStateMeta {
  /** Human-readable status label. */
  label: string;
  /** Badge tone of the status badge. */
  tone: BadgeTone;
  /** Tailwind class of the status dot. */
  dot: string;
}

/**
 * Presentation metadata for the runtime state of one tunnel (the states
 * are produced by the backend - see src-tauri/src/tunnels/mod.rs).
 */
export function tunnelStateMeta(state: TunnelState): TunnelStateMeta {
  switch (state) {
    case "running":
      return { label: "Running", tone: "ok", dot: "bg-emerald-400" };
    case "starting":
      return { label: "Starting", tone: "warn", dot: "bg-amber-400 animate-pulse" };
    case "failed":
      return { label: "Failed", tone: "error", dot: "bg-red-400" };
    default:
      return { label: "Stopped", tone: "neutral", dot: "bg-slate-500" };
  }
}

export interface ApiKeySourceMeta {
  /** Human-readable key-source label. */
  label: string;
  /** Hint shown under the key source field in the definition form. */
  hint: string;
}

export const API_KEY_SOURCE_META: Record<ApiKeySourceKind, ApiKeySourceMeta> = {
  managed: {
    label: "Managed key file",
    hint: "McpGo stores the key in its secrets directory; set it with the Set key action on the card.",
  },
  env: {
    label: "Environment variable",
    hint: "The runtime reads the key from this variable of the McpGo process (set it before launching McpGo).",
  },
  file: {
    label: "Key file",
    hint: "The runtime reads the key from this file (the pattern recommended by the tunnel-client docs).",
  },
};

/** Residency options of the definition form, global first. */
export const RESIDENCY_OPTIONS: Array<{ value: TunnelResidency; label: string }> = [
  { value: "global", label: "global (api.openai.com)" },
  { value: "us", label: "us (us.api.openai.com)" },
  { value: "eu", label: "eu (eu.api.openai.com)" },
  { value: "au", label: "au (au.api.openai.com)" },
  { value: "ca", label: "ca (ca.api.openai.com)" },
  { value: "gb", label: "gb (gb.api.openai.com)" },
  { value: "in", label: "in (in.api.openai.com)" },
  { value: "jp", label: "jp (jp.api.openai.com)" },
  { value: "kr", label: "kr (kr.api.openai.com)" },
  { value: "sg", label: "sg (sg.api.openai.com)" },
  { value: "ae", label: "ae (ae.api.openai.com)" },
];
