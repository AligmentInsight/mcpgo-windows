// React hook for the tunnel log panel.
//
// Like the service log panel, the tunnel log is POLLED while the panel is
// open: the runtime writes its ndjson log in a separate process, and
// McpGo's IPC bridge sees nothing when it appends. Unlike the service log,
// the lines stay unparsed (the tunnel-client owns their schema) - the hook
// hands raw strings to the panel.
//
// The hook lives inside the log panel component (mounted only while the
// panel is open), so no polling happens while the panel is closed.

import { useEffect, useState } from "react";
import { readTunnelLog } from "../api/tunnels";

/** Re-read interval while the panel is open. */
const POLL_INTERVAL_MS = 2000;

export interface UseTunnelLogResult {
  /** Raw ndjson lines, oldest first (newest 2000). */
  lines: string[];
  /** True while the initial read is in flight. */
  loading: boolean;
}

/**
 * Polls the runtime log of definition `id`. Pass `null` to idle the hook
 * (no reads, no timers).
 */
export function useTunnelLog(id: string | null): UseTunnelLogResult {
  const [lines, setLines] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (id === null) {
      setLines([]);
      setLoading(false);
      return;
    }

    let cancelled = false;

    const readLog = (initial: boolean) => {
      if (initial) setLoading(true);
      readTunnelLog(id)
        .then((log) => {
          if (!cancelled) {
            setLines(log.lines);
            setLoading(false);
          }
        })
        .catch(() => {
          // Without the IPC bridge (browser preview) or on a read failure
          // the panel simply shows whatever it already has.
          if (!cancelled) setLoading(false);
        });
    };

    readLog(true);
    const timer = setInterval(() => {
      readLog(false);
    }, POLL_INTERVAL_MS);

    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [id]);

  return { lines, loading };
}
