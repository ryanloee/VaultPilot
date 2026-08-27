//! Sync commands — LAN device discovery, pairing, and bidirectional sync.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use vaultpilot_lib::storage::rebuild_index_async;
use vaultpilot_lib::sync_discovery;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfoDto {
    pub device_id: String,
    pub hostname: String,
    pub platform: String,
    pub vault_pilot_version: String,
    pub note_count: usize,
    pub vault_name: String,
}

impl From<sync_discovery::DeviceInfo> for DeviceInfoDto {
    fn from(d: sync_discovery::DeviceInfo) -> Self {
        Self {
            device_id: d.device_id,
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

/// A device found by the LAN scan, with its network address.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedDeviceDto {
    pub ip: String,
    pub device_id: String,
    pub hostname: String,
    pub platform: String,
    pub vault_pilot_version: String,
    pub note_count: usize,
    pub vault_name: String,
}

/// Scan the local /24 subnet for other VaultPilot instances.
#[tauri::command]
pub async fn scan_lan_devices() -> Result<Vec<ScannedDeviceDto>, String> {
    Ok(vaultpilot_lib::sync_discovery::scan_lan()
        .await
        .into_iter()
        .map(|(ip, d)| ScannedDeviceDto {
            ip,
            device_id: d.device_id,
            hostname: d.hostname,
            platform: d.platform,
            vault_pilot_version: d.vault_pilot_version,
            note_count: d.note_count,
            vault_name: d.vault_name,
        })
        .collect())
}

/// Generate a pairing code for the *acceptor* to display (idempotent — returns
/// the existing valid code if one is still live).
#[tauri::command]
pub async fn generate_pair_code() -> Result<String, String> {
    vaultpilot_lib::sync::generate_pair_code().map_err(|e| e.to_string())
}

/// Force a fresh pairing code (invalidates any prior code). Used by the UI's
/// "重新生成" button.
#[tauri::command]
pub async fn regenerate_pair_code() -> Result<String, String> {
    vaultpilot_lib::sync::regenerate_pair_code().map_err(|e| e.to_string())
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
///
/// `mode` is `"full"` or `"selected"`; when `"selected"`, only paths under one
/// of the `includes` folder/file prefixes are transferred.
///
/// On success the search index is rebuilt automatically — sync writes raw
/// markdown files and `list_notes` reads the index, so without this the notes
/// UI stays stale/empty until a manual rebuild.
#[tauri::command]
pub async fn sync_with_peer(
    state: tauri::State<'_, AppState>,
    ip: String,
    device_id: String,
    mode: Option<String>,
    includes: Option<Vec<String>>,
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
    let mode = mode.unwrap_or_else(|| "full".to_string());
    let includes = includes.unwrap_or_default();
    let result = vaultpilot_lib::sync::sync_with_peer(&target_ip, &peer, &mode, &includes)
        .await
        .map_err(|e| e.to_string())?;
    // Best-effort: keep the index in step with newly pulled files.
    let _ = rebuild_index_async(&state.storage).await;
    Ok(result)
}

/// The paired peer's vault manifest — the "download from peer" side of the
/// selective-sync picker.
#[tauri::command]
pub async fn get_peer_manifest(
    ip: String,
) -> Result<Vec<vaultpilot_lib::sync::ManifestEntry>, String> {
    vaultpilot_lib::sync::fetch_peer_manifest(&ip)
        .await
        .map_err(|e| e.to_string())
}

/// This vault's manifest — the "send to peer" side of the picker.
#[tauri::command]
pub async fn list_local_manifest() -> Result<Vec<vaultpilot_lib::sync::ManifestEntry>, String> {
    tokio::task::spawn_blocking(vaultpilot_lib::sync::local_manifest)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Sync exactly the files picked per direction by the user. Rebuilds the
/// index afterwards (same rationale as [`sync_with_peer`]).
#[tauri::command]
pub async fn sync_selected(
    state: tauri::State<'_, AppState>,
    ip: String,
    device_id: String,
    pull: Option<Vec<String>>,
    push: Option<Vec<String>>,
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
    let sel = vaultpilot_lib::sync::SyncSelection {
        pull: pull.unwrap_or_default(),
        push: push.unwrap_or_default(),
    };
    let result = vaultpilot_lib::sync::sync_with_peer_selection(&target_ip, &peer, &sel)
        .await
        .map_err(|e| e.to_string())?;
    // Best-effort: keep the index in step with newly pulled files.
    let _ = rebuild_index_async(&state.storage).await;
    Ok(result)
}
