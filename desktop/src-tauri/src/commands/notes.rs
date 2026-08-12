//! Note-related commands — thin wrappers over `vaultpilot_lib::storage::*`.
//!
//! Return types are `serde_json::Value` where the exact Rust type isn't easily
//! nameable from this crate (some are private or nested). The on-wire shape is
//! identical because the underlying types are `Serialize`.

use crate::state::AppState;
use std::path::Path;
use vaultpilot_lib::models::NoteDocument;
use vaultpilot_lib::storage::{
    delete_note_async, find_backlinks_async, find_related_notes_async, get_snapshot_async,
    import_markdown_async, list_notes_async, list_snapshots_for_note_async, load_note_async,
    rebuild_index_async, restore_snapshot_async, save_note_async,
};

#[tauri::command]
pub async fn list_notes(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let v = list_notes_async(&state.storage)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn load_note(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<NoteDocument, String> {
    load_note_async(&state.storage, &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_note(
    state: tauri::State<'_, AppState>,
    note: NoteDocument,
) -> Result<NoteDocument, String> {
    save_note_async(&state.storage, note)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_note(state: tauri::State<'_, AppState>, id: String) -> Result<bool, String> {
    let cleanup = vaultpilot_lib::storage::load_settings_async(&state.storage)
        .await
        .unwrap_or_default()
        .attachment_cleanup_on_note_delete
        .resolve_delete_attachments();
    delete_note_async(&state.storage, &id, cleanup)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn find_related_notes(
    state: tauri::State<'_, AppState>,
    id: String,
    limit: Option<usize>,
) -> Result<serde_json::Value, String> {
    let v = find_related_notes_async(&state.storage, &id, limit.unwrap_or(5))
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn find_backlinks(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    let v = find_backlinks_async(&state.storage, &id)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn import_markdown(
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
) -> Result<serde_json::Value, String> {
    let v = import_markdown_async(&state.storage, &paths)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rebuild_index(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let v = rebuild_index_async(&state.storage)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_snapshots(
    state: tauri::State<'_, AppState>,
    note_id: String,
) -> Result<serde_json::Value, String> {
    let v = list_snapshots_for_note_async(&state.storage, &note_id)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_snapshot(
    state: tauri::State<'_, AppState>,
    snapshot_id: String,
) -> Result<serde_json::Value, String> {
    let v = get_snapshot_async(&state.storage, &snapshot_id)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restore_snapshot(
    state: tauri::State<'_, AppState>,
    note_id: String,
    snapshot_id: String,
) -> Result<NoteDocument, String> {
    restore_snapshot_async(&state.storage, &note_id, &snapshot_id)
        .await
        .map_err(|e| e.to_string())
}

/// Reads an image file and returns it as a `data:` URL (base64). Confined to
/// the vault root to prevent path traversal, matching the sidecar's behavior.
#[tauri::command]
pub async fn read_image_preview(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let settings = vaultpilot_lib::storage::initialize_storage_async(&state.storage)
        .await
        .map_err(|e| e.to_string())?;
    let vault_root = Path::new(&settings.vault_dir);
    let confined =
        vaultpilot_lib::normalize_tool_path(&path, vault_root).map_err(|e| e.to_string())?;
    read_image_as_data_url(&confined.to_string_lossy())
}

/// Opens a path in the system file manager. Confined to the vault root.
#[tauri::command]
pub async fn open_vault_directory(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let settings = vaultpilot_lib::storage::initialize_storage_async(&state.storage)
        .await
        .map_err(|e| e.to_string())?;
    let vault_root = Path::new(&settings.vault_dir);
    let confined =
        vaultpilot_lib::normalize_tool_path(&path, vault_root).map_err(|e| e.to_string())?;
    open_in_file_manager(&confined.to_string_lossy())
}

// ── helpers (ported from src/bin/vaultpilot-agent.rs) ──────────────────────

fn read_image_as_data_url(path: &str) -> Result<String, String> {
    const MAX_IMAGE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len() > MAX_IMAGE_SIZE {
        return Err(format!(
            "image too large ({} bytes, limit is {} bytes): {}",
            metadata.len(),
            MAX_IMAGE_SIZE,
            path
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let media_type = Path::new(path)
        .extension()
        .and_then(|v| v.to_str())
        .map(|v| v.to_ascii_lowercase())
        .as_deref()
        .and_then(|ext| match ext {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "webp" => Some("image/webp"),
            "gif" => Some("image/gif"),
            _ => None,
        })
        .ok_or_else(|| "unsupported image format".to_string())?;
    use base64::Engine;
    Ok(format!(
        "data:{};base64,{}",
        media_type,
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn open_in_file_manager(path: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let target = Path::new(path);
    if !target.exists() {
        return Err("vault directory does not exist".to_string());
    }
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        return Err("unsupported platform".to_string());
    }
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    {
        Command::new(program)
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
