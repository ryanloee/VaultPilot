//! Collection commands — hierarchical note grouping (parent/child trees).

use crate::state::AppState;
use serde_json::{json, Value};
use vaultpilot_lib::models::Collection;
use vaultpilot_lib::storage::{
    add_note_to_collection_async, create_collection_async, delete_collection_async,
    get_collections_for_note_with_context, list_collections_async, list_notes_in_collection_async,
    move_collection_async, remove_note_from_collection_async, rename_collection_async,
};

#[tauri::command]
pub async fn list_collections(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let v = list_collections_async(&state.storage)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_collection(
    state: tauri::State<'_, AppState>,
    name: String,
    description: Option<String>,
    parent_id: Option<String>,
) -> Result<Collection, String> {
    create_collection_async(
        &state.storage,
        name,
        description.unwrap_or_default(),
        parent_id.unwrap_or_default(),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_collection(
    state: tauri::State<'_, AppState>,
    collection_id: String,
    name: String,
) -> Result<bool, String> {
    rename_collection_async(&state.storage, collection_id, name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn move_collection(
    state: tauri::State<'_, AppState>,
    collection_id: String,
    new_parent_id: Option<String>,
) -> Result<bool, String> {
    move_collection_async(
        &state.storage,
        collection_id,
        new_parent_id.unwrap_or_default(),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_collection(
    state: tauri::State<'_, AppState>,
    collection_id: String,
) -> Result<bool, String> {
    delete_collection_async(&state.storage, collection_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_note_to_collection(
    state: tauri::State<'_, AppState>,
    note_id: String,
    collection_id: String,
) -> Result<bool, String> {
    add_note_to_collection_async(&state.storage, note_id, collection_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_note_from_collection(
    state: tauri::State<'_, AppState>,
    note_id: String,
    collection_id: String,
) -> Result<bool, String> {
    remove_note_from_collection_async(&state.storage, note_id, collection_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_notes_in_collection(
    state: tauri::State<'_, AppState>,
    collection_id: String,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Value, String> {
    let v = list_notes_in_collection_async(
        &state.storage,
        collection_id,
        limit.unwrap_or(200),
        offset.unwrap_or(0),
    )
    .await
    .map_err(|e| e.to_string())?;
    serde_json::to_value(json!({ "notes": v })).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_collections_for_note(
    state: tauri::State<'_, AppState>,
    note_id: String,
) -> Result<Value, String> {
    let v = get_collections_for_note_with_context(&state.storage, &note_id)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&v).map_err(|e| e.to_string())
}
