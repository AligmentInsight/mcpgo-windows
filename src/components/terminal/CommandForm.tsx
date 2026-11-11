// Command entry form of the terminal view: a multi-line PowerShell command,
// an optional absolute working directory and the Run action.
//
// Keyboard model: Enter runs the command, Shift+Enter inserts a newline.

import { useState, type KeyboardEvent } from "react";

export interface CommandFormProps {
  /** Receives the trimmed command and the optional working directory. */
  onRun: (command: string, workingDir?: string) => void;
  /** Disables the form while a job is being started. */
  running: boolean;
  /** Start failure message, shown below the form. */
  error: string | null;
}

export default function CommandForm({ onRun, running, error }: CommandFormProps) {
  const [command, setCommand] = useState("");
  const [workingDir, setWorkingDir] = useState("");

  const submit = () => {
    const trimmed = command.trim();
    if (trimmed.length === 0 || running) return;
    onRun(trimmed, workingDir.trim() || undefined);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
    }
  };

  return (
    <section className="rounded-xl border border-slate-800 bg-slate-900/60">
      <div className="border-b border-slate-800 px-5 py-4">
        <h2 className="text-sm font-semibold text-slate-100">Run a PowerShell job</h2>
        <p className="mt-0.5 text-xs text-slate-400">
          Each run is tracked as a job with a job_status file and a job_logs file.
        </p>
      </div>
      <div className="space-y-3 px-5 py-4">
        <div>
          <label htmlFor="job-command" className="mb-1 block text-xs font-medium text-slate-400">
            PowerShell command
          </label>
          <textarea
            id="job-command"
            rows={3}
            value={command}
            onChange={(event) => setCommand(event.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={"Get-Process | Sort-Object CPU -Descending | Select-Object -First 5"}
            spellCheck={false}
            className="w-full resize-y rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
          />
        </div>
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-64 flex-1">
            <label
              htmlFor="job-working-dir"
              className="mb-1 block text-xs font-medium text-slate-400"
            >
              Working directory (optional, absolute)
            </label>
            <input
              id="job-working-dir"
              type="text"
              value={workingDir}
              onChange={(event) => setWorkingDir(event.target.value)}
              placeholder="C:\\Projects"
              spellCheck={false}
              className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
            />
          </div>
          <button
            type="button"
            onClick={submit}
            disabled={command.trim().length === 0 || running}
            className="rounded-lg bg-sky-500 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-400 disabled:cursor-not-allowed disabled:bg-slate-700 disabled:text-slate-400"
          >
            {running ? "Starting..." : "Run"}
          </button>
        </div>
        <p className="text-xs text-slate-500">
          Enter runs the command, Shift+Enter adds a new line. Output streams into the selected
          job's pane.
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
