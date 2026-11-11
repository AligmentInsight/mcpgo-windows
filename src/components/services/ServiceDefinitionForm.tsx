// Create/edit form for service definitions. Textareas use a one-item-per-
// line format (arguments: one per line; environment: KEY=VALUE per line):
// the target users are developers, and raw line-based editing is
// transparent and paste-friendly.
//
// The parent remounts this component (React key) when switching the edited
// service or reopening the editor, so plain useState initialization from
// `initial` is sufficient - no prop-sync effects needed.

import { useState } from "react";
import type { RuntimeKind, ServiceDefinition, ServiceDefinitionInput } from "../../types/service";
import { RUNTIME_META } from "./serviceStyle";

export interface ServiceDefinitionFormProps {
  mode: "create" | "edit";
  /** Definition to prefill in edit mode (ignored in create mode). */
  initial?: ServiceDefinition;
  /** Disables the submit button while an action is in flight. */
  busy: boolean;
  /** Server-side error message of the last action. */
  error: string | null;
  /** Receives the parsed input and, in create mode, the optional id. */
  onSubmit: (input: ServiceDefinitionInput, id?: string) => void;
  onCancel: () => void;
}

/** Environment keys: a letter or underscore first, then word characters. */
const ENV_KEY_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** Parses "KEY=VALUE" lines; returns null when a line is malformed. */
function parseEnv(text: string): Record<string, string> | null {
  const env: Record<string, string> = {};
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (trimmed.length === 0) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) return null;
    const key = trimmed.slice(0, eq).trim();
    if (!ENV_KEY_PATTERN.test(key)) return null;
    env[key] = trimmed.slice(eq + 1).trim();
  }
  return env;
}

