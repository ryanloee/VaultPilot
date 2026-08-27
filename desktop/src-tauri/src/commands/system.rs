//! System / health-check commands.

use crate::state::AppState;

/// Simple liveness probe. Returns `true` once the command dispatcher and the
/// app state (and therefore the `vaultpilot_lib` link) are healthy.
#[tauri::command]
pub async fn ping(_state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(true)
}

/// Reports whether desktop-only integrations such as the updater are
/// available. The updater plugin is not supported on Android/iOS.
#[tauri::command]
pub fn is_desktop() -> bool {
    cfg!(desktop)
}

/// Open an https URL in the system browser. Mobile builds use this to jump to
/// the GitHub release page (the in-app updater plugin is desktop-only).
/// Only `https://` URLs are accepted — never file/other schemes.
#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("only https URLs are allowed, got: {url}"));
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}
