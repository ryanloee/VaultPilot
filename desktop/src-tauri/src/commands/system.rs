//! System / health-check commands.

use crate::state::AppState;

/// Simple liveness probe. Returns `true` once the command dispatcher and the
/// app state (and therefore the `vaultpilot_lib` link) are healthy.
#[tauri::command]
pub async fn ping(_state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(true)
}
