// Output pane of the selected job: renders the parsed job_logs records
// (timestamps dimmed, stdout light, stderr red, system records muted),
// auto-scrolls to the newest line and exposes cancellation while running.

import { useEffect, useRef } from "react";
import Badge from "../common/Badge";
import type { JobLogLine, JobStatus } from "../../types/job";
import { JOB_STATE_META } from "./stateStyle";

export interface JobOutputPaneProps {
  job: JobStatus | null;
  lines: JobLogLine[];
  loading: boolean;
  onCancel: (jobId: string) => void;
}

/** Tailwind text color per output stream. */
function lineClass(stream: JobLogLine["stream"]): string {
  switch (stream) {
    case "err":
      return "text-red-300";
    case "sys":
      return "text-slate-500";
    default:
      return "text-slate-200";
  }
}

export default function JobOutputPane({ job, lines, loading, onCancel }: JobOutputPaneProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Stick to the bottom while output is streaming.
  useEffect(() => {
    const element = scrollRef.current;
    if (element !== null) element.scrollTop = element.scrollHeight;
  }, [lines]);

  if (job === null) {
    return (
      <section className="flex min-h-72 items-center justify-center rounded-xl border border-slate-800 bg-slate-900/60 px-6 text-sm text-slate-500">
        Select a job to see its output.
      </section>
    );
  }

  const meta = JOB_STATE_META[job.state];

  return (
    <section className="flex min-h-72 flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-900/60">
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800 px-4 py-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span aria-hidden="true" className={`h-2 w-2 shrink-0 rounded-full ${meta.dot}`} />
          <span className="truncate text-xs font-semibold text-slate-100">{job.title}</span>
          <Badge tone={meta.tone}>{meta.label}</Badge>
          {job.exitCode !== null && (
            <span className="font-mono text-[11px] text-slate-500">exit {job.exitCode}</span>
          )}
        </div>
        <div className="flex items-center gap-2">
          {job.state === "running" && (
            <button
              type="button"
              onClick={() => onCancel(job.id)}
              className="rounded-lg border border-slate-700 px-3 py-1 text-xs text-slate-300 transition hover:border-red-500/40 hover:bg-red-500/10 hover:text-red-300"
            >
              Cancel
            </button>
          )}
        </div>
      </header>

      {job.error !== null && (
        <p className="border-b border-red-500/20 bg-red-500/10 px-4 py-2 text-xs text-red-300">
          {job.error}
        </p>
      )}

      <div ref={scrollRef} className="min-h-56 flex-1 overflow-y-auto bg-slate-950/50 px-4 py-3">
        {lines.length === 0 && loading && (
          <p className="font-mono text-xs text-slate-500">Reading job log...</p>
        )}
        {lines.length === 0 && !loading && (
          <p className="font-mono text-xs text-slate-500">
            No output recorded for this job yet.
          </p>
        )}
        {lines.map((line, index) => (
          <div key={index} className={`font-mono text-xs leading-5 ${lineClass(line.stream)}`}>
            {line.time !== null && (
              <span className="mr-2 select-none text-slate-600">{line.time}</span>
            )}
            <span className="whitespace-pre-wrap break-all">{line.text.length > 0 ? line.text : " "}</span>
          </div>
        ))}
      </div>

      <footer className="border-t border-slate-800 px-4 py-2">
        <p className="truncate text-[11px] text-slate-500" title={job.logFile}>
          Log file: {job.logFile}
        </p>
      </footer>
    </section>
  );
}
