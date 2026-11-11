// Targets pane of the selected session: the live DevTools target list
// (tabs, workers, background pages) plus the endpoint banner with the
// Chrome version and the debug HTTP base URL MCP servers attach to.
//
// The list is polled by the useBrowserTargets hook while a session is
// selected (the DevTools endpoint has no push channel McpGo could use).

import Badge from "../common/Badge";
import { useBrowserTargets } from "../../hooks/useBrowserTargets";
import type { BrowserSession } from "../../types/browser";
import { BROWSER_STATE_META, SESSION_ORIGIN_META } from "./browserStyle";

export interface TargetsPaneProps {
  session: BrowserSession | null;
}

export default function TargetsPane({ session }: TargetsPaneProps) {
  const { targets, chromeVersion, loading, error } = useBrowserTargets(
    session !== null ? session.id : null,
  );

  if (session === null) {
    return (
      <section className="flex min-h-72 items-center justify-center rounded-xl border border-slate-800 bg-slate-900/60 px-6 text-sm text-slate-500">
        Select a session to see its DevTools targets.
      </section>
    );
  }

  const stateMeta = BROWSER_STATE_META[session.state];
  const originMeta = SESSION_ORIGIN_META[session.origin];
  const live = session.state === "running";

  return (
    <section className="flex min-h-72 flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-900/60">
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800 px-4 py-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span aria-hidden="true" className={`h-2 w-2 shrink-0 rounded-full ${stateMeta.dot}`} />
          <span className="font-mono text-xs font-semibold text-slate-100">{session.id}</span>
          <Badge tone={stateMeta.tone}>{stateMeta.label}</Badge>
          <Badge tone={originMeta.tone} title={originMeta.title}>
            {originMeta.label}
          </Badge>
          {(chromeVersion ?? session.chromeVersion) !== null && (
            <span className="font-mono text-[11px] text-slate-500">
              {chromeVersion ?? session.chromeVersion}
            </span>
          )}
        </div>
        <span className="font-mono text-[11px] text-slate-500">
          {session.debugHttp} ({session.port})
        </span>
      </header>

      {session.error !== null && (
        <p className="border-b border-red-500/20 bg-red-500/10 px-4 py-2 text-xs text-red-300">
          {session.error}
        </p>
      )}
      {error !== null && session.error === null && (
        <p className="border-b border-amber-500/20 bg-amber-500/10 px-4 py-2 text-xs text-amber-300">
          {error}
        </p>
      )}

      <div className="min-h-56 flex-1 overflow-y-auto bg-slate-950/50 px-4 py-3">
        {!live ? (
          <p className="font-mono text-xs text-slate-500">
            This session is {session.state}; target lists are read while a session runs.
          </p>
        ) : targets.length === 0 && loading ? (
          <p className="font-mono text-xs text-slate-500">Reading DevTools targets...</p>
        ) : targets.length === 0 ? (
          <p className="font-mono text-xs text-slate-500">
            No debuggable targets reported by this Chrome yet.
          </p>
        ) : (
          <ul className="space-y-1.5">
            {targets.map((target) => (
              <li
                key={target.targetId}
                className="rounded-lg border border-slate-800/70 bg-slate-900/70 px-3 py-2"
              >
                <div className="flex flex-wrap items-center gap-1.5">
                  <Badge tone={target.kind === "page" || target.kind === "tab" ? "ok" : "neutral"}>
                    {target.kind}
                  </Badge>
                  <span className="min-w-0 truncate text-xs font-medium text-slate-100">
                    {target.title.length > 0 ? target.title : "(untitled)"}
                  </span>
                </div>
                <p
                  className="mt-1 truncate font-mono text-[11px] text-slate-500"
                  title={target.url}
                >
                  {target.url}
                </p>
                {target.webSocketDebuggerUrl !== null && (
                  <p
                    className="mt-0.5 truncate font-mono text-[11px] text-slate-600"
                    title={target.webSocketDebuggerUrl}
                  >
                    {target.webSocketDebuggerUrl}
                  </p>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>

      <footer className="border-t border-slate-800 px-4 py-2">
        <p className="truncate text-[11px] text-slate-500" title={session.userDataDir ?? undefined}>
          DevTools HTTP: {session.debugHttp}/json/version - profile:{" "}
          {session.userDataDir ?? "(owned by the discovered Chrome)"}
        </p>
      </footer>
    </section>
  );
}
