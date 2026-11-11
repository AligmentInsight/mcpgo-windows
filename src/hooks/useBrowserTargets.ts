// React hook for the targets pane.
//
// The DevTools endpoint has no push channel McpGo could listen to (target
// changes happen inside Chrome), so the live list is POLLED while the pane
// is open: an initial read plus one re-read every three seconds - the same
// pattern as the service log panel. The hook lives inside the pane
// component (mounted only while a session is selected), so no polling
// happens while nothing consumes it.

import { useEffect, useState } from "react";
import { readBrowserTargets } from "../api/browser";
import type { BrowserTarget } from "../types/browser";

/** Re-read interval while a session is selected. */
const POLL_INTERVAL_MS = 3000;

export interface UseBrowserTargetsResult {
  /** Live targets of the session, pages first. */
  targets: BrowserTarget[];
  /** Version string of the moment of the last successful read. */
  chromeVersion: string | null;
  /** True while the initial read is in flight. */
  loading: boolean;
  /** Read failure of the last poll (cleared by the next successful read). */
  error: string | null;
}

/**
 * Polls the DevTools targets of session `sessionId`. Pass `null` to idle
 * the hook (no reads, no timers) - used when no session is selected.
 */
export function useBrowserTargets(sessionId: string | null): UseBrowserTargetsResult {
  const [targets, setTargets] = useState<BrowserTarget[]>([]);
  const [chromeVersion, setChromeVersion] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (sessionId === null) {
      setTargets([]);
      setChromeVersion(null);
      setLoading(false);
      setError(null);
      return;
    }

    let cancelled = false;

    const readTargets = (initial: boolean) => {
      if (initial) setLoading(true);
      readBrowserTargets(sessionId)
        .then((list) => {
          if (!cancelled) {
            setTargets(list.targets);
            setChromeVersion(list.chromeVersion);
            setError(null);
            setLoading(false);
          }
        })
        .catch((reason: unknown) => {
          // A session that just stopped (or whose endpoint hiccuped)
          // reports the failure; the next poll may still recover.
          if (!cancelled) {
            setError(String(reason));
            setLoading(false);
          }
        });
    };

    readTargets(true);
    const timer = setInterval(() => {
      readTargets(false);
    }, POLL_INTERVAL_MS);

    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [sessionId]);

  return { targets, chromeVersion, loading, error };
}
