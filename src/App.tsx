// ============================================================================
// McpGo for Windows - application shell
// ----------------------------------------------------------------------------
// Renders the global layout: the header (TitleBar), the sidebar navigation
// and the active view. Phase 3 added the Terminal view (PowerShell jobs),
// Phase 4 the Services view (LaunchAgents port), Phase 5 the Browser view
// (Chrome/CDP sessions), Phase 6 the Tunnels view (OpenAI Secure MCP
// Tunnels) together with this full sidebar navigation - the domain set of
// the roadmap is now complete, so the tab bar of the earlier phases
// graduated into the permanent left rail.
// ============================================================================

import { useState } from "react";
import TitleBar from "./components/layout/TitleBar";
import EnvironmentPanel from "./components/environment/EnvironmentPanel";
import BrowserView from "./components/browser/BrowserView";
import ServicesView from "./components/services/ServicesView";
import TerminalView from "./components/terminal/TerminalView";
import TunnelsView from "./components/tunnels/TunnelsView";

type ViewId = "terminal" | "browser" | "tunnels" | "services" | "environment";

const VIEWS: Array<{ id: ViewId; label: string; hint: string }> = [
  { id: "terminal", label: "Terminal", hint: "PowerShell jobs (job_status / job_logs)" },
  { id: "browser", label: "Browser", hint: "Chrome/CDP debug sessions" },
  { id: "tunnels", label: "Tunnels", hint: "OpenAI Secure MCP Tunnels" },
  { id: "services", label: "Services", hint: "Windows services (LaunchAgents port)" },
  { id: "environment", label: "Environment", hint: "Machine diagnostics" },
];

export default function App() {
  const [view, setView] = useState<ViewId>("terminal");

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100">
      <TitleBar />
      <div className="mx-auto flex max-w-5xl">
        <nav
          aria-label="Application views"
          className="w-44 shrink-0 border-r border-slate-800 px-3 py-6"
        >
          <ul className="space-y-1">
            {VIEWS.map((entry) => (
              <li key={entry.id}>
                <button
                  type="button"
                  onClick={() => setView(entry.id)}
                  aria-pressed={view === entry.id}
                  aria-current={view === entry.id ? "page" : undefined}
                  title={entry.hint}
                  className={`w-full rounded-lg border px-3 py-2 text-left text-sm font-medium transition ${
                    view === entry.id
                      ? "border-sky-500/40 bg-sky-500/10 text-sky-300"
                      : "border-transparent text-slate-400 hover:bg-slate-900 hover:text-slate-200"
                  }`}
                >
                  {entry.label}
                </button>
              </li>
            ))}
          </ul>
        </nav>
        <main className="min-w-0 flex-1 px-6 py-6">
          {view === "terminal" ? (
            <TerminalView />
          ) : view === "browser" ? (
            <BrowserView />
          ) : view === "tunnels" ? (
            <TunnelsView />
          ) : view === "services" ? (
            <ServicesView />
          ) : (
            <EnvironmentPanel />
          )}
        </main>
      </div>
    </div>
  );
}
