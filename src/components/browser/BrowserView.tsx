// Browser view - the Phase 5 face of the Chrome/CDP domain (the port of
// the macOS Playwright browser management). Composition only: data flows
// through useBrowserSessions, the live targets live in useBrowserTargets,
// presentation lives in LaunchPanel, SessionListPanel and TargetsPane.

import LaunchPanel from "./LaunchPanel";
import SessionListPanel from "./SessionListPanel";
import TargetsPane from "./TargetsPane";
import Badge from "../common/Badge";
import { useBrowserSessions } from "../../hooks/useBrowserSessions";

export default function BrowserView() {
  const {
    sessions,
    profile,
    loading,
    loadError,
    selectedId,
    select,
    launch,
    stop,
    acting,
    actionError,
  } = useBrowserSessions();
  const selectedSession = sessions.find((session) => session.id === selectedId) ?? null;

  return (
    <div className="space-y-4">
      <header>
        <h1 className="text-lg font-semibold text-slate-100">Browser</h1>
        <p className="text-xs text-slate-400">
          Launch debug-enabled Chrome sessions with a managed, non-default profile (the
          Chrome 136 requirement) or attach to a running DevTools endpoint discovered in the
          McpGo port range. Targets are read-only; MCP servers and Playwright attach to the
          loopback endpoint themselves.
        </p>
      </header>

      <div className="flex flex-wrap items-center gap-2 rounded-xl border border-slate-800 bg-slate-900/60 px-4 py-3 text-xs text-slate-400">
        <span className="font-medium text-slate-300">Chrome:</span>
        {profile === null ? (
          <span className="text-slate-500">loading machine profile...</span>
        ) : profile.chromeAvailable ? (
          <>
            <Badge tone="ok">found</Badge>
            <span className="min-w-0 truncate font-mono" title={profile.chromeExe ?? undefined}>
              {profile.chromeExe}
            </span>
          </>
        ) : (
          <>
            <Badge tone="error">not found</Badge>
            <span>
              Install Google Chrome in a standard location, then reload the view.
            </span>
          </>
        )}
        {profile !== null && profile.defaultUserDataDir !== null && (
          <span
            className="min-w-0 truncate font-mono text-slate-500"
            title={profile.defaultUserDataDir}
          >
            default profile: {profile.defaultUserDataDir}
          </span>
        )}
      </div>

      {loadError !== null && (
        <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {loadError}
        </p>
      )}

      <LaunchPanel
        onLaunch={(options) => {
          void launch(options);
        }}
        acting={acting}
        error={actionError}
        profile={profile}
      />

      {loading && sessions.length === 0 ? (
        <p className="px-1 py-6 text-sm text-slate-500">Loading browser sessions...</p>
      ) : (
        <div className="grid items-start gap-4 lg:grid-cols-[340px_minmax(0,1fr)]">
          <SessionListPanel
            sessions={sessions}
            selectedId={selectedId}
            onSelect={select}
            onStop={(sessionId) => {
              void stop(sessionId);
            }}
            busy={acting}
          />
          <TargetsPane session={selectedSession} />
        </div>
      )}
    </div>
  );
}
