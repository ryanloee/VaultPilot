//! Shared Capability Registry — MCP/Skills/Tools once configured, reusable across agents.
//!
//! #1742: Creates a centralized capability store so users don't need to
//! re-configure the same MCP server or custom skill for every agent backend.
//!
//! ## Architecture
//!
//! - [`Capability`] enum: three kinds — `McpServer`, `Skill`, `Tool`
//! - [`CapabilityRegistry`]: the store, persisted to `.vaultpilot/capabilities.yaml`
//! - [`AgentBinding`]: which capabilities are enabled for which agent backend
//!
//! ## Usage
//!
//! ```rust,ignore
//! let mut registry = CapabilityRegistry::load(&vault_dir)?;
//! registry.add_server("my-github", "http://localhost:8080", None);
//! registry.enable_for_agent("my-github", "claude-code");
//! registry.save(&vault_dir)?;
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── Capability types ──────────────────────────────────────────

/// The three kinds of shareable capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Capability {
    /// An MCP server (stdio or HTTP) exposing external tools.
    McpServer {
        id: String,
        name: String,
        description: String,
        /// Transport: "stdio" (command) or "http" (URL).
        transport: McpTransport,
        /// OAuth / API key / token configuration (provider-specific).
        auth: Option<AuthConfig>,
    },
    /// A custom skill — system prompt fragment + optional tool list.
    Skill {
        id: String,
        name: String,
        description: String,
        /// System-prompt fragment injected when this skill is enabled.
        system_prompt_fragment: String,
        /// Tool ids this skill needs (must also be enabled).
        required_tool_ids: Vec<String>,
    },
    /// An atomic tool (vault search, file management, etc.).
    Tool {
        id: String,
        name: String,
        description: String,
        /// Rust-side tool identifier matching the agent engine's tool registry.
        tool_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum McpTransport {
    /// Standard stdio transport: program + args.
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    /// HTTP transport with URL.
    Http { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum AuthConfig {
    /// Bearer token or API key.
    BearerToken { token: String },
    /// OAuth2 client credentials.
    OAuth2 {
        client_id: String,
        client_secret: String,
        token_url: String,
    },
}

// ── Agent binding ─────────────────────────────────────────────

/// Which capabilities are enabled for a given agent backend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentBinding {
    /// Capability ids enabled for this agent.
    pub enabled: HashSet<String>,
}

// ── Registry ──────────────────────────────────────────────────

/// The central capability store, persisted as YAML.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilityRegistry {
    /// All registered capabilities (keyed by id).
    pub capabilities: HashMap<String, Capability>,
    /// Per-agent capability bindings.
    pub agent_bindings: HashMap<String, AgentBinding>,
}

impl CapabilityRegistry {
    /// Load from `.vaultpilot/capabilities.yaml` inside the vault directory.
    /// Returns an empty registry if the file doesn't exist.
    pub fn load(vault_dir: &Path) -> Result<Self, anyhow::Error> {
        let config_path = capabilities_path(vault_dir);
        if !config_path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&config_path)?;
        let registry: Self = serde_yaml_ng::from_str(&raw)?;
        Ok(registry)
    }

    /// Persist to `.vaultpilot/capabilities.yaml`.
    pub fn save(&self, vault_dir: &Path) -> Result<(), anyhow::Error> {
        let config_dir = vault_dir.join(".vaultpilot");
        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("capabilities.yaml");
        let yaml = serde_yaml_ng::to_string(self)?;
        std::fs::write(&config_path, yaml)?;
        Ok(())
    }

    // ── Capability management ──────────────────────────────

    /// Register (or overwrite) a capability.
    pub fn register(&mut self, cap: Capability) {
        let id = cap.id().to_string();
        self.capabilities.insert(id, cap);
    }

    /// Remove a capability and all its agent bindings.
    pub fn remove(&mut self, id: &str) {
        self.capabilities.remove(id);
        for binding in self.agent_bindings.values_mut() {
            binding.enabled.remove(id);
        }
    }

    /// List all registered capability ids.
    pub fn list_ids(&self) -> Vec<&str> {
        self.capabilities.keys().map(|s| s.as_str()).collect()
    }

    // ── Convenience constructors ───────────────────────────

    /// Register an MCP server.
    pub fn add_server(
        &mut self,
        id: &str,
        name: &str,
        description: &str,
        transport: McpTransport,
        auth: Option<AuthConfig>,
    ) {
        self.register(Capability::McpServer {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            transport,
            auth,
        });
    }

    /// Register a custom skill.
    pub fn add_skill(
        &mut self,
        id: &str,
        name: &str,
        description: &str,
        system_prompt_fragment: &str,
        required_tool_ids: Vec<String>,
    ) {
        self.register(Capability::Skill {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            system_prompt_fragment: system_prompt_fragment.to_string(),
            required_tool_ids,
        });
    }

    /// Register an atomic tool reference.
    pub fn add_tool(&mut self, id: &str, name: &str, description: &str, tool_id: &str) {
        self.register(Capability::Tool {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            tool_id: tool_id.to_string(),
        });
    }

    // ── Agent binding management ───────────────────────────

    /// Enable a capability for a given agent backend.
    pub fn enable_for_agent(&mut self, capability_id: &str, agent_id: &str) -> bool {
        if !self.capabilities.contains_key(capability_id) {
            return false;
        }
        self.agent_bindings
            .entry(agent_id.to_string())
            .or_default()
            .enabled
            .insert(capability_id.to_string());
        true
    }

    /// Disable a capability for a given agent backend.
    pub fn disable_for_agent(&mut self, capability_id: &str, agent_id: &str) {
        if let Some(binding) = self.agent_bindings.get_mut(agent_id) {
            binding.enabled.remove(capability_id);
            // Remove empty bindings to keep the file clean.
            if binding.enabled.is_empty() {
                self.agent_bindings.remove(agent_id);
            }
        }
    }

    /// List capabilities enabled for a specific agent.
    pub fn enabled_for(&self, agent_id: &str) -> Vec<&Capability> {
        let binding = match self.agent_bindings.get(agent_id) {
            Some(b) => &b.enabled,
            None => return vec![],
        };
        binding
            .iter()
            .filter_map(|id| self.capabilities.get(id))
            .collect()
    }

    /// List all agent ids that have at least one capability enabled.
    pub fn agent_ids(&self) -> Vec<&str> {
        self.agent_bindings
            .keys()
            .filter(|k| !self.agent_bindings[*k].enabled.is_empty())
            .map(|s| s.as_str())
            .collect()
    }
}

