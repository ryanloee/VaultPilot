//! Settings commands — thin wrappers over `vaultpilot_lib::storage`.

use crate::state::AppState;
use vaultpilot_lib::ai::connectivity::{
    check_provider_connection, CheckProviderConnectionParams, ProviderConnectionResult,
};
use vaultpilot_lib::models::AppSettings;
use vaultpilot_lib::storage::{initialize_storage_async, save_settings_async};

/// Reads (initializing if absent) the persisted `AppSettings` from
/// `settings.json`. Mirrors the sidecar agent's `getSettings` RPC.
///
/// NOTE: keys are masked on the agent side; here we return the unmasked view
/// because the desktop UI needs the full provider list to be editable.
#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings, String> {
    let start = std::time::Instant::now();
    let result = initialize_storage_async(&state.storage)
        .await
        .map_err(|e| e.to_string());
    eprintln!("[timing] get_settings took {:?}", start.elapsed());
    result
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

/// Probes the given provider endpoint (GET /models or /api/tags) so the
/// settings UI can validate the configuration before saving (#3480).
///
/// The caller passes the freshly typed fields (never the masked stored key).
#[tauri::command]
pub async fn test_provider_connection(
    api_base: String,
    api_key: String,
    provider_type: Option<String>,
    model: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<ProviderConnectionResult, String> {
    let params = CheckProviderConnectionParams {
        api_base,
        api_key,
        provider_type: provider_type.unwrap_or_default(),
        model,
        timeout_ms,
    };
    Ok(check_provider_connection(&params).await)
}
