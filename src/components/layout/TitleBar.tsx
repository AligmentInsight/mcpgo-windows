// In-app header bar: application identity (logo, name, platform tagline)
// and the running version read from the Tauri bundle metadata.
//
// The native OS window decorations stay enabled (decorations: true in
// tauri.conf.json), so this component is purely informational - it is NOT
// a draggable custom title bar.

import { useEffect, useState } from "react";
import { fetchAppVersion } from "../../api/app";

export default function TitleBar() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    fetchAppVersion()
      .then((v) => {
        if (!cancelled) setVersion(v);
      })
      .catch(() => {
        // Version is cosmetic: on failure the badge simply stays hidden.
        if (!cancelled) setVersion(null);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <header className="border-b border-slate-800 bg-slate-900/60">
      <div className="mx-auto flex max-w-5xl items-center justify-between px-6 py-4">
        <div className="flex items-center gap-3">
          <div
            aria-hidden="true"
            className="flex h-9 w-9 items-center justify-center rounded-lg bg-sky-500/15 font-mono text-sm text-sky-400"
          >
            {">_"}
          </div>
          <div>
            <h1 className="text-base font-semibold leading-tight">McpGo</h1>
            <p className="text-xs text-slate-400">MCP Manager for Windows</p>
          </div>
        </div>
        {version !== null && (
          <span className="rounded-full bg-slate-800 px-2.5 py-1 font-mono text-xs text-slate-300">
            v{version}
          </span>
        )}
      </div>
    </header>
  );
}
