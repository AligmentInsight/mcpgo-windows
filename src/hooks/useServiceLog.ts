// React hook for the service log panel.
//
// Unlike jobs (which invalidate their log view through `job://output`
// events), services have no push channel: the service worker runs in a
// separate process owned by the SCM, and McpGo's IPC bridge sees nothing
// when it appends to the log file. The log is therefore POLLED while the
// panel is open: an initial read plus one re-read every two seconds.
//
// The hook lives inside the log panel component (mounted only while the
// panel is open), so no polling happens while the panel is closed.

import { useEffect, useState } from "react";
import { readServiceLog } from "../api/services";
import type { JobLogLine } from "../types/job";

/** Re-read interval while the panel is open. */
const POLL_INTERVAL_MS = 2000;

export interface UseServiceLogResult {
  /** Parsed log records, oldest first. */
  lines: JobLogLine[];
  /** True while the initial read is in flight. */
  loading: boolean;
}

/**
 * Polls the service log of definition `id`. Pass `null` to idle the hook
 * (no reads, no timers).
 */
export function useServiceLog(id: string | null): UseServiceLogResult {
  const [lines, setLines] = useState<JobLogLine[]>([]);
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
      readServiceLog(id)
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
