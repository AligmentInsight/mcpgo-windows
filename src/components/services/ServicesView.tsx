// Services view - the Phase 4 face of the Windows service manager (the
// port of the macOS LaunchAgents domain). Composition only: data flows
// through useServices, presentation lives in ServiceCard and
// ServiceDefinitionForm.
//
// SCM state is pulled, not pushed (no event channel exists for service
// state changes): the Refresh button re-queries, and every mutating action
// refreshes automatically through the hook.

import { useState } from "react";
import ServiceCard from "./ServiceCard";
import ServiceDefinitionForm from "./ServiceDefinitionForm";
import { useServices } from "../../hooks/useServices";
import type { ServiceSummary } from "../../types/service";

type EditorState =
  | { mode: "closed" }
  | { mode: "create" }
  | { mode: "edit"; summary: ServiceSummary };

export default function ServicesView() {
  const {
    summaries,
    loading,
    loadError,
    refresh,
    create,
    update,
    remove,
    install,
    uninstall,
    start,
    stop,
    restart,
    saving,
    actionError,
  } = useServices();
  const [editor, setEditor] = useState<EditorState>({ mode: "closed" });
  const [logsOpenId, setLogsOpenId] = useState<string | null>(null);

  const closeEditor = () => setEditor({ mode: "closed" });

  const toggleLogs = (summary: ServiceSummary) => {
    setLogsOpenId((current) =>
      current === summary.definition.id ? null : summary.definition.id,
    );
  };

  const handleDelete = async (summary: ServiceSummary) => {
    const message =
      `Delete the service definition "${summary.definition.name}"?\n\n` +
      "Only the definition file is removed; an installed Windows service is left untouched.";
    if (!window.confirm(message)) return;
    const done = await remove(summary.definition.id);
    // Close the editor only on success and only if it was editing the
    // deleted definition.
    if (done) {
      setEditor((current) =>
        current.mode === "edit" &&
        current.summary.definition.id === summary.definition.id
          ? { mode: "closed" }
          : current,
      );
      setLogsOpenId((current) =>
        current === summary.definition.id ? null : current,
      );
    }
  };

  const handleUninstall = async (summary: ServiceSummary) => {
    const message =
      `Uninstall the Windows service "${summary.serviceName}"?\n\n` +
      "The service is stopped and removed from the Service Control Manager. " +
      "The definition file stays and can be reinstalled later.\n\n" +
      "A UAC prompt appears; the action blocks until the elevated script finishes.";
    if (!window.confirm(message)) return;
    await uninstall(summary.definition.id);
  };

  return (
    <div className="space-y-4">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-slate-100">Services</h1>
          <p className="text-xs text-slate-400">
            Definitions port of the macOS LaunchAgents; each installs as a Windows service
            (SCM) hosting the McpGo service worker. Install, start, stop and uninstall
            operations show a UAC prompt and block until they finish.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {editor.mode === "closed" ? (
            <button
              type="button"
              onClick={() => setEditor({ mode: "create" })}
              className="rounded-lg bg-sky-500 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-400"
            >
              New service
            </button>
          ) : (
            <button
              type="button"
              onClick={closeEditor}
              disabled={saving}
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
        <ServiceDefinitionForm
          key={editor.mode === "edit" ? editor.summary.definition.id : "create"}
          mode={editor.mode}
          initial={editor.mode === "edit" ? editor.summary.definition : undefined}
          busy={saving}
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
        <p className="px-1 py-6 text-sm text-slate-500">Loading services...</p>
      ) : summaries.length === 0 ? (
        <section className="rounded-xl border border-slate-800 bg-slate-900/60 px-6 py-10 text-center text-sm text-slate-500">
          No service definitions yet. Click "New service" to define the first background
          service - it is stored as a JSON file next to this application&apos;s data, ready
          to be installed as a Windows service.
        </section>
      ) : (
        <div className="space-y-3">
          {summaries.map((summary) => (
            <ServiceCard
              key={summary.definition.id}
              summary={summary}
              onEdit={(selected) => setEditor({ mode: "edit", summary: selected })}
              onDelete={(selected) => {
                void handleDelete(selected);
              }}
              onInstall={(selected) => {
                void install(selected.definition.id);
              }}
              onUninstall={(selected) => {
                void handleUninstall(selected);
              }}
              onStart={(selected) => {
                void start(selected.definition.id);
              }}
              onStop={(selected) => {
                void stop(selected.definition.id);
              }}
              onRestart={(selected) => {
                void restart(selected.definition.id);
              }}
              logOpen={logsOpenId === summary.definition.id}
              onToggleLogs={toggleLogs}
              busy={saving}
            />
          ))}
        </div>
      )}
    </div>
  );
}
