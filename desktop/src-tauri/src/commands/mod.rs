//! Tauri command modules. Each file groups related `#[tauri::command]`s that
//! thinly wrap the corresponding `vaultpilot_lib` async functions.

pub mod chat;
pub mod notes;
pub mod settings;
pub mod system;
