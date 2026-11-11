// React hook for the output pane of the selected job.
//
// Design (mirrors the backend contract in src-tauri/src/jobs/mod.rs):
// the job_logs FILE is the single source of truth. `job://output` events
// are treated purely as invalidation signals - the hook re-reads the log
// via `readJobLog` with a short debounce. This keeps live rendering and
// full history consistent when switching jobs without any merge logic.

import { useEffect, useState } from "react";
import { onJobOutputEvent, onJobStatusEvent, readJobLog } from "../api/terminal";
import type { JobLogLine } from "../types/job";

/** Debounce for log re-reads while output is streaming. */
const REFRESH_DEBOUNCE_MS = 150;

export interface UseJobLogResult {
  lines: JobLogLine[];
  loading: boolean;
}

export function useJobLog(jobId: string | null): UseJobLogResult {
  const [lines, setLines] = useState<JobLogLine[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (jobId === null) {
      setLines([]);
      setLoading(false);
      return;
    }

    let cancelled = false;
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;
    let unlistenOutput: (() => void) | undefined;
    let unlistenStatus: (() => void) | undefined;

    const readLog = (initial: boolean) => {
      if (initial) setLoading(true);
      readJobLog(jobId)
        .then((log) => {
          if (!cancelled) {
            setLines(log.lines);
            setLoading(false);
          }
        })
        .catch(() => {
          // A job that aborted before its log existed yields an empty
          // file / not-found: show whatever we have.
          if (!cancelled) setLoading(false);
        });
    };

    const scheduleRefresh = () => {
      if (refreshTimer !== undefined) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => {
        refreshTimer = undefined;
        readLog(false);
      }, REFRESH_DEBOUNCE_MS);
    };

    // Initial history read for the newly selected job.
    readLog(true);

    // Live updates: output events invalidate; the final status event
    // guarantees the footer record is fetched.
    onJobOutputEvent((id) => {
      if (id === jobId) scheduleRefresh();
    })
      .then((unsubscribe) => {
        unlistenOutput = unsubscribe;
      })
      .catch(() => {
        // Without the IPC bridge there are no events to react to.
      });

    onJobStatusEvent((job) => {
      if (job.id === jobId && job.state !== "running") scheduleRefresh();
    })
      .then((unsubscribe) => {
        unlistenStatus = unsubscribe;
      })
      .catch(() => {
        // Same as above.
      });

    return () => {
      cancelled = true;
      if (refreshTimer !== undefined) clearTimeout(refreshTimer);
      unlistenOutput?.();
      unlistenStatus?.();
    };
  }, [jobId]);

  return { lines, loading };
}
