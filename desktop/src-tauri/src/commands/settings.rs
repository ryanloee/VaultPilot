//! Settings commands — thin wrappers over `vaultpilot_lib::storage`.

use crate::state::AppState;
use vaultpilot_lib::models::AppSettings;
use vaultpilot_lib::storage::{initialize_storage_async, save_settings_async};

/// Reads (initializing if absent) the persisted `AppSettings` from
/// `settings.json`. Mirrors the sidecar agent's `getSettings` RPC.
///
/// NOTE: keys are masked on the agent side; here we return the unmasked view
/// because the desktop UI needs the full provider list to be editable.
#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings, String> {
    initialize_storage_async(&state.storage)
        .await
        .map_err(|e| e.to_string())
}

/// Persists `settings` to `settings.json` and returns the saved value.
#[tauri::command]
pub async fn save_settings(
    state: tauri::State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    save_settings_async(&state.storage, settings)
        .await
        .map_err(|e| e.to_string())
}
