// Log panel of one service (mounted by ServiceCard while its panel is
// open): renders the parsed service log records - runtime output, runtime
// errors and the worker's own system records - auto-scrolls to the newest
// line and shows the log file path in the footer.
//
// Presentation mirrors the terminal's JobOutputPane (same canonical log
// grammar, same colour language); data flows through useServiceLog, which
// polls the file while this panel is mounted.

import { useEffect, useRef } from "react";
import Badge from "../common/Badge";
import type { JobLogLine } from "../../types/job";
import type { ServiceSummary } from "../../types/service";
import { scmStatusMeta } from "./serviceStyle";
import { useServiceLog } from "../../hooks/useServiceLog";

export interface ServiceLogPanelProps {
  /** The service whose log is shown (definition + SCM state). */
  summary: ServiceSummary;
  /** Closes the panel (owned by ServicesView). */
  onClose: () => void;
}

/** Tailwind text colour per output stream (same language as the terminal). */
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

export default function ServiceLogPanel({ summary, onClose }: ServiceLogPanelProps) {
  const { lines, loading } = useServiceLog(summary.definition.id);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Stick to the bottom while output is streaming.
  useEffect(() => {
    const element = scrollRef.current;
    if (element !== null) element.scrollTop = element.scrollHeight;
  }, [lines]);

  const scm = scmStatusMeta(summary.installed, summary.scmStatus);

  return (
    <section
      aria-label={`Service log of ${summary.definition.name}`}
      className="flex max-h-80 flex-col overflow-hidden rounded-lg border border-slate-800 bg-slate-950/60"
    >
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800 px-4 py-2">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span aria-hidden="true" className={`h-2 w-2 shrink-0 rounded-full ${scm.dot}`} />
          <span className="truncate font-mono text-xs font-semibold text-slate-100">
            {summary.serviceName}
          </span>
          <Badge tone={scm.tone}>{scm.label}</Badge>
        </div>
        <button
          type="button"
          onClick={onClose}
          title="Close the log panel"
          className="rounded-lg border border-slate-700 px-3 py-1 text-xs text-slate-300 transition hover:border-slate-500 hover:text-slate-100"
        >
          Close
        </button>
      </header>

      <div ref={scrollRef} className="min-h-40 flex-1 overflow-y-auto px-4 py-3">
        {lines.length === 0 && loading && (
          <p className="font-mono text-xs text-slate-500">Reading service log...</p>
        )}
        {lines.length === 0 && !loading && (
          <p className="font-mono text-xs text-slate-500">
            No log records yet. The service worker writes here once the service runs.
          </p>
        )}
        {lines.map((line, index) => (
          <div key={index} className={`font-mono text-xs leading-5 ${lineClass(line.stream)}`}>
            {line.time !== null && (
              <span className="mr-2 select-none text-slate-600">{line.time}</span>
            )}
            <span className="whitespace-pre-wrap break-all">
              {line.text.length > 0 ? line.text : " "}
            </span>
          </div>
        ))}
      </div>

      <footer className="border-t border-slate-800 px-4 py-2">
        <p className="truncate text-[11px] text-slate-500" title={summary.logPath}>
          Log file: {summary.logPath} (refreshed every 2 s)
        </p>
      </footer>
    </section>
  );
}