/** One argument per line; blank lines are dropped, spaces are kept. */
function parseArgs(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

/** Renders env entries back into textarea text (KEY=VALUE per line). */
function formatEnv(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
}

export default function ServiceDefinitionForm({
  mode,
  initial,
  busy,
  error,
  onSubmit,
  onCancel,
}: ServiceDefinitionFormProps) {
  const [name, setName] = useState(initial?.name ?? "");
  const [explicitId, setExplicitId] = useState("");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [runtime, setRuntime] = useState<RuntimeKind>(initial?.runtime ?? "node");
  const [command, setCommand] = useState(initial?.command ?? "");
  const [argsText, setArgsText] = useState((initial?.args ?? []).join("\n"));
  const [workingDir, setWorkingDir] = useState(initial?.workingDir ?? "");
  const [envText, setEnvText] = useState(initial ? formatEnv(initial.env) : "");
  const [autostart, setAutostart] = useState(initial?.autostart ?? false);
  const [keepAlive, setKeepAlive] = useState(initial?.keepAlive ?? false);
  const [runtimePath, setRuntimePath] = useState(initial?.runtimePath ?? "");
  const [parseError, setParseError] = useState<string | null>(null);

  const canSubmit = name.trim().length > 0 && command.trim().length > 0 && !busy;

  const submit = () => {
    setParseError(null);
    const env = parseEnv(envText);
    if (env === null) {
      setParseError(
        "Each environment line must be KEY=VALUE (keys: letters, digits, underscores; starting with a letter or underscore).",
      );
      return;
    }
    const trimmedWorkingDir = workingDir.trim();
    const trimmedRuntimePath = runtimePath.trim();
    const input: ServiceDefinitionInput = {
      name: name.trim(),
      description: description.trim(),
      runtime,
      command: command.trim(),
      args: parseArgs(argsText),
      workingDir: trimmedWorkingDir.length > 0 ? trimmedWorkingDir : null,
      env,
      autostart,
      keepAlive,
      runtimePath: trimmedRuntimePath.length > 0 ? trimmedRuntimePath : null,
    };
    onSubmit(
      input,
      mode === "create" && explicitId.trim().length > 0 ? explicitId.trim() : undefined,
    );
  };

  return (
    <section className="rounded-xl border border-slate-800 bg-slate-900/60">
      <div className="border-b border-slate-800 px-5 py-4">
        <h2 className="text-sm font-semibold text-slate-100">
          {mode === "create" ? "New service definition" : `Edit ${initial?.id ?? "service"}`}
        </h2>
        <p className="mt-0.5 text-xs text-slate-400">
          The Windows port of a LaunchAgent plist: runtime, command and environment of one
          background service.
        </p>
      </div>

      <div className="space-y-3 px-5 py-4">
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-56 flex-1">
            <label htmlFor="svc-name" className="mb-1 block text-xs font-medium text-slate-400">
              Display name
            </label>
            <input
              id="svc-name"
              type="text"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="My MCP Server"
              className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
            />
          </div>
          {mode === "create" && (
            <div className="min-w-48 flex-1">
              <label htmlFor="svc-id" className="mb-1 block text-xs font-medium text-slate-400">
                Service id (optional)
              </label>
              <input
                id="svc-id"
                type="text"
                value={explicitId}
                onChange={(event) => setExplicitId(event.target.value)}
                placeholder="my-mcp-server"
                spellCheck={false}
                className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
              />
              <p className="mt-1 text-[11px] text-slate-500">
                Derived from the name when empty. Lowercase letters, digits, hyphens.
              </p>
            </div>
          )}
        </div>

        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-48">
            <label htmlFor="svc-runtime" className="mb-1 block text-xs font-medium text-slate-400">
              Runtime
            </label>
            <select
              id="svc-runtime"
              value={runtime}
              onChange={(event) => setRuntime(event.target.value as RuntimeKind)}
              className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 focus:border-sky-500 focus:outline-none"
            >
              {(Object.keys(RUNTIME_META) as RuntimeKind[]).map((kind) => (
                <option key={kind} value={kind}>
                  {RUNTIME_META[kind].label}
                </option>
              ))}
            </select>
          </div>
          <label className="flex cursor-pointer items-center gap-2 pb-2 text-xs text-slate-400">
            <input
              type="checkbox"
              checked={autostart}
              onChange={(event) => setAutostart(event.target.checked)}
              className="h-4 w-4 rounded border-slate-600 bg-slate-950 accent-sky-500"
            />
            Start automatically at system boot
          </label>
        </div>

        <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
          <label className="flex cursor-pointer items-center gap-2 text-xs text-slate-400">
            <input
              type="checkbox"
              checked={keepAlive}
              onChange={(event) => setKeepAlive(event.target.checked)}
              className="h-4 w-4 rounded border-slate-600 bg-slate-950 accent-sky-500"
            />
            Restart automatically on failure
            <span className="text-slate-500">(LaunchAgent KeepAlive)</span>
          </label>
        </div>

        <div>
          <label htmlFor="svc-description" className="mb-1 block text-xs font-medium text-slate-400">
            Description (optional)
          </label>
          <input
            id="svc-description"
            type="text"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            placeholder="What this service does"
            className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
          />
        </div>

        <div>
          <label htmlFor="svc-command" className="mb-1 block text-xs font-medium text-slate-400">
            Command
          </label>
          <input
            id="svc-command"
            type="text"
            value={command}
            onChange={(event) => setCommand(event.target.value)}
            placeholder={RUNTIME_META[runtime].commandHint}
            spellCheck={false}
            className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
          />
          <p className="mt-1 text-[11px] text-slate-500">
            {RUNTIME_META[runtime].commandHint}
          </p>
        </div>

        <div className="grid gap-3 sm:grid-cols-2">
          <div>
            <label htmlFor="svc-args" className="mb-1 block text-xs font-medium text-slate-400">
              Arguments (one per line)
            </label>
            <textarea
              id="svc-args"
              rows={3}
              value={argsText}
              onChange={(event) => setArgsText(event.target.value)}
              placeholder={"--port\n3000"}
              spellCheck={false}
              className="w-full resize-y rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
            />
          </div>
          <div>
            <label htmlFor="svc-env" className="mb-1 block text-xs font-medium text-slate-400">
              Environment (KEY=VALUE per line)
            </label>
            <textarea
              id="svc-env"
              rows={3}
              value={envText}
              onChange={(event) => setEnvText(event.target.value)}
              placeholder={"MCP_TRANSPORT=stdio\nNODE_ENV=production"}
              spellCheck={false}
              className="w-full resize-y rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
            />
          </div>
        </div>

        <div>
          <label htmlFor="svc-working-dir" className="mb-1 block text-xs font-medium text-slate-400">
            Working directory (optional, absolute)
          </label>
          <input
            id="svc-working-dir"
            type="text"
            value={workingDir}
            onChange={(event) => setWorkingDir(event.target.value)}
            placeholder="C:\\Projects\\mcp-server"
            spellCheck={false}
            className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
          />
        </div>

        <div>
          <label htmlFor="svc-runtime-path" className="mb-1 block text-xs font-medium text-slate-400">
            Runtime executable (optional, absolute)
          </label>
          <input
            id="svc-runtime-path"
            type="text"
            value={runtimePath}
            onChange={(event) => setRuntimePath(event.target.value)}
            placeholder={RUNTIME_META[runtime].runtimePathHint}
            spellCheck={false}
            className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none"
          />
          <p className="mt-1 text-[11px] text-slate-500">
            Overrides the PATH lookup. Recommended for per-user runtime installs: the
            installed service runs as LocalSystem, whose PATH differs from yours.
          </p>
        </div>

        {parseError !== null && (
          <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-300">
            {parseError}
          </p>
        )}
        {error !== null && (
          <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
            {error}
          </p>
        )}

        <div className="flex items-center justify-end gap-2 pt-1">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="rounded-lg border border-slate-700 px-4 py-2 text-sm text-slate-300 transition hover:border-slate-500 hover:text-slate-100 disabled:cursor-not-allowed disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={submit}
            disabled={!canSubmit}
            className="rounded-lg bg-sky-500 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-400 disabled:cursor-not-allowed disabled:bg-slate-700 disabled:text-slate-400"
          >
            {mode === "create" ? "Create service" : "Save changes"}
          </button>
        </div>
      </div>
    </section>
  );
}
