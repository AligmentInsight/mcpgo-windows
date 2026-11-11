// Shared presentation metadata for browser session states and origins,
// used by the session list and the targets pane (status dot color, badge
// tone, label).

import type { BadgeTone } from "../common/Badge";
import type { BrowserSessionOrigin, BrowserSessionState } from "../../types/browser";

export interface BrowserStateMeta {
  /** Tailwind class of the status dot. */
  dot: string;
  /** Badge tone for the state badge. */
  tone: BadgeTone;
  /** Human-readable state label. */
  label: string;
}

export const BROWSER_STATE_META: Record<BrowserSessionState, BrowserStateMeta> = {
  starting: { dot: "bg-amber-400 animate-pulse", tone: "warn", label: "Starting" },
  running: { dot: "bg-sky-400 animate-pulse", tone: "neutral", label: "Running" },
  stopped: { dot: "bg-slate-500", tone: "neutral", label: "Stopped" },
  failed: { dot: "bg-red-400", tone: "error", label: "Failed" },
};

export interface SessionOriginMeta {
  /** Badge tone for the origin badge. */
  tone: BadgeTone;
  /** Human-readable origin label. */
  label: string;
  /** Tooltip explaining what the origin means. */
  title: string;
}

export const SESSION_ORIGIN_META: Record<BrowserSessionOrigin, SessionOriginMeta> = {
  launched: {
    tone: "neutral",
    label: "Launched",
    title: "Chrome started by McpGo; stopping it terminates the process.",
  },
  discovered: {
    tone: "ok",
    label: "Discovered",
    title:
      "A Chrome that was already running with --remote-debugging-port; " +
      "McpGo does not own the process and cannot stop it.",
  },
};
