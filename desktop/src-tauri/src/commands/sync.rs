//! Sync commands — LAN device discovery, pairing, and bidirectional sync.

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

/// Generate a pairing code for the *acceptor* to display.
#[tauri::command]
pub async fn generate_pair_code() -> Result<String, String> {
    vaultpilot_lib::sync::generate_pair_code().map_err(|e| e.to_string())
}

/// List devices this instance has paired with.
#[tauri::command]
pub async fn list_sync_peers() -> Result<Vec<vaultpilot_lib::sync::PeerDevice>, String> {
    Ok(vaultpilot_lib::sync::list_peers())
}

/// Remove a paired device by id.
#[tauri::command]
pub async fn remove_sync_peer(device_id: String) -> Result<(), String> {
    vaultpilot_lib::sync::remove_peer(&device_id).map_err(|e| e.to_string())
}

/// Complete pairing from the *initiator* side against a remote IP + code.
#[tauri::command]
pub async fn complete_pairing(
    ip: String,
    pair_code: String,
) -> Result<vaultpilot_lib::sync::PeerDevice, String> {
    vaultpilot_lib::sync::complete_pairing(&ip, &pair_code)
        .await
        .map_err(|e| e.to_string())
}

/// Bidirectionally sync with a paired device (looked up by `device_id`).
#[tauri::command]
pub async fn sync_with_peer(
    ip: String,
    device_id: String,
) -> Result<vaultpilot_lib::sync::SyncResult, String> {
    let peer = vaultpilot_lib::sync::list_peers()
        .into_iter()
        .find(|p| p.device_id == device_id)
        .ok_or_else(|| "未找到该配对设备".to_string())?;
    let target_ip = if ip.is_empty() {
        peer.ip
            .clone()
            .ok_or_else(|| "该设备无 IP 记录，请重新配对或手动指定 IP".to_string())?
    } else {
        ip
    };
    vaultpilot_lib::sync::sync_with_peer(&target_ip, &peer)
        .await
        .map_err(|e| e.to_string())
}
