// Log panel of one tunnel (mounted by TunnelCard while its panel is
// open): renders the raw ndjson lines of the runtime log, auto-scrolls to
// the newest line and shows the log file path in the footer.
//
// Unlike the service log panel (which renders the parsed canonical
// grammar), these lines stay unparsed: tunnel-client owns the schema.
// Presentation still mirrors the service log panel's shape; data flows
// through useTunnelLog, which polls the file while this panel is mounted.

import { useEffect, useRef } from "react";
import Badge from "../common/Badge";
import type { TunnelSummary } from "../../types/tunnel";
import { tunnelStateMeta } from "./tunnelStyle";
import { useTunnelLog } from "../../hooks/useTunnelLog";

export interface TunnelLogPanelProps {
  /** The tunnel whose log is shown (definition + runtime state). */
  summary: TunnelSummary;
  /** Closes the panel (owned by TunnelsView). */
  onClose: () => void;
}

export default function TunnelLogPanel({ summary, onClose }: TunnelLogPanelProps) {
  const { lines, loading } = useTunnelLog(summary.definition.id);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Stick to the bottom while output is streaming.
  useEffect(() => {
    const element = scrollRef.current;
    if (element !== null) element.scrollTop = element.scrollHeight;
  }, [lines]);

  const meta = tunnelStateMeta(summary.status.state);

  return (
    <section
      aria-label={`Runtime log of ${summary.definition.name}`}
      className="flex max-h-80 flex-col overflow-hidden rounded-lg border border-slate-800 bg-slate-950/60"
    >
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800 px-4 py-2">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span aria-hidden="true" className={`h-2 w-2 shrink-0 rounded-full ${meta.dot}`} />
          <span className="truncate font-mono text-xs font-semibold text-slate-100">
            {summary.definition.id}
          </span>
          <Badge tone={meta.tone}>{meta.label}</Badge>
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
          <p className="font-mono text-xs text-slate-500">Reading runtime log...</p>
        )}
        {lines.length === 0 && !loading && (
          <p className="font-mono text-xs text-slate-500">
            No log records yet. The runtime writes here once it starts (ndjson, one
            record per line).
          </p>
        )}
        {lines.map((line, index) => (
          <div key={index} className="font-mono text-xs leading-5 text-slate-300">
            <span className="whitespace-pre-wrap break-all">
              {line.length > 0 ? line : " "}
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
