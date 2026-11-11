// One row of the environment panel: status dot, component label, badges,
// detected path (monospace, truncated with a full-path tooltip) and the
// explanatory note produced by the Rust backend.

import Badge from "../common/Badge";
import type { ComponentCheck } from "../../types/environment";

export interface StatusRowProps {
  check: ComponentCheck;
}

export default function StatusRow({ check }: StatusRowProps) {
  const available = check.available;

  const dotClass = available
    ? "bg-emerald-400"
    : check.required
      ? "bg-red-400"
      : "bg-amber-400";

  return (
    <li className="flex items-start gap-3 py-3">
      <span
        aria-hidden="true"
        className={`mt-1.5 h-2.5 w-2.5 shrink-0 rounded-full ${dotClass}`}
      />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium text-slate-100">{check.label}</span>
          <Badge tone={available ? "ok" : check.required ? "error" : "warn"}>
            {available ? "Detected" : check.required ? "Missing" : "Not found"}
          </Badge>
          <Badge
            tone="neutral"
            title={
              check.required
                ? "McpGo cannot operate without this component"
                : "McpGo works without this component; the related module stays disabled"
            }
          >
            {check.required ? "Required" : "Optional"}
          </Badge>
        </div>
        <p
          className="mt-0.5 truncate font-mono text-xs text-slate-500"
          title={check.path ?? undefined}
        >
          {check.path ?? "not found"}
        </p>
        <p className="mt-0.5 text-xs text-slate-400">{check.note}</p>
      </div>
    </li>
  );
}
