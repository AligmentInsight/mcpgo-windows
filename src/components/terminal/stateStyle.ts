// Shared presentation metadata for job states, used by the job list and
// the output pane (status dot color, badge tone, label).

import type { BadgeTone } from "../common/Badge";
import type { JobState } from "../../types/job";

export interface JobStateMeta {
  /** Tailwind class of the status dot. */
  dot: string;
  /** Badge tone for the state badge. */
  tone: BadgeTone;
  /** Human-readable state label. */
  label: string;
}

export const JOB_STATE_META: Record<JobState, JobStateMeta> = {
  running: { dot: "bg-sky-400 animate-pulse", tone: "neutral", label: "Running" },
  completed: { dot: "bg-emerald-400", tone: "ok", label: "Completed" },
  failed: { dot: "bg-red-400", tone: "error", label: "Failed" },
  canceled: { dot: "bg-slate-500", tone: "neutral", label: "Canceled" },
};
