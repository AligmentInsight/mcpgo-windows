// TypeScript mirrors of the IPC payload types defined in
// src-tauri/src/jobs/mod.rs (JobStatus, JobState, JobLog, event payloads).
//
// serde(rename_all = "camelCase") on the Rust side produces camelCase keys
// on the wire. Keep these interfaces in sync when the Rust types change.

/** Lifecycle state of a job. */
export type JobState = "running" | "completed" | "failed" | "canceled";

/** Output stream a job_logs record belongs to. */
export type LogStream = "out" | "err" | "sys";

/** Contents of one job_status file / IPC `JobStatus`. */
export interface JobStatus {
  /** Generated id, e.g. "job_1731234567890_3". */
  id: string;
  /** Human-readable title (explicit or derived from the command). */
  title: string;
  /** The full PowerShell command. */
  command: string;
  /** Shell executing the job ("pwsh-7" | "powershell-5.1"). */
  shell: string;
  state: JobState;
  exitCode: number | null;
  /** True as soon as cancellation has been requested. */
  cancelRequested: boolean;
  /** Creation time, epoch milliseconds. */
  createdAt: number;
  /** Process spawn time, epoch milliseconds. */
  startedAt: number;
  /** Termination time, epoch milliseconds. */
  finishedAt: number | null;
  /** Absolute path of the job_logs file. */
  logFile: string;
  /** Absolute working directory, when one was requested. */
  workingDir: string | null;
  /** Non-fatal infrastructure error recorded by the backend. */
  error: string | null;
}

/** One parsed job_logs record. */
export interface JobLogLine {
  /** RFC 3339 timestamp as written in the file, or null for raw lines. */
  time: string | null;
  stream: LogStream;
  text: string;
}

/** Response of the `read_job_log` command. */
export interface JobLog {
  jobId: string;
  lines: JobLogLine[];
}

/** Payload of the `job://status` event. */
export interface JobStatusEventPayload {
  job: JobStatus;
}

/** Payload of the `job://output` event (invalidation signal). */
export interface JobOutputEventPayload {
  jobId: string;
}
