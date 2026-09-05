//! Mail (IMAP-to-vault) management commands.
//!
//! Thin `#[tauri::command]` wrappers over `vaultpilot_lib::mail`. The lib
//! module is gated behind the `email` cargo feature (default-on; Android
//! disables it because native-tls/OpenSSL can't cross-compile), so this whole
//! module is gated on the matching `desktop` cfg that tauri-build sets for
//! non-mobile targets.
//!
//! Passwords are encrypted at rest and `#[serde(skip_serializing)]` on the
//! lib struct, so account listing never leaks credentials to the frontend:
//! the DTO below only carries the non-secret fields.

#![cfg(desktop)]

use crate::state::AppState;
use serde::{Deserialize, Serialize};

/// Mail account as seen by the frontend - no password, ever.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAccountDto {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub use_tls: bool,
    pub sync_enabled: bool,
    pub sync_frequency_minutes: u64,
    pub last_sync_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(desktop)]
impl From<vaultpilot_lib::mail::MailAccount> for MailAccountDto {
    fn from(a: vaultpilot_lib::mail::MailAccount) -> Self {
        Self {
            id: a.id,
            name: a.name,
            host: a.host,
            port: a.port,
            username: a.username,
            use_tls: a.use_tls,
            sync_enabled: a.sync_enabled,
            sync_frequency_minutes: a.sync_frequency_minutes,
            last_sync_at: a.last_sync_at,
            created_at: a.created_at,
            updated_at: a.updated_at,
        }
    }
}

#[cfg(desktop)]
use vaultpilot_lib::storage::initialize_storage_async;

#[tauri::command]
#[cfg(desktop)]
pub async fn list_mail_accounts(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<MailAccountDto>, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    vaultpilot_lib::mail::list_mail_accounts_async(&ctx)
        .await
        .map(|accounts| accounts.into_iter().map(MailAccountDto::from).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[cfg(desktop)]
#[allow(clippy::too_many_arguments)]
pub async fn add_mail_account(
    state: tauri::State<'_, AppState>,
    name: String,
    host: String,
    port: u16,
    username: String,
    password: String,
    use_tls: bool,
    sync_frequency_minutes: u64,
) -> Result<MailAccountDto, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    vaultpilot_lib::mail::add_mail_account_async(
        &ctx,
        name,
        host,
        port,
        username,
        password,
        use_tls,
        sync_frequency_minutes,
    )
    .await
    .map(MailAccountDto::from)
    .map_err(|e| e.to_string())
}

#[tauri::command]
#[cfg(desktop)]
pub async fn delete_mail_account(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    vaultpilot_lib::mail::delete_mail_account_async(&ctx, id)
        .await
        .map_err(|e| e.to_string())
}

/// Sync one account now: IMAP fetch + vault ingest (blocking network I/O on
/// the lib side; the command itself just awaits).
#[tauri::command]
#[cfg(desktop)]
pub async fn sync_mail_account(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    let result = vaultpilot_lib::mail::sync_mail_account_async(&ctx, id)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&result).map_err(|e| e.to_string())
}

/// Search already-imported emails (subject / from / body).
#[tauri::command]
#[cfg(desktop)]
pub async fn search_emails(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<serde_json::Value, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    let emails = vaultpilot_lib::mail::search_emails_async(
        &ctx,
        query,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
    )
    .await
    .map_err(|e| e.to_string())?;
    serde_json::to_value(&emails).map_err(|e| e.to_string())
}
