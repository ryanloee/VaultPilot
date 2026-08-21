//! Sync discovery commands — probe LAN IPs for other VaultPilot instances.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use vaultpilot_lib::sync_discovery;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfoDto {
    pub hostname: String,
    pub platform: String,
    pub vault_pilot_version: String,
    pub note_count: usize,
    pub vault_name: String,
}

impl From<sync_discovery::DeviceInfo> for DeviceInfoDto {
    fn from(d: sync_discovery::DeviceInfo) -> Self {
        Self {
            hostname: d.hostname,
            platform: d.platform,
            vault_pilot_version: d.vault_pilot_version,
            note_count: d.note_count,
            vault_name: d.vault_name,
        }
    }
}

/// Probe `http://{ip}:37421/hello` for a VaultPilot discovery endpoint.
/// Returns `null` when no client is found at that address.
#[tauri::command]
pub async fn discover_device(
    _state: tauri::State<'_, AppState>,
    ip: String,
) -> Result<Option<DeviceInfoDto>, String> {
    sync_discovery::discover_device(&ip)
        .await
        .map(|opt| opt.map(DeviceInfoDto::from))
        .map_err(|e| e.to_string())
}
