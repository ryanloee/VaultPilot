//! VaultPilot desktop backend (Tauri v2).
//!
//! In-process link to [`vaultpilot_lib`] — every frontend `invoke` lands in a
//! `#[tauri::command]` here, which calls the existing storage/agent/ai
//! functions directly (zero IPC, zero subprocess).

mod commands;
mod state;

use crate::state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Initialize the shared storage context up front so that a failure
            // (e.g. config dir not writable) surfaces as a visible error rather
            // than a silent crash inside the first command.
            let state = AppState::new().expect("failed to initialize app state");
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::system::ping,
            commands::settings::get_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
