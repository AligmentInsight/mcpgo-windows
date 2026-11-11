// ============================================================================
// McpGo for Windows - Vite configuration (Phase 2)
// ----------------------------------------------------------------------------
// Follows the official Tauri 2 + Vite conventions:
//  * fixed port 1420 matching `build.devUrl` in src-tauri/tauri.conf.json;
//  * `TAURI_DEV_HOST` support (device/remote development);
//  * `TAURI_ENV_*` variables set by the Tauri CLI are forwarded to the
//    client bundle through the environment prefix;
//  * the src-tauri workspace is excluded from the dev-server watcher.
// ============================================================================

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Set by `tauri dev` when the dev server must be reachable over the network.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],

  // Keep the dev server output readable next to the Rust log stream.
  clearScreen: false,

  server: {
    port: 1420,
    // Fail loudly instead of silently picking another port: the Tauri
    // backend expects exactly http://localhost:1420 (tauri.conf.json).
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      // Rust backend changes go through `cargo`/`tauri dev`, not Vite.
      ignored: ["**/src-tauri/**"],
    },
  },

  // Expose Tauri CLI-provided environment variables to the client bundle.
  envPrefix: ["VITE_", "TAURI_ENV_"],

  build: {
    // WebView2 (Windows) is evergreen Chromium; the safari fallback keeps
    // the bundle sane for quick checks in an ordinary browser.
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: "esbuild",
    sourcemap: false,
  },
});
