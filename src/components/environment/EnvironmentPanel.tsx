// ============================================================================
// Environment status panel - the Phase 2 view of the application.
// ----------------------------------------------------------------------------
// Fetches the report cached by the Rust backend at startup (see the
// `environment_report` command in src-tauri/src/main.rs) and renders:
//  * one StatusRow per detected component;
//  * a summary badge (X / Y detected);
//  * the application directories used by later phases (job_status,
//    job_logs, service definitions).
// ============================================================================

import { useMemo } from "react";
import Badge from "../common/Badge";
import StatusRow from "./StatusRow";
import { useEnvironmentReport } from "../../hooks/useEnvironmentReport";
import type { ComponentCheck, EnvironmentReport } from "../../types/environment";

/** Display order of the component checks. */
function listChecks(report: EnvironmentReport): ComponentCheck[] {
  return [
    report.powershell51,
    report.powershell7,
    report.chromeExe,
    report.chromeUserData,
    report.nodeJs,
    report.python,
  ];
}

/** Application directories reported by the backend. */
function listDirectories(
  report: EnvironmentReport,
): Array<{ label: string; path: string }> {
  return [
    { label: "Job status files (job_status)", path: report.jobsStatusDir },
    { label: "Job log files (job_logs)", path: report.jobsLogsDir },
    { label: "Service definitions", path: report.servicesDir },
  ];
}

export default function EnvironmentPanel() {
  const { report, error, loading } = useEnvironmentReport();

  const checks = useMemo(() => (report ? listChecks(report) : []), [report]);
  const directories = useMemo(
    () => (report ? listDirectories(report) : []),
    [report],
  );
  const detected = checks.filter((check) => check.available).length;

  return (
    <section className="rounded-xl border border-slate-800 bg-slate-900/60">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-800 px-5 py-4">
        <div>
          <h2 className="text-sm font-semibold text-slate-100">System environment</h2>
          <p className="mt-0.5 text-xs text-slate-400">
            Detected once at application startup by the Rust backend.
          </p>
        </div>
        {!loading && error === null && (
          <Badge tone={detected === checks.length ? "ok" : "warn"}>
            {detected} / {checks.length} detected
          </Badge>
        )}
      </header>

      {loading && (
        <p className="px-5 py-6 text-sm text-slate-400">
          Reading system report...
        </p>
      )}

      {!loading && error !== null && (
        <div className="m-5 rounded-lg border border-red-500/30 bg-red-500/10 px-4 py-3">
          <p className="text-sm font-medium text-red-300">
            Failed to read the environment report
          </p>
          <p className="mt-1 font-mono text-xs text-red-200/80">{error}</p>
          <p className="mt-1 text-xs text-slate-400">
            The IPC bridge is only available inside the Tauri window
            (npm run tauri dev).
          </p>
        </div>
      )}

      {!loading && error === null && report !== null && (
        <>
          <ul className="divide-y divide-slate-800/70 px-5">
            {checks.map((check) => (
              <StatusRow key={check.id} check={check} />
            ))}
          </ul>
          <footer className="border-t border-slate-800 px-5 py-4">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-500">
              Application directories
            </h3>
            <ul className="mt-2 space-y-1.5">
              {directories.map((dir) => (
                <li
                  key={dir.path}
                  className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 text-xs"
                >
                  <span className="min-w-56 shrink-0 text-slate-400">{dir.label}</span>
                  <code className="break-all font-mono text-slate-300">{dir.path}</code>
                </li>
              ))}
            </ul>
          </footer>
        </>
      )}
    </section>
  );
}
