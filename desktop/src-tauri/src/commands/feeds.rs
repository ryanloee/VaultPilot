//! Feed subscription management commands.
//!
//! Thin `#[tauri::command]` wrappers over `vaultpilot_lib::storage::feeds`
//! (CRUD) and `vaultpilot_lib::feed_ingest` (network refresh). CRUD is
//! storage-only and synchronous; refresh does outbound HTTP and is async.
//!
//! `FeedSubscription` already uses `#[serde(rename_all = "camelCase")]`, so it
//! crosses the bridge unchanged (no DTO needed). `FeedPollResult` likewise.

use crate::state::AppState;
use vaultpilot_lib::feed_ingest::{poll_all_feeds, poll_single_feed_by_id, FeedPollResult};
use vaultpilot_lib::models::FeedSubscription;
use vaultpilot_lib::storage::{
    create_feed_async, delete_feed_async, initialize_storage_async, list_feeds_async,
    set_feed_enabled_async, update_feed_async,
};

#[tauri::command]
pub async fn list_feeds(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FeedSubscription>, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    list_feeds_async(&ctx).await.map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn add_feed(
    state: tauri::State<'_, AppState>,
    url: String,
    title: String,
    kind: String,
    collection: String,
    tags: String,
    interval_minutes: i64,
) -> Result<FeedSubscription, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    create_feed_async(&ctx, url, title, kind, collection, tags, interval_minutes)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_feed(
    state: tauri::State<'_, AppState>,
    id: String,
    title: String,
    kind: String,
    collection: String,
    tags: String,
    interval_minutes: i64,
    enabled: bool,
) -> Result<bool, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    update_feed_async(
        &ctx,
        id,
        title,
        kind,
        collection,
        tags,
        interval_minutes,
        enabled,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_feed(state: tauri::State<'_, AppState>, id: String) -> Result<bool, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    delete_feed_async(&ctx, id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_feed_enabled(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<bool, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    set_feed_enabled_async(&ctx, id, enabled)
        .await
        .map_err(|e| e.to_string())
}

/// Fetch all enabled feeds now and ingest new entries as vault notes.
///
/// Runs the shared lib poller (`feed_ingest::poll_all_feeds`) — the same
/// engine the CLI `feed refresh` path uses. Each feed reports its own status
/// so one hostile/slow feed doesn't hide the rest.
#[tauri::command]
pub async fn refresh_feeds(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FeedPollResult>, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    Ok(poll_all_feeds(&ctx).await)
}

/// Fetch a single feed now (used for per-feed "立即刷新").
#[tauri::command]
pub async fn refresh_feed(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<FeedPollResult, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    poll_single_feed_by_id(&ctx, &id).await
}
