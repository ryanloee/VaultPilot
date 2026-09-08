//! `mcp-config.json` — shared config file for the standalone
//! `vaultpilot-mcp` stdio connector.
//!
//! Lives in the vault root. The connector discovers it at startup
//! (vault_dir fallback + token), and the desktop integrations UI reads and
//! writes the token here so the generated token survives page switches and
//! app restarts — the file is the single source of truth, the UI only
//! edits it.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Contents of `mcp-config.json`. Both fields optional; unknown fields from
/// hand-edited files are ignored and preserved via load-modify-write.
/// Field names are snake_case to match the `vaultpilot-mcp` connector's
/// parser (`vault_dir` / `token`) — do not camelCase this file format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConnectorConfig {
    /// Vault directory for the connector (informational; the connector's
    /// `--vault-dir` arg takes precedence, then discovery, then sidecar).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_dir: Option<String>,
    /// Expected token. When set, the connector requires a matching
    /// `VAULTPILOT_MCP_TOKEN` env proof or `initialize _meta` token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Path of the config file inside a vault root.
pub fn config_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join("mcp-config.json")
}

/// Load the config from a vault root. Returns `None` when the file is
/// missing or unparsable (the connector treats both as "no config").
pub fn load_config(vault_dir: &Path) -> Option<McpConnectorConfig> {
    let content = std::fs::read_to_string(config_path(vault_dir)).ok()?;
    serde_json::from_str(&content).ok()
}

/// Set (or clear, with `None`) the token in the vault-root config,
/// preserving any other fields the file may carry.
pub fn save_token(vault_dir: &Path, token: Option<&str>) -> Result<()> {
    let mut cfg = load_config(vault_dir).unwrap_or_default();
    cfg.token = token.map(str::to_string);
    let path = config_path(vault_dir);
    let body = serde_json::to_string_pretty(&cfg).context("failed to serialize mcp-config")?;
    std::fs::write(&path, body + "\n")
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vp-mcp-config-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn load_missing_file_returns_none() {
        let vault = temp_vault();
        assert!(load_config(&vault).is_none());
    }

    #[test]
    fn save_then_load_round_trips_token() {
        let vault = temp_vault();
        save_token(&vault, Some("test-token-standin")).expect("save");
        let cfg = load_config(&vault).expect("load");
        assert_eq!(cfg.token.as_deref(), Some("test-token-standin"));
        assert!(cfg.vault_dir.is_none());
    }

    #[test]
    fn save_preserves_other_fields() {
        let vault = temp_vault();
        // Hand-edited file with extra/unknown fields and a vault_dir.
        std::fs::write(
            config_path(&vault),
            r#"{"vault_dir": "D:/vault", "token": "old", "custom": 1}"#,
        )
        .expect("seed");
        save_token(&vault, Some("test-token-standin")).expect("save");
        let cfg = load_config(&vault).expect("load");
        assert_eq!(cfg.token.as_deref(), Some("test-token-standin"));
        assert_eq!(cfg.vault_dir.as_deref(), Some("D:/vault"));
    }

    #[test]
    fn save_none_clears_token() {
        let vault = temp_vault();
        save_token(&vault, Some("test-token-standin")).expect("save");
        save_token(&vault, None).expect("clear");
        let cfg = load_config(&vault).expect("load");
        assert!(cfg.token.is_none());
    }

    #[test]
    fn unparsable_file_loads_as_none_and_save_overwrites() {
        let vault = temp_vault();
        std::fs::write(config_path(&vault), "not json at all").expect("seed");
        assert!(load_config(&vault).is_none());
        save_token(&vault, Some("test-token-standin")).expect("save over garbage");
        assert_eq!(
            load_config(&vault).and_then(|c| c.token),
            Some("test-token-standin".to_string())
        );
    }
}
