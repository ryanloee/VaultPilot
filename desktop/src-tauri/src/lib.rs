//! VaultPilot desktop backend (Tauri v2).
//!
//! In-process link to [`vaultpilot_lib`] — every frontend `invoke` lands in a
//! `#[tauri::command]` here, which calls the existing storage/agent/ai
//! functions directly (zero IPC, zero subprocess).

mod commands;
mod state;

use crate::state::AppState;
use tauri::{Manager, WindowEvent};
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem};
#[cfg(desktop)]
use tauri::tray::TrayIconBuilder;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // On mobile (Android/iOS) the OS does not set the APPDATA /
            // LOCALAPPDATA / HOME environment variables that
            // `StorageContext::for_sidecar()` relies on. Bridge them to the
            // Tauri-resolved app directories so settings.json, chat state and
            // the SQLite vault land in the app's own data dir instead of the
            // temp fallback. Desktop platforms keep their native env vars.
            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                let path = app.path();
                let set = |name: &str, dir: Option<std::path::PathBuf>| {
                    if let Some(dir) = dir {
                        std::env::set_var(name, dir);
                    }
                };
                set("APPDATA", path.app_config_dir().ok());
                set("LOCALAPPDATA", path.app_local_data_dir().ok());
                set("HOME", path.app_data_dir().ok());
                set("USERPROFILE", path.app_data_dir().ok());
            }
            // Initialize the shared storage context up front so that a failure
            // (e.g. config dir not writable) surfaces as a visible error rather
            // than a silent crash inside the first command.
            let state = AppState::new().expect("failed to initialize app state");
            app.manage(state);

            // System tray so the app keeps running in the background when the
            // main window is closed (close-to-tray, desktop only).
            #[cfg(desktop)]
            {
                let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show, &quit])?;
                let _tray = TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                        // Left-click on the tray icon shows the main window
                        // (right-click opens the menu above).
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Close-to-tray: hide instead of exiting so the app keeps running
            // in the background. Exit via the tray menu's "退出" item.
            #[cfg(desktop)]
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            // system
            commands::system::ping,
            // settings
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::test_provider_connection,
            // notes
            commands::notes::list_notes,
            commands::notes::load_note,
            commands::notes::save_note,
            commands::notes::delete_note,
            commands::notes::find_related_notes,
            commands::notes::find_backlinks,
            commands::notes::import_markdown,
            commands::notes::rebuild_index,
            commands::notes::read_image_preview,
            commands::notes::open_vault_directory,
            commands::notes::list_snapshots,
            commands::notes::get_snapshot,
            commands::notes::restore_snapshot,
            // collections
            commands::collections::list_collections,
            commands::collections::create_collection,
            commands::collections::rename_collection,
            commands::collections::move_collection,
            commands::collections::delete_collection,
            commands::collections::add_note_to_collection,
            commands::collections::remove_note_from_collection,
            commands::collections::list_notes_in_collection,
            commands::collections::get_collections_for_note,
            // chat
            commands::chat::load_chat_state,
            commands::chat::save_chat_state,
            commands::chat::list_actions,
            commands::chat::execute_ai_action,
            commands::chat::compress_chat_history,
            commands::chat::ask_with_ai,
            commands::chat::save_temp_attachment,
            commands::chat::transcribe_audio,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