// ── Helper ────────────────────────────────────────────────────

fn capabilities_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(".vaultpilot").join("capabilities.yaml")
}

impl Capability {
    /// Get the capability's unique id.
    pub fn id(&self) -> &str {
        match self {
            Capability::McpServer { id, .. }
            | Capability::Skill { id, .. }
            | Capability::Tool { id, .. } => id,
        }
    }

    /// Human-readable name.
    pub fn name(&self) -> &str {
        match self {
            Capability::McpServer { name, .. }
            | Capability::Skill { name, .. }
            | Capability::Tool { name, .. } => name,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    /// Auto-cleanup helper for temp vault directories.
    struct TempVaultGuard(std::path::PathBuf);
    impl Drop for TempVaultGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_vault(name: &str) -> (std::path::PathBuf, TempVaultGuard) {
        let dir = temp_dir().join(format!("vp-cap-test-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        let guard = TempVaultGuard(dir.clone());
        (dir, guard)
    }

    fn empty_registry() -> CapabilityRegistry {
        CapabilityRegistry::default()
    }

    #[test]
    fn test_empty_registry() {
        let reg = empty_registry();
        assert!(reg.capabilities.is_empty());
        assert!(reg.agent_bindings.is_empty());
        assert!(reg.list_ids().is_empty());
    }

    #[test]
    fn test_register_mcp_server() {
        let mut reg = empty_registry();
        reg.add_server(
            "github-tools",
            "GitHub Tools",
            "Access GitHub issues and PRs",
            McpTransport::Stdio {
                command: "github-mcp-server".into(),
                args: vec!["--stdio".into()],
                env: HashMap::new(),
            },
            Some(AuthConfig::BearerToken {
                token: "ghp_test".into(),
            }),
        );
        assert_eq!(reg.list_ids(), vec!["github-tools"]);
        let cap = reg.capabilities.get("github-tools").unwrap();
        assert_eq!(cap.name(), "GitHub Tools");
    }

    #[test]
    fn test_register_skill() {
        let mut reg = empty_registry();
        reg.add_skill(
            "code-review",
            "Code Review",
            "Review code for bugs and style",
            "You are a code review assistant. Analyze carefully.",
            vec!["read_file".into(), "search_files".into()],
        );
        assert!(reg.capabilities.contains_key("code-review"));
    }

    #[test]
    fn test_register_tool() {
        let mut reg = empty_registry();
        reg.add_tool(
            "vault-search",
            "Vault Search",
            "Full-text vault search",
            "search_notes",
        );
        assert!(reg.capabilities.contains_key("vault-search"));
    }

    #[test]
    fn test_enable_disable_for_agent() {
        let mut reg = empty_registry();
        reg.add_server(
            "mcp-github",
            "GitHub",
            "desc",
            McpTransport::Http {
                url: "http://localhost:9999".into(),
            },
            None,
        );
        reg.add_skill("review", "Review", "desc", "You review code.", vec![]);

        // Enable
        assert!(reg.enable_for_agent("mcp-github", "claude-code"));
        assert!(reg.enable_for_agent("review", "claude-code"));

        let enabled = reg.enabled_for("claude-code");
        assert_eq!(enabled.len(), 2);
        assert!(enabled.iter().any(|c| c.id() == "mcp-github"));
        assert!(enabled.iter().any(|c| c.id() == "review"));

        // Disable one
        reg.disable_for_agent("review", "claude-code");
        let enabled = reg.enabled_for("claude-code");
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id(), "mcp-github");

        // Disable last → binding removed
        reg.disable_for_agent("mcp-github", "claude-code");
        assert!(reg.enabled_for("claude-code").is_empty());
        assert!(!reg.agent_bindings.contains_key("claude-code"));
    }

    #[test]
    fn test_enable_nonexistent_capability_fails() {
        let mut reg = empty_registry();
        assert!(!reg.enable_for_agent("no-such-cap", "any-agent"));
    }

    #[test]
    fn test_remove_capability_cleans_bindings() {
        let mut reg = empty_registry();
        reg.add_server(
            "test",
            "Test",
            "desc",
            McpTransport::Http {
                url: "http://x".into(),
            },
            None,
        );
        reg.enable_for_agent("test", "agent-a");
        reg.enable_for_agent("test", "agent-b");

        reg.remove("test");
        assert!(!reg.capabilities.contains_key("test"));
        assert!(reg.enabled_for("agent-a").is_empty());
        assert!(reg.enabled_for("agent-b").is_empty());
    }

    #[test]
    fn test_serialization_round_trip() {
        let mut reg = empty_registry();
        reg.add_server(
            "srv",
            "Server",
            "desc",
            McpTransport::Stdio {
                command: "node".into(),
                args: vec!["server.js".into()],
                env: HashMap::from([("NODE_ENV".into(), "production".into())]),
            },
            Some(AuthConfig::BearerToken {
                token: "secret123".into(),
            }),
        );
        reg.add_skill("skill-a", "Skill A", "desc", "prompt", vec![]);
        reg.add_tool("tool-x", "Tool X", "desc", "x_id");
        reg.enable_for_agent("srv", "claude");
        reg.enable_for_agent("skill-a", "claude");

        let yaml = serde_yaml_ng::to_string(&reg).unwrap();
        let restored: CapabilityRegistry = serde_yaml_ng::from_str(&yaml).unwrap();

        assert_eq!(restored.capabilities.len(), 3);
        assert_eq!(restored.enabled_for("claude").len(), 2);
    }

    #[test]
    fn test_save_and_load_to_disk() {
        let (vault, _guard) = temp_vault("save-load");

        let mut reg = empty_registry();
        reg.add_server(
            "disk-test",
            "Disk",
            "desc",
            McpTransport::Http {
                url: "http://x".into(),
            },
            None,
        );
        reg.save(&vault).unwrap();

        // Reload
        let loaded = CapabilityRegistry::load(&vault).unwrap();
        assert!(loaded.capabilities.contains_key("disk-test"));
    }

    #[test]
    fn test_load_missing_file_returns_empty() {
        let (vault, _guard) = temp_vault("missing");
        let nonexistent = vault.join("no-vaultpilot-dir");
        let loaded = CapabilityRegistry::load(&nonexistent).unwrap();
        assert!(loaded.capabilities.is_empty());
    }
}
