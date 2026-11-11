// Tunnels view - the Phase 6 face of the OpenAI Secure MCP Tunnel domain.
// Composition only: data flows through useTunnels, presentation lives in
// TunnelCard, TunnelDefinitionForm and TunnelLogPanel.
//
// The header renders the machine profile: the tunnel-client availability
// (with the managed bin directory shown as the install target), the
// managed health port range and the OpenAI links (tunnel management,
// docs, releases) as copyable mono text.

import { useState } from "react";
import TunnelCard from "./TunnelCard";
import TunnelDefinitionForm from "./TunnelDefinitionForm";
import { useTunnels } from "../../hooks/useTunnels";
import type { TunnelSummary } from "../../types/tunnel";

type EditorState =
  | { mode: "closed" }
  | { mode: "create" }
  | { mode: "edit"; summary: TunnelSummary };

export default function TunnelsView() {
  const {
    summaries,
    profile,
    loading,
    loadError,
    refresh,
    create,
    update,
    remove,
    setApiKey,
    start,
    stop,
    acting,
    actionError,
  } = useTunnels();
  const [editor, setEditor] = useState<EditorState>({ mode: "closed" });
  const [logsOpenId, setLogsOpenId] = useState<string | null>(null);

  const closeEditor = () => setEditor({ mode: "closed" });

  const toggleLogs = (summary: TunnelSummary) => {
    setLogsOpenId((current) =>
      current === summary.definition.id ? null : summary.definition.id,
    );
  };

  const handleSetKey = async (summary: TunnelSummary) => {
    const key = window.prompt(
      `Store the runtime API key of "${summary.definition.name}"?\n\n` +
        "The key is written to McpGo's secrets directory, passed to the runtime " +
        "through its profile and never shown again.",
      "",
    );
    if (key === null) return; // cancelled
    await setApiKey(summary.definition.id, key);
  };

  const handleDelete = async (summary: TunnelSummary) => {
    const message =
      `Delete the tunnel definition "${summary.definition.name}"?\n\n` +
      "The definition file, the generated profile and the managed key file are " +
      "removed; the runtime log stays behind as evidence.";
    if (!window.confirm(message)) return;
    const done = await remove(summary.definition.id);
    if (done) {
      setEditor((current) =>
        current.mode === "edit" && current.summary.definition.id === summary.definition.id
          ? { mode: "closed" }
          : current,
      );
      setLogsOpenId((current) =>
        current === summary.definition.id ? null : current,
      );
    }
  };

  const handleStop = async (summary: TunnelSummary) => {
    const message =
      `Stop the runtime of "${summary.definition.name}"?\n\n` +
      "The tunnel-client process is terminated (its stdio MCP child goes with it). " +
      "The tunnel definition stays; start it again at any time.";
    if (!window.confirm(message)) return;
    await stop(summary.definition.id);
  };

  return (
    <div className="space-y-4">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-slate-100">Tunnels</h1>
          <p className="text-xs text-slate-400">
            OpenAI Secure MCP Tunnels: expose local MCP servers to ChatGPT, Codex and
            the Responses API without a public endpoint. McpGo generates the runtime
            profiles and supervises one official tunnel-client process per tunnel.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {editor.mode === "closed" ? (
            <button
              type="button"
              onClick={() => setEditor({ mode: "create" })}
              className="rounded-lg bg-sky-500 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-400"
            >
              New tunnel
            </button>
          ) : (
            <button
              type="button"
              onClick={closeEditor}
              disabled={acting}
              className="rounded-lg border border-slate-700 px-4 py-2 text-sm text-slate-300 transition hover:border-slate-500 hover:text-slate-100 disabled:cursor-not-allowed disabled:opacity-50"
            >
              Close editor
            </button>
          )}
          <button
            type="button"
            onClick={() => {
              void refresh();
            }}
            disabled={loading}
            className="rounded-lg border border-slate-700 px-4 py-2 text-sm text-slate-300 transition hover:border-sky-500/40 hover:bg-sky-500/10 hover:text-sky-300 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {loading ? "Refreshing..." : "Refresh"}
          </button>
        </div>
      </header>

      <section className="rounded-xl border border-slate-800 bg-slate-900/60 px-4 py-3">
        <div className="grid gap-x-6 gap-y-2 text-[11px] leading-5 text-slate-400 sm:grid-cols-2">
          <p className="flex flex-wrap items-baseline gap-1">
            <span className="select-none text-slate-500">tunnel-client:</span>
            {profile === null ? (
              <span className="text-slate-500">loading...</span>
            ) : profile.clientAvailable ? (
              <span className="break-all font-mono text-emerald-300" title={profile.clientPath ?? undefined}>
                {profile.clientPath}
              </span>
            ) : (
              <span className="text-amber-300">
                not found - install it into the managed bin directory
              </span>
            )}
          </p>
          <p className="flex flex-wrap items-baseline gap-1">
            <span className="select-none text-slate-500">bin:</span>
            {profile !== null ? (
              <span className="break-all font-mono text-slate-300" title={profile.managedBinDir}>
                {profile.managedBinDir}
              </span>
            ) : (
              <span className="text-slate-500">loading...</span>
            )}
          </p>
          <p className="flex flex-wrap items-baseline gap-1">
            <span className="select-none text-slate-500">health ports:</span>
            {profile !== null ? (
              <span className="font-mono text-slate-300">
                {profile.healthPortStart}..{profile.healthPortEnd}
              </span>
            ) : (
              <span className="text-slate-500">loading...</span>
            )}
          </p>
          <p className="flex flex-wrap items-baseline gap-1">
            <span className="select-none text-slate-500">manage tunnels:</span>
            {profile !== null ? (
              <span
                className="break-all font-mono text-sky-300"
                title={profile.platformTunnelsUrl}
              >
                {profile.platformTunnelsUrl}
              </span>
            ) : (
              <span className="text-slate-500">loading...</span>
            )}
          </p>
        </div>
      </section>

      {loadError !== null && (
        <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {loadError}
        </p>
      )}
      {actionError !== null && editor.mode === "closed" && (
        <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {actionError}
        </p>
      )}

      {editor.mode !== "closed" && (
        <TunnelDefinitionForm
          key={editor.mode === "edit" ? editor.summary.definition.id : "create"}
          mode={editor.mode}
          initial={editor.mode === "edit" ? editor.summary.definition : undefined}
          busy={acting}
          error={actionError}
          onSubmit={(input, id) => {
            if (editor.mode === "edit") {
              void update(editor.summary.definition.id, input).then((ok) => {
                if (ok) closeEditor();
              });
            } else {
              void create(input, id).then((ok) => {
                if (ok) closeEditor();
              });
            }
          }}
          onCancel={closeEditor}
        />
      )}

      {loading && summaries.length === 0 ? (
        <p className="px-1 py-6 text-sm text-slate-500">Loading tunnels...</p>
      ) : summaries.length === 0 ? (
        <section className="rounded-xl border border-slate-800 bg-slate-900/60 px-6 py-10 text-center text-sm text-slate-500">
          No tunnel definitions yet. Click "New tunnel" to expose your first local MCP
          server through an OpenAI Secure MCP Tunnel - create the tunnel id in the
          Platform settings, then point it at a stdio command or an HTTP endpoint.
        </section>
      ) : (
        <div className="space-y-3">
          {summaries.map((summary) => (
            <TunnelCard
              key={summary.definition.id}
              summary={summary}
              onStart={(selected) => {
                void start(selected.definition.id);
              }}
              onStop={(selected) => {
                void handleStop(selected);
              }}
              onSetKey={(selected) => {
                void handleSetKey(selected);
              }}
              onEdit={(selected) => setEditor({ mode: "edit", summary: selected })}
              onDelete={(selected) => {
                void handleDelete(selected);
              }}
              logOpen={logsOpenId === summary.definition.id}
              onToggleLogs={toggleLogs}
              busy={acting}
            />
          ))}
        </div>
      )}
    </div>
  );
}
