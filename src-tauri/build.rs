//! Tauri build script for McpGo-Windows.
//!
//! Validates `tauri.conf.json`, embeds the application icons declared in the
//! `bundle.icon` list and generates the permission schemas and asset
//! artifacts required by `capabilities/default.json` at compile time.

fn main() {
    tauri_build::build()
}
