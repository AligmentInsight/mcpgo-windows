// Launch form of the browser view: start URL, headless mode, explicit
// port and user data directory (both optional), plus extra Chrome flags.
//
// Field policy mirrors the backend contract (src-tauri/src/browser):
// * empty port    -> the next free port of the McpGo range is allocated;
// * empty profile -> a managed profile directory is used (the default
//   Chrome profile is rejected by the backend since Chrome 136);
// * reserved flags (--remote-debugging-port, --user-data-dir,
//   --remote-debugging-pipe) are dropped by the backend.

import { useState } from "react";
import Badge from "../common/Badge";
import type { BrowserLaunchOptions, BrowserProfile } from "../../types/browser";

export interface LaunchPanelProps {
  /** Receives the assembled launch options. */
  onLaunch: (options: BrowserLaunchOptions) => void;
  /** Disables the form while a launch or stop is in flight. */
  acting: boolean;
  /** Launch failure message, shown below the form. */
  error: string | null;
  /** Machine profile info (may still be loading when the form renders). */
  profile: BrowserProfile | null;
}

const INPUT_CLASS =
  "w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none";

export default function LaunchPanel({ onLaunch, acting, error, profile }: LaunchPanelProps) {
  const [startUrl, setStartUrl] = useState("");
  const [headless, setHeadless] = useState(false);
  const [port, setPort] = useState("");
  const [userDataDir, setUserDataDir] = useState("");
  const [extraArgs, setExtraArgs] = useState("");

  const chromeMissing = profile !== null && !profile.chromeAvailable;
  const parsedPort = port.trim() === "" ? null : Number(port.trim());
  const portInvalid = parsedPort !== null && (!Number.isInteger(parsedPort) || parsedPort < 1024 || parsedPort > 65535);
  const portHint =
    parsedPort !== null && !portInvalid
      ? `explicit port ${parsedPort}`
      : `auto-allocated from ${profile?.portRangeStart ?? 9222}..${profile?.portRangeEnd ?? 9321}`;

  const submit = () => {
    if (acting || portInvalid) return;
    const options: BrowserLaunchOptions = {
      port: parsedPort,
      headless,
      startUrl: startUrl.trim() === "" ? null : startUrl.trim(),
      userDataDir: userDataDir.trim() === "" ? null : userDataDir.trim(),
      extraArgs:
        extraArgs.trim() === ""
          ? []
          : extraArgs.trim().split(/\s+/).filter((arg) => arg.length > 0),
    };
    onLaunch(options);
  };

  return (
    <section className="rounded-xl border border-slate-800 bg-slate-900/60">
      <div className="border-b border-slate-800 px-5 py-4">
        <h2 className="text-sm font-semibold text-slate-100">Launch a debug-enabled Chrome</h2>
        <p className="mt-0.5 text-xs text-slate-400">
          Chrome starts with --remote-debugging-port and a non-default user data directory;
          every session is reachable on the loopback DevTools endpoint for Playwright/CDP.
        </p>
      </div>
      <div className="space-y-3 px-5 py-4">
        <div className="grid gap-3 sm:grid-cols-2">
          <div>
            <label htmlFor="browser-start-url" className="mb-1 block text-xs font-medium text-slate-400">
              Start URL (optional)
            </label>
            <input
              id="browser-start-url"
              type="text"
              value={startUrl}
              onChange={(event) => setStartUrl(event.target.value)}
              placeholder="https://example.com/"
              spellCheck={false}
              className={INPUT_CLASS}
            />
          </div>
          <div>
            <label htmlFor="browser-port" className="mb-1 block text-xs font-medium text-slate-400">
              DevTools port (optional)
            </label>
            <input
              id="browser-port"
              type="number"
              min={1024}
              max={65535}
              value={port}
              onChange={(event) => setPort(event.target.value)}
              placeholder="auto"
              spellCheck={false}
              className={INPUT_CLASS}
            />
          </div>
          <div className="sm:col-span-2">
            <label htmlFor="browser-user-data-dir" className="mb-1 block text-xs font-medium text-slate-400">
              User data directory (optional, non-default)
            </label>
            <input
              id="browser-user-data-dir"
              type="text"
              value={userDataDir}
              onChange={(event) => setUserDataDir(event.target.value)}
              placeholder="empty = managed profile per session"
              spellCheck={false}
              className={INPUT_CLASS}
            />
          </div>
          <div className="sm:col-span-2">
            <label htmlFor="browser-extra-args" className="mb-1 block text-xs font-medium text-slate-400">
              Extra Chrome flags (optional, space-separated)
            </label>
            <input
              id="browser-extra-args"
              type="text"
              value={extraArgs}
              onChange={(event) => setExtraArgs(event.target.value)}
              placeholder="--incognito --window-size=1280,800"
              spellCheck={false}
              className={INPUT_CLASS}
            />
          </div>
        </div>

        <label className="flex items-center gap-2 text-xs text-slate-300">
          <input
            type="checkbox"
            checked={headless}
            onChange={(event) => setHeadless(event.target.checked)}
            className="h-4 w-4 rounded border-slate-600 bg-slate-950 accent-sky-500"
          />
          Headless (no window; terminated when the application exits)
        </label>

        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-xs text-slate-500">
            Port: {portHint}. Managed profiles:
            <span className="ml-1 font-mono" title={profile?.managedProfilesDir ?? undefined}>
              {profile?.managedProfilesDir ?? "loading..."}
            </span>
          </p>
          <div className="flex items-center gap-2">
            {chromeMissing && <Badge tone="error">Chrome not found</Badge>}
            <button
              type="button"
              onClick={submit}
              disabled={acting || portInvalid || chromeMissing}
              className="rounded-lg bg-sky-500 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-400 disabled:cursor-not-allowed disabled:bg-slate-700 disabled:text-slate-400"
            >
              {acting ? "Working..." : "Launch"}
            </button>
          </div>
        </div>
        <p className="text-xs text-slate-500">
          Since Chrome 136 the debugging switch is ignored on the default profile - leave the
          directory empty or pick a non-default one; the default directory is rejected.
        </p>
        {error !== null && (
          <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
            {error}
          </p>
        )}
      </div>
    </section>
  );
}
