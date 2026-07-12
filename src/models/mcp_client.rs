//! MCP Client configuration — defines external MCP servers that the Agent
//! connects to for dynamic tool discovery (#1889).
//!
//! The config file `mcp_servers.json` lives in the vault's `.vaultpilot/`
//! directory. Each entry specifies how to connect to a remote MCP Server
//! (stdio or HTTP transport) along with optional authentication.

use serde::{Deserialize, Serialize};

/// MCP transport protocol for connecting to an external server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    /// JSON-RPC over stdin/stdout (subprocess).
    Stdio {
        /// Executable to spawn (e.g. "npx", "python3").
        command: String,
        /// Arguments for the subprocess.
        #[serde(default)]
        args: Vec<String>,
    },
    /// JSON-RPC over HTTP (SSE streaming or REST).
    Http {
        /// Base URL of the MCP server (e.g. "http://localhost:3000/mcp").
        url: String,
        /// Optional Bearer token or API key.
        #[serde(default)]
        auth_header: Option<String>,
    },
}

/// Configuration for a single external MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    /// Human-readable name for this server (e.g. "github-tools").
    pub name: String,
    /// MCP transport configuration.
    pub transport: McpTransport,
    /// Optional description shown in the UI.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether this server is enabled. Disabled servers are skipped
    /// during tool discovery but kept in config for easy re-enabling.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Top-level structure of `mcp_servers.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServersConfig {
    /// List of registered external MCP servers.
    #[serde(default)]
    pub servers: Vec<McpServerEntry>,
}

/// Load the MCP servers config from `vault_dir/.vaultpilot/mcp_servers.json`.
/// Returns `McpServersConfig::default()` (empty) if the file does not exist.
pub fn load_mcp_servers_config(vault_dir: &str) -> McpServersConfig {
    let path = std::path::Path::new(vault_dir)
        .join(".vaultpilot")
        .join("mcp_servers.json");

    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<McpServersConfig>(&content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!(
                    "Warning: failed to parse mcp_servers.json ({path}): {e}",
                    path = path.display()
                );
                McpServersConfig::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => McpServersConfig::default(),
        Err(e) => {
            eprintln!(
                "Warning: cannot read mcp_servers.json ({path}): {e}",
                path = path.display()
            );
            McpServersConfig::default()
        }
    }
}

/// Return only enabled servers from the config.
pub fn enabled_servers(config: &McpServersConfig) -> Vec<&McpServerEntry> {
    config.servers.iter().filter(|s| s.enabled).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_servers_config_default_is_empty() {
        let cfg = McpServersConfig::default();
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn parse_minimal_config_with_single_http_entry() {
        let json = r#"{
            "servers": [
                {
                    "name": "github-tools",
                    "transport": {
                        "http": {
                            "url": "http://localhost:8080/mcp",
                            "auth_header": "Bearer ghp_xxxxx"
                        }
                    }
                }
            ]
        }"#;

        let cfg: McpServersConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        let s = &cfg.servers[0];
        assert_eq!(s.name, "github-tools");
        assert!(s.enabled);
        assert!(s.description.is_none());

        match &s.transport {
            McpTransport::Http { url, auth_header } => {
                assert_eq!(url, "http://localhost:8080/mcp");
                assert_eq!(auth_header.as_deref(), Some("Bearer ghp_xxxxx"));
            }
            _ => panic!("expected HTTP transport"),
        }
    }

    #[test]
    fn parse_stdio_entry() {
        let json = r#"{
            "servers": [
                {
                    "name": "linear-tools",
                    "transport": {
                        "stdio": {
                            "command": "npx",
                            "args": ["-y", "@linear/mcp-server"]
                        }
                    }
                }
            ]
        }"#;

        let cfg: McpServersConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        let s = &cfg.servers[0];
        assert_eq!(s.name, "linear-tools");

        match &s.transport {
            McpTransport::Stdio { command, args } => {
                assert_eq!(command, "npx");
                assert_eq!(args, &["-y", "@linear/mcp-server"]);
            }
            _ => panic!("expected Stdio transport"),
        }
    }

    #[test]
    fn disabled_server_filtered_out() {
        let json = r#"{
            "servers": [
                {"name": "enabled-one", "transport": {"http": {"url": "http://localhost:1/mcp"}}, "enabled": true},
                {"name": "disabled-one", "transport": {"http": {"url": "http://localhost:2/mcp"}}, "enabled": false}
            ]
        }"#;

        let cfg: McpServersConfig = serde_json::from_str(json).unwrap();
        let active = enabled_servers(&cfg);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "enabled-one");
    }

    #[test]
    fn round_trip_config() {
        let cfg = McpServersConfig {
            servers: vec![
                McpServerEntry {
                    name: "test-stdio".into(),
                    transport: McpTransport::Stdio {
                        command: "python3".into(),
                        args: vec!["-m".into(), "my_mcp".into()],
                    },
                    description: Some("Test MCP server".into()),
                    enabled: true,
                },
                McpServerEntry {
                    name: "test-http".into(),
                    transport: McpTransport::Http {
                        url: "https://example.com/mcp".into(),
                        auth_header: None,
                    },
                    description: None,
                    enabled: false,
                },
            ],
        };

        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let cfg2: McpServersConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg2.servers.len(), 2);

        // Check stdio entry
        let s0 = &cfg2.servers[0];
        assert_eq!(s0.name, "test-stdio");
        assert!(s0.enabled);
        assert_eq!(s0.description.as_deref(), Some("Test MCP server"));

        if let McpTransport::Stdio { command, args } = &s0.transport {
            assert_eq!(command, "python3");
            assert_eq!(args, &["-m", "my_mcp"]);
        } else {
            panic!("expected Stdio");
        }

        // Check http entry (disabled)
        let s1 = &cfg2.servers[1];
        assert_eq!(s1.name, "test-http");
        assert!(!s1.enabled);
        assert!(s1.description.is_none());

        if let McpTransport::Http { url, auth_header } = &s1.transport {
            assert_eq!(url, "https://example.com/mcp");
            assert!(auth_header.is_none());
        } else {
            panic!("expected Http");
        }
    }

    #[test]
    fn empty_servers_array_is_valid() {
        let json = r#"{"servers": []}"#;
        let cfg: McpServersConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn missing_servers_field_defaults_to_empty() {
        let json = r#"{}"#;
        let cfg: McpServersConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn enabled_defaults_to_true() {
        let json = r#"{
            "servers": [
                {
                    "name": "no-enabled-field",
                    "transport": {"http": {"url": "http://localhost:9/mcp"}}
                }
            ]
        }"#;
        let cfg: McpServersConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.servers[0].enabled);
    }
}
