// One tunnel in the tunnels view: the definition summary joined with the
// runtime state badge, the target chip, warnings and the available
// actions:
//   stopped/failed -> Start, Set key (managed), Logs, Edit, Delete
//   starting/running -> Stop, Logs, Edit, Delete (disabled - stop first)
// The log panel mounts inside the card while its `logOpen` flag is set
// (owned by TunnelsView); data flow only - all shared state lives in
// TunnelsView.

import Badge from "../common/Badge";
import type { TunnelSummary } from "../../types/tunnel";
import TunnelLogPanel from "./TunnelLogPanel";
import { API_KEY_SOURCE_META, TARGET_META, tunnelStateMeta } from "./tunnelStyle";

/** Tailwind classes of the secondary (bordered) action buttons. */
const BUTTON_SECONDARY =
  "rounded-lg border border-slate-700 px-3 py-1 text-xs text-slate-300 transition disabled:cursor-not-allowed disabled:opacity-50";
/** Tailwind classes of the start action (the primary call to action). */
const BUTTON_START =
  "rounded-lg bg-sky-500 px-3 py-1 text-xs font-semibold text-slate-950 transition hover:bg-sky-400 disabled:cursor-not-allowed disabled:opacity-50";
/** Hover accent appended to the secondary buttons per action kind. */
const HOVER_NEUTRAL = "hover:border-slate-500 hover:text-slate-100";
const HOVER_SKY = "hover:border-sky-500/40 hover:bg-sky-500/10 hover:text-sky-300";
const HOVER_RED = "hover:border-red-500/40 hover:bg-red-500/10 hover:text-red-300";

export interface TunnelCardProps {
  summary: TunnelSummary;
  onStart: (summary: TunnelSummary) => void;
  onStop: (summary: TunnelSummary) => void;
  onSetKey: (summary: TunnelSummary) => void;
  onEdit: (summary: TunnelSummary) => void;
  onDelete: (summary: TunnelSummary) => void;
  /** Whether this tunnel's log panel is open (owned by TunnelsView). */
  logOpen: boolean;
  /** Opens/closes this tunnel's log panel. */
  onToggleLogs: (summary: TunnelSummary) => void;
  /** Disables the actions while a mutating action is in flight. */
  busy: boolean;
}

