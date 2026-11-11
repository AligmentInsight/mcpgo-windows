// IPC wrappers for application metadata (Tauri bundle information).

import { getVersion } from "@tauri-apps/api/app";

/** Returns the application version from tauri.conf.json. */
export function fetchAppVersion(): Promise<string> {
  return getVersion();
}
