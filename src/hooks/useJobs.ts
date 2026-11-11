// React hook for the job list: initial load, live updates through the
// job://status event and the user actions (start / cancel / select).

import { useCallback, useEffect, useState } from "react";
import { cancelJob, listJobs, onJobStatusEvent, startJob } from "../api/terminal";
import type { JobStatus } from "../types/job";

export interface UseJobsResult {
  /** Known jobs, newest first. */
  jobs: JobStatus[];
  /** Currently selected job id (drives the output pane). */
  selectedId: string | null;
  select: (jobId: string) => void;
  /** Starts a new job and selects it. */
  start: (command: string, workingDir?: string) => Promise<void>;
  /** Requests cancellation of a running job. */
  cancel: (jobId: string) => Promise<void>;
  starting: boolean;
  startError: string | null;
  cancelError: string | null;
}

/** Newest first; stable tie-breaking for jobs created in the same millisecond. */
function sortJobs(list: JobStatus[]): JobStatus[] {
  return [...list].sort(
    (a, b) => b.createdAt - a.createdAt || b.startedAt - a.startedAt || b.id.localeCompare(a.id),
  );
}

/** Inserts or replaces a job, keeping the list sorted. */
function upsert(list: JobStatus[], job: JobStatus): JobStatus[] {
  return sortJobs([...list.filter((existing) => existing.id !== job.id), job]);
}

export function useJobs(): UseJobsResult {
  const [jobs, setJobs] = useState<JobStatus[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const [cancelError, setCancelError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    listJobs()
      .then((list) => {
        if (!cancelled) setJobs(sortJobs(list));
      })
      .catch(() => {
        // IPC unavailable (e.g. plain browser preview): the list stays
        // empty and the output pane shows the bridge error instead.
      });

    onJobStatusEvent((job) => {
      setJobs((previous) => upsert(previous, job));
    })
      .then((unsubscribe) => {
        unlisten = unsubscribe;
      })
      .catch(() => {
        // Same as above: without the bridge there are no events either.
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const start = useCallback(async (command: string, workingDir?: string) => {
    setStarting(true);
    setStartError(null);
    try {
      const job = await startJob(command, workingDir);
      setJobs((previous) => upsert(previous, job));
      setSelectedId(job.id);
    } catch (reason: unknown) {
      setStartError(String(reason));
    } finally {
      setStarting(false);
    }
  }, []);

  const cancel = useCallback(async (jobId: string) => {
    setCancelError(null);
    try {
      const job = await cancelJob(jobId);
      setJobs((previous) => upsert(previous, job));
    } catch (reason: unknown) {
      setCancelError(String(reason));
    }
  }, []);

  return {
    jobs,
    selectedId,
    select: setSelectedId,
    start,
    cancel,
    starting,
    startError,
    cancelError,
  };
}
