// Job list of the terminal view: one entry per known job (current and
// previous sessions), newest first. Clicking an entry selects it for the
// output pane; running jobs expose a Cancel action.

import Badge from "../common/Badge";
import { formatRelativeTime } from "../../utils/format";
import type { JobStatus } from "../../types/job";
import { JOB_STATE_META } from "./stateStyle";

export interface JobListPanelProps {
  jobs: JobStatus[];
  selectedId: string | null;
  onSelect: (jobId: string) => void;
  onCancel: (jobId: string) => void;
}

export default function JobListPanel({
  jobs,
  selectedId,
  onSelect,
  onCancel,
}: JobListPanelProps) {
  return (
    <section className="rounded-xl border border-slate-800 bg-slate-900/60">
      <header className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
        <h2 className="text-sm font-semibold text-slate-100">Jobs</h2>
        <span className="text-xs text-slate-500">{jobs.length}</span>
      </header>
      {jobs.length === 0 ? (
        <p className="px-4 py-6 text-xs text-slate-500">
          No jobs yet. Run a command above - finished jobs from previous sessions are listed here
          too.
        </p>
      ) : (
        <ul className="max-h-96 divide-y divide-slate-800/70 overflow-y-auto">
          {jobs.map((job) => {
            const meta = JOB_STATE_META[job.state];
            const selected = job.id === selectedId;
            return (
              <li key={job.id} className="flex items-stretch">
                <button
                  type="button"
                  onClick={() => onSelect(job.id)}
                  aria-current={selected ? "true" : undefined}
                  className={`min-w-0 flex-1 px-4 py-3 text-left transition hover:bg-slate-800/40 ${
                    selected ? "bg-slate-800/60" : ""
                  }`}
                >
                  <span className="flex items-center gap-2">
                    <span
                      aria-hidden="true"
                      className={`h-2 w-2 shrink-0 rounded-full ${meta.dot}`}
                    />
                    <span className="truncate text-xs font-medium text-slate-100">
                      {job.title}
                    </span>
                  </span>
                  <span className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-slate-500">
                    <span className="font-mono">{job.shell}</span>
                    <span aria-hidden="true">-</span>
                    <span>{formatRelativeTime(job.createdAt)}</span>
                    {job.state === "running" && job.cancelRequested && (
                      <Badge tone="warn">canceling...</Badge>
                    )}
                    {job.exitCode !== null && (
                      <span className="font-mono">exit {job.exitCode}</span>
                    )}
                  </span>
                </button>
                {job.state === "running" && (
                  <button
                    type="button"
                    onClick={() => onCancel(job.id)}
                    className="shrink-0 border-l border-slate-800/70 px-3 text-xs text-slate-400 transition hover:bg-red-500/10 hover:text-red-300"
                    title="Request cancellation"
                  >
                    Cancel
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
