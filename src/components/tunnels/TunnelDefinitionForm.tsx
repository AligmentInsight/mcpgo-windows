// Create/edit form for tunnel definitions. Textareas use a one-item-per-
// line format (arguments: one per line) like the service form: the target
// users are developers, and raw line-based editing is transparent and
// paste-friendly.
//
// The key source drives the reference field: managed keys need no
// reference (the Set key action on the card stores the value), env keys
// take a variable name, file keys take an absolute path.
//
// The parent remounts this component (React key) when switching the edited
// tunnel or reopening the editor, so plain useState initialization from
// `initial` is sufficient - no prop-sync effects needed.

import { useState } from "react";
import type {
  ApiKeySourceKind,
  TunnelDefinition,
  TunnelDefinitionInput,
  TunnelResidency,
  TunnelTargetKind,
} from "../../types/tunnel";
import { API_KEY_SOURCE_META, RESIDENCY_OPTIONS, TARGET_META } from "./tunnelStyle";

export interface TunnelDefinitionFormProps {
  mode: "create" | "edit";
  /** Definition to prefill in edit mode (ignored in create mode). */
  initial?: TunnelDefinition;
  /** Disables the submit button while an action is in flight. */
  busy: boolean;
  /** Server-side error message of the last action. */
  error: string | null;
  /** Receives the parsed input and, in create mode, the optional id. */
  onSubmit: (input: TunnelDefinitionInput, id?: string) => void;
  onCancel: () => void;
}

