//! VaultPilot desktop backend (Tauri v2).
//!
//! In-process link to [`vaultpilot_lib`] — every frontend `invoke` lands in a
//! `#[tauri::command]` here, which calls the existing storage/agent/ai
//! functions directly (zero IPC, zero subprocess).

mod commands;
mod state;

use crate::state::AppState;
use std::sync::Arc;
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem};
#[cfg(desktop)]
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init());

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    builder
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
            app.manage(state.clone());
            #[cfg(desktop)]
            let scheduler_storage = state.storage.clone();
            let discovery_storage = state.storage.clone();

            // Spawn the trigger-rule scheduler so cron rules actually fire in
            // the desktop app. The rules UI only persists rows in
            // `trigger_rules` — without this loop nothing ever evaluates
            // them (previously only the CLI's `trigger start` command ran an
            // executor, so rules created in the app silently never fired).
            // It runs for the whole app session on the Tauri async runtime;
            // close-to-tray keeps the process (and thus the scheduler) alive.
            // Mobile: skipped — the OS kills backgrounded apps anyway, so a
            // tick loop would only burn battery; rules fire again once a
            // desktop instance (or the CLI) is running.
            #[cfg(desktop)]
            tauri::async_runtime::spawn(async move {
                let executor =
                    vaultpilot_lib::orchestration::trigger_executor::TriggerExecutor::new(
                        (*scheduler_storage).clone(),
                    );
                executor.run_forever().await;
            });

            // LAN sync discovery server — listens on port 37421 and answers
            // GET /hello with device info so other VaultPilot instances on
            // the LAN can find us by IP. Non-fatal when the port is taken.
            // Pairing events are forwarded to the UI via the `sync-pairing`
            // Tauri event so the acceptor side gets a visible prompt.
            let sync_app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let storage = discovery_storage;
                let (note_count, vault_name) = {
                    let conn = match storage.get_connection() {
                        Ok(c) => c,
                        Err(_) => return,
                    };
                    let count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))
                        .unwrap_or(0);
                    (count as usize, storage.vault_dir_name())
                };
                let on_event: Arc<dyn Fn(vaultpilot_lib::sync::SyncPairingEvent) + Send + Sync> =
                    Arc::new(move |e| {
                        let _ = sync_app_handle.emit("sync-pairing", e.clone());
                        // Native OS notification (Windows toast / Android
                        // notification) so a pairing attempt is still visible
                        // when the window is hidden in the tray. When visible,
                        // the in-app banner in SyncPanel is enough.
                        let visible = sync_app_handle
                            .get_webview_window("main")
                            .map(|w| w.is_visible().unwrap_or(false))
                            .unwrap_or(false);
                        if !visible {
                            use tauri_plugin_notification::NotificationExt;
                            use vaultpilot_lib::sync::SyncPairingEvent;
                            let (title, body) = match &e {
                                SyncPairingEvent::Accepted { hostname, .. } => (
                                    "VaultPilot 配对成功".to_string(),
                                    format!("「{hostname}」已与本设备配对"),
                                ),
                                SyncPairingEvent::Rejected { reason } => {
                                    ("VaultPilot 配对被拒绝".to_string(), reason.clone())
                                }
                            };
                            let _ = sync_app_handle
                                .notification()
                                .builder()
                                .title(title)
                                .body(body)
                                .show();
                        }
                    });
                vaultpilot_lib::sync::start_sync_server(note_count, vault_name, on_event).await;
            });

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
            commands::system::is_desktop,
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
            commands::notes::vault_sync_status,
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
            // triggers
            commands::triggers::list_trigger_rules,
            commands::triggers::list_trigger_executions,
            commands::triggers::create_trigger_rule,
            commands::triggers::update_trigger_rule,
            commands::triggers::fire_trigger_rule_now,
            commands::triggers::toggle_trigger_rule,
            commands::triggers::delete_trigger_rule,
            commands::triggers::delete_trigger_execution,
            commands::triggers::clear_trigger_executions,
            // sync
            commands::sync::discover_device,
            commands::sync::scan_lan_devices,
            commands::sync::generate_pair_code,
            commands::sync::regenerate_pair_code,
            commands::sync::list_sync_peers,
            commands::sync::remove_sync_peer,
            commands::sync::complete_pairing,
            commands::sync::sync_with_peer,
            commands::sync::get_peer_manifest,
            commands::sync::list_local_manifest,
            commands::sync::sync_selected,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
