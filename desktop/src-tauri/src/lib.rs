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
            // system
            commands::system::ping,
            // settings
            commands::settings::get_settings,
            commands::settings::save_settings,
            // notes
            commands::notes::list_notes,
            commands::notes::load_note,
            commands::notes::save_note,
            commands::notes::delete_note,
            commands::notes::find_related_notes,
            commands::notes::import_markdown,
            commands::notes::rebuild_index,
            commands::notes::read_image_preview,
            commands::notes::open_vault_directory,
            commands::notes::list_snapshots,
            commands::notes::get_snapshot,
            commands::notes::restore_snapshot,
            // chat
            commands::chat::load_chat_state,
            commands::chat::save_chat_state,
            commands::chat::list_actions,
            commands::chat::execute_ai_action_cmd,
            commands::chat::compress_chat_history,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