/** One argument per line; blank lines are dropped, spaces are kept. */
function parseArgs(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

/** Parses a port input; null when blank, NaN when malformed. */
function parsePort(text: string): number | null | "NaN" {
  const trimmed = text.trim();
  if (trimmed.length === 0) return null;
  const port = Number(trimmed);
  return Number.isInteger(port) ? port : "NaN";
}

export default function TunnelDefinitionForm({
  mode,
  initial,
  busy,
  error,
  onSubmit,
  onCancel,
}: TunnelDefinitionFormProps) {
  const [name, setName] = useState(initial?.name ?? "");
  const [explicitId, setExplicitId] = useState("");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [tunnelId, setTunnelId] = useState(initial?.tunnelId ?? "");
  const [targetKind, setTargetKind] = useState<TunnelTargetKind>(
    initial?.targetKind ?? "stdio",
  );
  const [command, setCommand] = useState(initial?.command ?? "");
  const [argsText, setArgsText] = useState((initial?.args ?? []).join("\n"));
  const [workingDir, setWorkingDir] = useState(initial?.workingDir ?? "");
  const [serverUrl, setServerUrl] = useState(initial?.serverUrl ?? "");
  const [apiKeyKind, setApiKeyKind] = useState<ApiKeySourceKind>(
    initial?.apiKey.kind ?? "managed",
  );
  const [apiKeyReference, setApiKeyReference] = useState(
    initial?.apiKey.reference ?? "",
  );
  const [residency, setResidency] = useState<TunnelResidency>(
    initial?.residency ?? "global",
  );
  const [autostart, setAutostart] = useState(initial?.autostart ?? false);
  const [keepAlive, setKeepAlive] = useState(initial?.keepAlive ?? false);
  const [clientPath, setClientPath] = useState(initial?.clientPath ?? "");
  const [healthPortText, setHealthPortText] = useState(
    initial?.healthPort !== null && initial !== undefined && initial.healthPort !== null
      ? String(initial.healthPort)
      : "",
  );
  const [parseError, setParseError] = useState<string | null>(null);

  const canSubmit =
    name.trim().length > 0 &&
    tunnelId.trim().length > 0 &&
    (targetKind === "stdio"
      ? command.trim().length > 0
      : serverUrl.trim().length > 0) &&
    !busy;

  const submit = () => {
    setParseError(null);

    const port = parsePort(healthPortText);
    if (port === "NaN") {
      setParseError("The health port must be a whole number (1024..65535) or empty.");
      return;
    }

    const trimmedWorkingDir = workingDir.trim();
    const trimmedClientPath = clientPath.trim();
    const trimmedReference = apiKeyReference.trim();
    if (apiKeyKind !== "managed" && trimmedReference.length === 0) {
      setParseError(
        apiKeyKind === "env"
          ? "The environment variable name must not be empty."
          : "The key file path must not be empty.",
      );
      return;
    }

    const input: TunnelDefinitionInput = {
      name: name.trim(),
      description: description.trim(),
      tunnelId: tunnelId.trim(),
      targetKind,
      command: targetKind === "stdio" ? command.trim() : null,
      args: targetKind === "stdio" ? parseArgs(argsText) : [],
      workingDir:
        targetKind === "stdio" && trimmedWorkingDir.length > 0
          ? trimmedWorkingDir
          : null,
      serverUrl: targetKind === "http" ? serverUrl.trim() : null,
      apiKey: {
        kind: apiKeyKind,
        reference:
          apiKeyKind !== "managed" && trimmedReference.length > 0
            ? trimmedReference
            : null,
      },
      residency,
      autostart,
      keepAlive,
      clientPath: trimmedClientPath.length > 0 ? trimmedClientPath : null,
      healthPort: port,
    };
    onSubmit(
      input,
      mode === "create" && explicitId.trim().length > 0 ? explicitId.trim() : undefined,
    );
  };

  const inputClass =
    "w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 placeholder:text-slate-600 focus:border-sky-500 focus:outline-none";
  const monoInputClass = `${inputClass} font-mono`;

  return (
    <section className="rounded-xl border border-slate-800 bg-slate-900/60">
      <div className="border-b border-slate-800 px-5 py-4">
        <h2 className="text-sm font-semibold text-slate-100">
          {mode === "create" ? "New tunnel definition" : `Edit ${initial?.id ?? "tunnel"}`}
        </h2>
        <p className="mt-0.5 text-xs text-slate-400">
          One OpenAI Secure MCP Tunnel: the tunnel id, the local MCP server it exposes
          and where the runtime reads its API key.
        </p>
      </div>

      <div className="space-y-3 px-5 py-4">
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-56 flex-1">
            <label htmlFor="tun-name" className="mb-1 block text-xs font-medium text-slate-400">
              Display name
            </label>
            <input
              id="tun-name"
              type="text"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="My Files MCP"
              className={inputClass}
            />
          </div>
          {mode === "create" && (
            <div className="min-w-48 flex-1">
              <label htmlFor="tun-id" className="mb-1 block text-xs font-medium text-slate-400">
                Tunnel id (optional)
              </label>
              <input
                id="tun-id"
                type="text"
                value={explicitId}
                onChange={(event) => setExplicitId(event.target.value)}
                placeholder="my-files-mcp"
                spellCheck={false}
                className={monoInputClass}
              />
              <p className="mt-1 text-[11px] text-slate-500">
                Derived from the name when empty. Lowercase letters, digits, hyphens.
              </p>
            </div>
          )}
          <div className="min-w-72 flex-1">
            <label
              htmlFor="tun-openai-id"
              className="mb-1 block text-xs font-medium text-slate-400"
            >
              OpenAI tunnel id
            </label>
            <input
              id="tun-openai-id"
              type="text"
              value={tunnelId}
              onChange={(event) => setTunnelId(event.target.value)}
              placeholder="tunnel_0123456789abcdef0123456789abcdef"
              spellCheck={false}
              className={monoInputClass}
            />
            <p className="mt-1 text-[11px] text-slate-500">
              Create it in the Platform tunnel settings (link in the view header).
            </p>
          </div>
        </div>

        <div>
          <label htmlFor="tun-description" className="mb-1 block text-xs font-medium text-slate-400">
            Description (optional)
          </label>
          <input
            id="tun-description"
            type="text"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            placeholder="What this tunnel exposes"
            className={inputClass}
          />
        </div>

        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-40">
            <label
              htmlFor="tun-target"
              className="mb-1 block text-xs font-medium text-slate-400"
            >
              MCP server
            </label>
            <select
              id="tun-target"
              value={targetKind}
              onChange={(event) => setTargetKind(event.target.value as TunnelTargetKind)}
              className={inputClass}
            >
              {(Object.keys(TARGET_META) as TunnelTargetKind[]).map((kind) => (
                <option key={kind} value={kind}>
                  {TARGET_META[kind].label}
                </option>
              ))}
            </select>
          </div>
          <div className="min-w-40">
            <label
              htmlFor="tun-residency"
              className="mb-1 block text-xs font-medium text-slate-400"
            >
              Control plane
            </label>
            <select
              id="tun-residency"
              value={residency}
              onChange={(event) => setResidency(event.target.value as TunnelResidency)}
              className={inputClass}
            >
              {RESIDENCY_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
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
            Start automatically with McpGo
          </label>
          <label className="flex cursor-pointer items-center gap-2 pb-2 text-xs text-slate-400">
            <input
              type="checkbox"
              checked={keepAlive}
              onChange={(event) => setKeepAlive(event.target.checked)}
              className="h-4 w-4 rounded border-slate-600 bg-slate-950 accent-sky-500"
            />
            Restart automatically on failure
          </label>
        </div>

        {targetKind === "stdio" ? (
          <>
            <div>
              <label htmlFor="tun-command" className="mb-1 block text-xs font-medium text-slate-400">
                MCP server command
              </label>
              <input
                id="tun-command"
                type="text"
                value={command}
                onChange={(event) => setCommand(event.target.value)}
                placeholder={TARGET_META.stdio.commandHint}
                spellCheck={false}
                className={monoInputClass}
              />
              <p className="mt-1 text-[11px] text-slate-500">
                Executable of the MCP server the runtime spawns as its child. No double
                quotes - quote paths with spaces per argument below instead.
              </p>
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <div>
                <label htmlFor="tun-args" className="mb-1 block text-xs font-medium text-slate-400">
                  Arguments (one per line)
                </label>
                <textarea
                  id="tun-args"
                  rows={3}
                  value={argsText}
                  onChange={(event) => setArgsText(event.target.value)}
                  placeholder={"C:\\My Servers\\mcp-server.js\n--stdio"}
                  spellCheck={false}
                  className={`${monoInputClass} resize-y`}
                />
              </div>
              <div>
                <label
                  htmlFor="tun-working-dir"
                  className="mb-1 block text-xs font-medium text-slate-400"
                >
                  Working directory (optional, absolute)
                </label>
                <input
                  id="tun-working-dir"
                  type="text"
                  value={workingDir}
                  onChange={(event) => setWorkingDir(event.target.value)}
                  placeholder="C:\\Projects\\mcp-server"
                  spellCheck={false}
                  className={monoInputClass}
                />
              </div>
            </div>
          </>
        ) : (
          <div>
            <label htmlFor="tun-url" className="mb-1 block text-xs font-medium text-slate-400">
              MCP server URL
            </label>
            <input
              id="tun-url"
              type="text"
              value={serverUrl}
              onChange={(event) => setServerUrl(event.target.value)}
              placeholder={TARGET_META.http.commandHint}
              spellCheck={false}
              className={monoInputClass}
            />
            <p className="mt-1 text-[11px] text-slate-500">
              A streamable-HTTP endpoint the runtime forwards to (it must already be
              running - manage it as a McpGo service if it should be).
            </p>
          </div>
        )}

        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-44">
            <label
              htmlFor="tun-key-source"
              className="mb-1 block text-xs font-medium text-slate-400"
            >
              API key source
            </label>
            <select
              id="tun-key-source"
              value={apiKeyKind}
              onChange={(event) => setApiKeyKind(event.target.value as ApiKeySourceKind)}
              className={inputClass}
            >
              {(Object.keys(API_KEY_SOURCE_META) as ApiKeySourceKind[]).map((kind) => (
                <option key={kind} value={kind}>
                  {API_KEY_SOURCE_META[kind].label}
                </option>
              ))}
            </select>
          </div>
          {apiKeyKind !== "managed" && (
            <div className="min-w-64 flex-1">
              <label
                htmlFor="tun-key-reference"
                className="mb-1 block text-xs font-medium text-slate-400"
              >
                {apiKeyKind === "env" ? "Environment variable" : "Key file (absolute)"}
              </label>
              <input
                id="tun-key-reference"
                type="text"
                value={apiKeyReference}
                onChange={(event) => setApiKeyReference(event.target.value)}
                placeholder={
                  apiKeyKind === "env"
                    ? "CONTROL_PLANE_API_KEY"
                    : "C:\\Users\\dev\\.config\\tunnel-client\\runtime.key"
                }
                spellCheck={false}
                className={monoInputClass}
              />
            </div>
          )}
        </div>
        <p className="text-[11px] text-slate-500">
          {API_KEY_SOURCE_META[apiKeyKind].hint}
        </p>

        <div className="grid gap-3 sm:grid-cols-2">
          <div>
            <label
              htmlFor="tun-client-path"
              className="mb-1 block text-xs font-medium text-slate-400"
            >
              tunnel-client.exe (optional, absolute)
            </label>
            <input
              id="tun-client-path"
              type="text"
              value={clientPath}
              onChange={(event) => setClientPath(event.target.value)}
              placeholder="C:\\Tools\\tunnel-client\\tunnel-client.exe"
              spellCheck={false}
              className={monoInputClass}
            />
            <p className="mt-1 text-[11px] text-slate-500">
              Overrides the PATH lookup and the managed bin directory.
            </p>
          </div>
          <div>
            <label
              htmlFor="tun-health-port"
              className="mb-1 block text-xs font-medium text-slate-400"
            >
              Health port (optional)
            </label>
            <input
              id="tun-health-port"
              type="text"
              inputMode="numeric"
              value={healthPortText}
              onChange={(event) => setHealthPortText(event.target.value)}
              placeholder="auto (18080..18179)"
              spellCheck={false}
              className={monoInputClass}
            />
            <p className="mt-1 text-[11px] text-slate-500">
              Pinned loopback port of the runtime&apos;s health endpoint; empty allocates
              the next free port of the McpGo range.
            </p>
          </div>
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
            {mode === "create" ? "Create tunnel" : "Save changes"}
          </button>
        </div>
      </div>
    </section>
  );
}
