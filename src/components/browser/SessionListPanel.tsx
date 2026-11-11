// Session list of the browser view: one entry per session of this run
// (launched plus discovered), newest first. Clicking an entry selects it
// for the targets pane; running launched sessions expose a Stop action.

import Badge from "../common/Badge";
import { formatRelativeTime } from "../../utils/format";
import type { BrowserSession } from "../../types/browser";
import { BROWSER_STATE_META, SESSION_ORIGIN_META } from "./browserStyle";

export interface SessionListPanelProps {
  sessions: BrowserSession[];
  selectedId: string | null;
  onSelect: (sessionId: string) => void;
  onStop: (sessionId: string) => void;
  /** Disables the Stop buttons while another action is in flight. */
  busy: boolean;
}

export default function SessionListPanel({
  sessions,
  selectedId,
  onSelect,
  onStop,
  busy,
}: SessionListPanelProps) {
  return (
    <section className="rounded-xl border border-slate-800 bg-slate-900/60">
      <header className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
        <h2 className="text-sm font-semibold text-slate-100">Sessions</h2>
        <span className="text-xs text-slate-500">{sessions.length}</span>
      </header>
      {sessions.length === 0 ? (
        <p className="px-4 py-6 text-xs text-slate-500">
          No browser sessions yet. Launch a debug-enabled Chrome above - Chromes already
          running with a debugging port are discovered here too.
        </p>
      ) : (
        <ul className="max-h-96 divide-y divide-slate-800/70 overflow-y-auto">
          {sessions.map((session) => {
            const stateMeta = BROWSER_STATE_META[session.state];
            const originMeta = SESSION_ORIGIN_META[session.origin];
            const selected = session.id === selectedId;
            const stoppable =
              session.state === "running" && session.origin === "launched" && !session.stopRequested;
            return (
              <li key={session.id} className="flex items-stretch">
                <button
                  type="button"
                  onClick={() => onSelect(session.id)}
                  aria-current={selected ? "true" : undefined}
                  className={`min-w-0 flex-1 px-4 py-3 text-left transition hover:bg-slate-800/40 ${
                    selected ? "bg-slate-800/60" : ""
                  }`}
                >
                  <span className="flex flex-wrap items-center gap-1.5">
                    <span
                      aria-hidden="true"
                      className={`h-2 w-2 shrink-0 rounded-full ${stateMeta.dot}`}
                    />
                    <span className="font-mono text-xs font-medium text-slate-100">
                      {session.id}
                    </span>
                    <Badge tone={stateMeta.tone}>{stateMeta.label}</Badge>
                    <Badge tone={originMeta.tone} title={originMeta.title}>
                      {originMeta.label}
                    </Badge>
                    {session.headless && <Badge tone="neutral">headless</Badge>}
                  </span>
                  <span className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-slate-500">
                    <span className="font-mono">port {session.port}</span>
                    <span aria-hidden="true">-</span>
                    <span>{formatRelativeTime(session.createdAt)}</span>
                    {session.chromeVersion !== null && (
                      <>
                        <span aria-hidden="true">-</span>
                        <span className="truncate">{session.chromeVersion}</span>
                      </>
                    )}
                    {session.exitCode !== null && (
                      <>
                        <span aria-hidden="true">-</span>
                        <span className="font-mono">exit {session.exitCode}</span>
                      </>
                    )}
                    {session.stopRequested && !isTerminal(session.state) && (
                      <Badge tone="warn">stopping...</Badge>
                    )}
                  </span>
                </button>
                {stoppable && (
                  <button
                    type="button"
                    onClick={() => onStop(session.id)}
                    disabled={busy}
                    className="shrink-0 border-l border-slate-800/70 px-3 text-xs text-slate-400 transition hover:bg-red-500/10 hover:text-red-300 disabled:cursor-not-allowed disabled:opacity-50"
                    title="Terminate the Chrome of this session"
                  >
                    Stop
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

/** Local helper keeping the "stopping..." badge exclusive to live sessions. */
function isTerminal(state: BrowserSession["state"]): boolean {
  return state === "stopped" || state === "failed";
}
