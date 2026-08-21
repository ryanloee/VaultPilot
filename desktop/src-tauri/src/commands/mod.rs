//! Tauri command modules. Each file groups related `#[tauri::command]`s that
//! thinly wrap the corresponding `vaultpilot_lib` async functions.

pub mod chat;
pub mod collections;
pub mod notes;
pub mod settings;
pub mod sync;
pub mod system;
pub mod triggers;
