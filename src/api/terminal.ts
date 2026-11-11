// IPC wrappers for the terminal commands implemented in
// src-tauri/src/commands/terminal.rs and for the `job://*` events emitted
// by the job registry (src-tauri/src/jobs/mod.rs).
//
// Event names MUST stay in sync with the EVENT_JOB_* constants there.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  JobLog,
  JobStatus,
  JobOutputEventPayload,
  JobStatusEventPayload,
} from "../types/job";

/** Starts a new PowerShell job; returns its initial Running status. */
export function startJob(
  command: string,
  workingDir?: string,
  title?: string,
): Promise<JobStatus> {
  return invoke<JobStatus>("start_job", {
    command,
    workingDir: workingDir ?? null,
    title: title ?? null,
  });
}

/**
 * Requests cancellation of a running job. Resolves with the current status
 * (still "running", cancelRequested: true); the final "canceled" status
 * arrives through a job://status event.
 */
export function cancelJob(jobId: string): Promise<JobStatus> {
  return invoke<JobStatus>("cancel_job", { jobId });
}

/** Lists known jobs, newest first (current and previous sessions). */
export function listJobs(): Promise<JobStatus[]> {
  return invoke<JobStatus[]>("list_jobs");
}

/** Reads the parsed job_logs records of one job, oldest first. */
export function readJobLog(jobId: string): Promise<JobLog> {
  return invoke<JobLog>("read_job_log", { jobId });
}

/**
 * Subscribes to `job://output`. The event is an invalidation signal only -
 * the log file is the source of truth, listeners re-read it via
 * `readJobLog` (debounced).
 *
 * Returns an unsubscribe function.
 */
export function onJobOutputEvent(
  handler: (jobId: string) => void,
): Promise<() => void> {
  return listen<JobOutputEventPayload>("job://output", (event) => {
    handler(event.payload.jobId);
  });
}

/** Subscribes to `job://status` (every state transition). */
export function onJobStatusEvent(
  handler: (job: JobStatus) => void,
): Promise<() => void> {
  return listen<JobStatusEventPayload>("job://status", (event) => {
    handler(event.payload.job);
  });
}
