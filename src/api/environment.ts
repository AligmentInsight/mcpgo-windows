// IPC wrapper for the `environment_report` command implemented in
// src-tauri/src/main.rs. Returns the report cached at startup.

import { invoke } from "@tauri-apps/api/core";
import type { EnvironmentReport } from "../types/environment";

export function fetchEnvironmentReport(): Promise<EnvironmentReport> {
  return invoke<EnvironmentReport>("environment_report");
}
