// React hook encapsulating the lifecycle of the environment report IPC
// call: loading, error and result states, with cancellation on unmount.
// The report is computed once by the Rust backend at startup, so the hook
// performs exactly one invocation and never re-probes the file system.

import { useEffect, useState } from "react";
import { fetchEnvironmentReport } from "../api/environment";
import type { EnvironmentReport } from "../types/environment";

export interface UseEnvironmentReportResult {
  report: EnvironmentReport | null;
  error: string | null;
  loading: boolean;
}

export function useEnvironmentReport(): UseEnvironmentReportResult {
  const [report, setReport] = useState<EnvironmentReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState<boolean>(true);

  useEffect(() => {
    let cancelled = false;

    fetchEnvironmentReport()
      .then((value) => {
        if (!cancelled) {
          setReport(value);
          setLoading(false);
        }
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(String(reason));
          setLoading(false);
        }
      });

    // StrictMode in development runs effects twice on mount; the flag
    // guarantees the state of a discarded first pass is never applied.
    return () => {
      cancelled = true;
    };
  }, []);

  return { report, error, loading };
}
