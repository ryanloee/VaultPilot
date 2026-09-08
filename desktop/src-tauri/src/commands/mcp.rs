//! MCP connector config commands.
//!
//! The desktop integrations UI generates a token and persists it into the
//! vault-root `mcp-config.json` — the same file the standalone
//! `vaultpilot-mcp` connector reads at startup. The file (not the UI state)
//! is the source of truth, so the token survives page switches and restarts.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use vaultpilot_lib::mcp_config::{load_config, save_token, McpConnectorConfig};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfigDto {
    pub vault_dir: Option<String>,
    pub token: Option<String>,
}

/// Current connector config (token included — it is a local vault access
/// secret the owner manages from this machine, same trust level as the
/// settings page showing API keys).
#[tauri::command]
pub async fn get_mcp_config(
    state: tauri::State<'_, AppState>,
) -> Result<Option<McpConfigDto>, String> {
    let vault_dir = state.storage.vault_dir().to_path_buf();
    tokio::task::spawn_blocking(move || load_config(&vault_dir).map(McpConfigDto::from))
        .await
        .map_err(|e| e.to_string())
}

/// Persist a newly generated token into the vault-root `mcp-config.json`.
/// Existing fields in the file are preserved.
#[tauri::command]
pub async fn save_mcp_token(
    state: tauri::State<'_, AppState>,
    token: String,
) -> Result<(), String> {
    let vault_dir = state.storage.vault_dir().to_path_buf();
    tokio::task::spawn_blocking(move || save_token(&vault_dir, Some(&token)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

impl From<McpConnectorConfig> for McpConfigDto {
    fn from(c: McpConnectorConfig) -> Self {
        Self {
            vault_dir: c.vault_dir,
            token: c.token,
        }
    }
}
