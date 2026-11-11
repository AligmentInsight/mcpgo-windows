// One service in the services view: the definition summary joined with the
// SCM status badge, the runtime chip, warnings and the available actions:
//   not installed  -> Install, Logs, Edit, Delete
//   installed      -> Start (Stopped) / Stop + Restart (Running),
//                     Uninstall, Logs, Edit, Delete
// The log panel mounts inside the card while its `logOpen` flag is set
// (owned by ServicesView); data flow only - all shared state lives in
// ServicesView.

import Badge from "../common/Badge";
import type { ServiceSummary } from "../../types/service";
import ServiceLogPanel from "./ServiceLogPanel";
import { RUNTIME_META, scmStatusMeta } from "./serviceStyle";

/** Tailwind classes of the secondary (bordered) action buttons. */
const BUTTON_SECONDARY =
  "rounded-lg border border-slate-700 px-3 py-1 text-xs text-slate-300 transition disabled:cursor-not-allowed disabled:opacity-50";
/** Tailwind classes of the install action (the primary call to action). */
const BUTTON_INSTALL =
  "rounded-lg bg-sky-500 px-3 py-1 text-xs font-semibold text-slate-950 transition hover:bg-sky-400 disabled:cursor-not-allowed disabled:opacity-50";
/** Hover accent appended to the secondary buttons per action kind. */
const HOVER_NEUTRAL = "hover:border-slate-500 hover:text-slate-100";
const HOVER_SKY = "hover:border-sky-500/40 hover:bg-sky-500/10 hover:text-sky-300";
const HOVER_AMBER =
  "hover:border-amber-500/40 hover:bg-amber-500/10 hover:text-amber-300";
const HOVER_RED = "hover:border-red-500/40 hover:bg-red-500/10 hover:text-red-300";

export interface ServiceCardProps {
  summary: ServiceSummary;
  onEdit: (summary: ServiceSummary) => void;
  onDelete: (summary: ServiceSummary) => void;
  onInstall: (summary: ServiceSummary) => void;
  onUninstall: (summary: ServiceSummary) => void;
  onStart: (summary: ServiceSummary) => void;
  onStop: (summary: ServiceSummary) => void;
  onRestart: (summary: ServiceSummary) => void;
  /** Whether this service's log panel is open (owned by ServicesView). */
  logOpen: boolean;
  /** Opens/closes this service's log panel. */
  onToggleLogs: (summary: ServiceSummary) => void;
  /** Disables the actions while a mutating action is in flight. */
  busy: boolean;
}

