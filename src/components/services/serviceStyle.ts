// Shared presentation metadata for the services view: runtime chips and
// SCM status badges (used by ServiceCard and ServicesView).

import type { BadgeTone } from "../common/Badge";
import type { RuntimeKind } from "../../types/service";

export interface RuntimeMeta {
  /** Human-readable runtime label. */
  label: string;
  /** Tailwind classes of the runtime chip. */
  chipClass: string;
  /** Hint shown under the command field in the definition form. */
  commandHint: string;
  /** Input hint of the runtime executable field in the definition form. */
  runtimePathHint: string;
}

export const RUNTIME_META: Record<RuntimeKind, RuntimeMeta> = {
  node: {
    label: "Node.js",
    chipClass: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/30",
    commandHint: "entry script, e.g. C:\\servers\\mcp-server.js",
    runtimePathHint: "C:\\Program Files\\nodejs\\node.exe",
  },
  python: {
    label: "Python",
    chipClass: "bg-cyan-500/15 text-cyan-300 ring-cyan-500/30",
    commandHint: "entry script, e.g. C:\\servers\\server.py",
    runtimePathHint: "C:\\Python312\\python.exe",
  },
  powershell: {
    label: "PowerShell",
    chipClass: "bg-sky-500/15 text-sky-300 ring-sky-500/30",
    commandHint: "script path or PowerShell command",
    runtimePathHint: "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
  },
};

export interface ScmStatusMeta {
  /** Human-readable status label. */
  label: string;
  /** Badge tone of the status badge. */
  tone: BadgeTone;
  /** Tailwind class of the status dot. */
  dot: string;
}

/**
 * Presentation metadata for the SCM state of one service. The status
 * strings are the culture-invariant ServiceControllerStatus enum names
 * produced by the backend (see src-tauri/src/services/scm.rs).
 */
export function scmStatusMeta(installed: boolean, status: string | null): ScmStatusMeta {
  if (!installed) {
    return { label: "Not installed", tone: "neutral", dot: "bg-slate-500" };
  }
  switch (status) {
    case "Running":
      return { label: "Running", tone: "ok", dot: "bg-emerald-400" };
    case "StartPending":
    case "ContinuePending":
    case "PausePending":
    case "StopPending":
      return { label: status, tone: "warn", dot: "bg-amber-400 animate-pulse" };
    case "Paused":
      return { label: "Paused", tone: "warn", dot: "bg-amber-400" };
    case "Stopped":
      return { label: "Stopped", tone: "neutral", dot: "bg-slate-400" };
    default:
      return { label: status ?? "Unknown", tone: "error", dot: "bg-red-400" };
  }
}