export default function TunnelCard({
  summary,
  onStart,
  onStop,
  onSetKey,
  onEdit,
  onDelete,
  logOpen,
  onToggleLogs,
  busy,
}: TunnelCardProps) {
  const { definition, status } = summary;
  const meta = tunnelStateMeta(status.state);
  const target = TARGET_META[definition.targetKind];
  const keySource = API_KEY_SOURCE_META[definition.apiKey.kind];
  const isActive = status.state === "starting" || status.state === "running";
  const commandLine =
    definition.targetKind === "stdio"
      ? [definition.command ?? "", ...definition.args].join(" ").trim()
      : definition.serverUrl ?? "";

  return (
    <article className="rounded-xl border border-slate-800 bg-slate-900/60">
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800 px-4 py-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span aria-hidden="true" className={`h-2 w-2 shrink-0 rounded-full ${meta.dot}`} />
          <span className="truncate text-sm font-semibold text-slate-100">
            {definition.name}
          </span>
          <Badge tone={meta.tone}>{meta.label}</Badge>
          {definition.autostart && (
            <span
              className="text-[11px] text-slate-500"
              title="The runtime starts automatically with McpGo"
            >
              auto-start
            </span>
          )}
          {definition.keepAlive && (
            <span
              className="text-[11px] text-slate-500"
              title="The supervisor restarts the runtime after unexpected exits"
            >
              keep-alive
            </span>
          )}
          {status.restarts > 0 && (
            <span
              className="text-[11px] text-amber-400/80"
              title="Automatic restarts performed by keep-alive during this run"
            >
              {status.restarts} restart{status.restarts === 1 ? "" : "s"}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <span
            className={`rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${target.chipClass}`}
            title={
              definition.targetKind === "stdio"
                ? "The runtime spawns the MCP server as its child process"
                : "The runtime forwards to a streamable-HTTP MCP server"
            }
          >
            {target.label}
          </span>
          {!isActive ? (
            <button
              type="button"
              onClick={() => onStart(summary)}
              disabled={busy}
              title="Generate the profile, spawn the tunnel-client runtime and supervise it"
              className={BUTTON_START}
            >
              Start
            </button>
          ) : (
            <button
              type="button"
              onClick={() => onStop(summary)}
              disabled={busy}
              title="Terminate the runtime process (the supervisor finalizes the tunnel as stopped)"
              className={`${BUTTON_SECONDARY} hover:border-amber-500/40 hover:bg-amber-500/10 hover:text-amber-300`}
            >
              Stop
            </button>
          )}
          {definition.apiKey.kind === "managed" && !isActive && (
            <button
              type="button"
              onClick={() => onSetKey(summary)}
              disabled={busy}
              title="Store the runtime API key in McpGo's secrets directory (never echoed back)"
              className={`${BUTTON_SECONDARY} ${HOVER_SKY}`}
            >
              Set key
            </button>
          )}
          <button
            type="button"
            onClick={() => onToggleLogs(summary)}
            disabled={busy}
            aria-expanded={logOpen}
            title={
              logOpen
                ? "Close the runtime log panel"
                : "Open the runtime log panel (refreshed every 2 s while open)"
            }
            className={`${BUTTON_SECONDARY} ${HOVER_SKY}`}
          >
            {logOpen ? "Hide logs" : "Logs"}
          </button>
          <button
            type="button"
            onClick={() => onEdit(summary)}
            disabled={busy || isActive}
            title={
              isActive
                ? "Stop the runtime first - the running process keeps the previous profile"
                : "Edit the definition"
            }
            className={`${BUTTON_SECONDARY} ${HOVER_NEUTRAL}`}
          >
            Edit
          </button>
          <button
            type="button"
            onClick={() => onDelete(summary)}
            disabled={busy || isActive}
            title={
              isActive
                ? "Stop the runtime first"
                : "Delete the definition, the generated profile and the managed key (the log stays)"
            }
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
          <span className="mr-1 select-none text-slate-500">tunnel</span>
          {definition.tunnelId}
        </p>
        <p className="break-all font-mono text-[11px] leading-5 text-slate-300">
          <span className="mr-1 select-none text-slate-500">
            {definition.targetKind === "stdio" ? "mcp" : "url"}
          </span>
          {commandLine}
        </p>
        {status.state === "running" && status.uiUrl !== null && (
          <p className="break-all font-mono text-[11px] leading-5 text-emerald-300/90">
            <span className="mr-1 select-none text-slate-500">ui</span>
            {status.uiUrl}
            <span className="ml-2 text-slate-500">
              (open in a browser while the runtime is up)
            </span>
          </p>
        )}
        {status.error !== null && (
          <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
            {status.error}
          </p>
        )}
        {!summary.clientAvailable && (
          <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-300">
            tunnel-client.exe was not found - install it into the managed bin
            directory shown in the view header, add it to PATH, or set an explicit
            client path on this tunnel.
          </p>
        )}
        {!summary.apiKeyReady && (
          <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-300">
            {definition.apiKey.kind === "managed"
              ? "No managed API key stored yet - use the Set key action before starting."
              : `The ${keySource.label.toLowerCase()} (${definition.apiKey.reference ?? "unset"}) is not available - a start would fail its readiness check.`}
          </p>
        )}
        <p className="text-[11px] text-slate-500">
          <span className="font-mono">{definition.residency}</span>
          <span aria-hidden="true"> - </span>
          <span className="font-mono" title={summary.profilePath}>
            {definition.id}.yaml
          </span>
          {definition.healthPort !== null && (
            <>
              <span aria-hidden="true"> - </span>
              <span className="font-mono" title="Pinned health-listener port">
                :{definition.healthPort}
              </span>
            </>
          )}
        </p>
      </div>

      {logOpen && (
        <div className="border-t border-slate-800 px-4 py-3">
          <TunnelLogPanel summary={summary} onClose={() => onToggleLogs(summary)} />
        </div>
      )}
    </article>
  );
}