export default function ServiceCard({
  summary,
  onEdit,
  onDelete,
  onInstall,
  onUninstall,
  onStart,
  onStop,
  onRestart,
  logOpen,
  onToggleLogs,
  busy,
}: ServiceCardProps) {
  const { definition } = summary;
  const runtime = RUNTIME_META[definition.runtime];
  const scm = scmStatusMeta(summary.installed, summary.scmStatus);
  const commandLine = [definition.command, ...definition.args].join(" ");
  const isRunning = summary.scmStatus === "Running";
  const isStopped = summary.scmStatus === "Stopped";

  return (
    <article className="rounded-xl border border-slate-800 bg-slate-900/60">
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800 px-4 py-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span aria-hidden="true" className={`h-2 w-2 shrink-0 rounded-full ${scm.dot}`} />
          <span className="truncate text-sm font-semibold text-slate-100">
            {definition.name}
          </span>
          <Badge tone={scm.tone}>{scm.label}</Badge>
          {summary.installed && definition.autostart && (
            <span
              className="text-[11px] text-slate-500"
              title="The service starts automatically at system boot"
            >
              auto-start
            </span>
          )}
          {summary.installed && definition.keepAlive && (
            <span
              className="text-[11px] text-slate-500"
              title="The service worker restarts the runtime process after failures"
            >
              keep-alive
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <span
            className={`rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${runtime.chipClass}`}
            title={
              definition.runtimePath !== null
                ? `Runtime executable: ${definition.runtimePath}`
                : undefined
            }
          >
            {runtime.label}
            {definition.runtimePath !== null ? " (path)" : ""}
          </span>
          {!summary.installed ? (
            <button
              type="button"
              onClick={() => onInstall(summary)}
              disabled={busy}
              title="Register this definition as a Windows service (UAC prompt)"
              className={BUTTON_INSTALL}
            >
              Install
            </button>
          ) : (
            <>
              {isStopped && (
                <button
                  type="button"
                  onClick={() => onStart(summary)}
                  disabled={busy}
                  title="Start the service (UAC prompt)"
                  className={`${BUTTON_SECONDARY} ${HOVER_SKY}`}
                >
                  Start
                </button>
              )}
              {isRunning && (
                <>
                  <button
                    type="button"
                    onClick={() => onStop(summary)}
                    disabled={busy}
                    title="Stop the service and its runtime process (UAC prompt)"
                    className={`${BUTTON_SECONDARY} ${HOVER_AMBER}`}
                  >
                    Stop
                  </button>
                  <button
                    type="button"
                    onClick={() => onRestart(summary)}
                    disabled={busy}
                    title="Stop and start the service (UAC prompt)"
                    className={`${BUTTON_SECONDARY} ${HOVER_SKY}`}
                  >
                    Restart
                  </button>
                </>
              )}
              <button
                type="button"
                onClick={() => onUninstall(summary)}
                disabled={busy}
                title="Stop and delete the Windows service; the definition stays (UAC prompt)"
                className={`${BUTTON_SECONDARY} ${HOVER_AMBER}`}
              >
                Uninstall
              </button>
            </>
          )}
          <button
            type="button"
            onClick={() => onToggleLogs(summary)}
            disabled={busy}
            aria-expanded={logOpen}
            title={
              logOpen
                ? "Close the service log panel"
                : "Open the service log panel (refreshed every 2 s while open)"
            }
            className={`${BUTTON_SECONDARY} ${HOVER_SKY}`}
          >
            {logOpen ? "Hide logs" : "Logs"}
          </button>
          <button
            type="button"
            onClick={() => onEdit(summary)}
            disabled={busy}
            className={`${BUTTON_SECONDARY} ${HOVER_NEUTRAL}`}
          >
            Edit
          </button>
          <button
            type="button"
            onClick={() => onDelete(summary)}
            disabled={busy}
            className={`${BUTTON_SECONDARY} ${HOVER_RED}`}
          >
            Delete
          </button>
        </div>
      </header>

      <div className="space-y-2 px-4 py-3">
        <p className="text-xs text-slate-400">
          {definition.description.length > 0 ? definition.description : "No description."}
        </p>
        <p className="break-all font-mono text-[11px] leading-5 text-slate-300">
          <span className="mr-1 select-none text-slate-500">{definition.runtime}</span>
          {commandLine}
        </p>
        {definition.runtimePath !== null && (
          <p className="break-all font-mono text-[11px] leading-5 text-slate-400">
            <span className="mr-1 select-none text-slate-500">runtime</span>
            {definition.runtimePath}
          </p>
        )}
        {!summary.runtimeAvailable && definition.runtimePath === null && (
          <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-300">
            {runtime.label} was not found on this machine - set an absolute runtime path in
            the definition or install the runtime before starting the service.
          </p>
        )}
        <p className="text-[11px] text-slate-500">
          <span className="font-mono">{summary.serviceName}</span>
          <span aria-hidden="true"> - </span>
          <span className="font-mono" title={summary.definitionPath}>
            {definition.id}.json
          </span>
        </p>
      </div>

      {logOpen && (
        <div className="border-t border-slate-800 px-4 py-3">
          <ServiceLogPanel summary={summary} onClose={() => onToggleLogs(summary)} />
        </div>
      )}
    </article>
  );
}
