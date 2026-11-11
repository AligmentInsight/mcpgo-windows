//! IPC command modules. Each domain lives in its own file; `main.rs`
//! registers every command in `generate_handler!`.
//!
//! Conventions:
//! * command parameters are `snake_case` in Rust and are addressed with
//!   `camelCase` keys from the frontend (Tauri maps them automatically);
//! * handlers stay thin: validation and logic live in the domain modules
//!   (`jobs`, `terminal`, `browser`, ...), commands only adapt the IPC
//!   boundary.

pub mod browser;
pub mod services;
pub mod terminal;
pub mod tunnels;
