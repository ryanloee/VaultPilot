//! Agent Mode Phase 1 — sandboxed external AI agent integration.
//!
//! Lets VaultPilot run external AI agents (Claude Code, Codex, etc.) inside
//! the vault with strict permission and resource controls.
//!
//! # Design principles
//! - **Least privilege**: agents start read-only; write access is Phase 2.
//! - **Vault-scoped**: all file operations are confined to `vault_dir`.
//! - **Fail-closed**: any sandbox violation terminates the agent immediately.
//! - **Auditable**: every tool call is logged for security review.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::sanitize_error;

// ── Permission model ──────────────────────────────────────────────────────

/// Permission level for an agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AgentPermission {
    /// Can only read/search — no mutations.
    #[default]
    ReadOnly,
    /// Can read and write (Phase 2 — not yet enforced).
    ReadWrite,
}

// ── Resource limits ───────────────────────────────────────────────────────

/// Resource limits for a single agent execution session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResourceLimits {
    /// Maximum wall-clock time for the entire session.
    pub max_duration: Duration,
    /// Maximum number of tool invocations.
    pub max_tool_calls: u64,
    /// Maximum total input + output tokens (0 = unlimited).
    pub max_tokens: u64,
}

impl Default for AgentResourceLimits {
    fn default() -> Self {
        Self {
            max_duration: Duration::from_secs(300), // 5 minutes
            max_tool_calls: 100,
            max_tokens: 0, // unlimited by default
        }
    }
}

// ── Agent configuration ───────────────────────────────────────────────────

/// Configuration for an agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Human-readable agent name (e.g. "claude-code", "codex").
    pub name: String,
    /// Permission level.
    pub permission: AgentPermission,
    /// Resource limits.
    pub limits: AgentResourceLimits,
    /// Whitelisted tool names. Empty = all read-only tools allowed.
    pub allowed_tools: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "unnamed".into(),
            permission: AgentPermission::ReadOnly,
            limits: AgentResourceLimits::default(),
            allowed_tools: Vec::new(),
        }
    }
}

/// All known tools (read + write) — used as the default whitelist.
/// Write access is gated separately by `AgentPermission`.
const ALL_KNOWN_TOOLS: &[&str] = &[
    "search_notes",
    "read_file",
    "list_directory",
    "list_notes",
    "chat",
    "save_note",
    "write_note",
    "delete_note",
    "rename_note",
];

// ── Audit log ─────────────────────────────────────────────────────────────

/// A single audit entry for an agent tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAuditEntry {
    pub timestamp: String,
    pub tool: String,
    pub args_summary: String,
    pub allowed: bool,
    pub reason: String,
}

// ── Tool proxy ────────────────────────────────────────────────────────────

/// Result of a proxied tool call.
#[derive(Debug, Clone)]
pub struct ToolProxyResult {
    /// Whether the call was allowed and executed.
    pub allowed: bool,
    /// Human-readable reason (denial reason or "ok").
    pub reason: String,
}

/// Wraps tool dispatch with sandboxing, permission checks, and audit logging.
///
/// The proxy intercepts every tool call from an agent and:
/// 1. Checks if the tool is in the whitelist.
/// 2. For file-path tools, confines the path to `vault_dir`.
/// 3. Enforces read-only mode (blocks writes in Phase 1).
/// 4. Checks resource limits (tool call count, session duration).
/// 5. Logs every call to the audit trail.
pub struct ToolProxy {
    config: AgentConfig,
    vault_dir: PathBuf,
    tool_call_count: AtomicU64,
    session_start: Instant,
    audit_log: Mutex<Vec<AgentAuditEntry>>,
}

impl ToolProxy {
    pub fn new(config: AgentConfig, vault_dir: impl Into<PathBuf>) -> Self {
        Self {
            config,
            vault_dir: vault_dir.into(),
            tool_call_count: AtomicU64::new(0),
            session_start: Instant::now(),
            audit_log: Mutex::new(Vec::new()),
        }
    }

    /// Validate a tool call before execution. Returns `Ok(ToolProxyResult)`
    /// with `allowed=true` if the call may proceed.
    pub fn check_tool_call(&self, tool: &str, args_json: &str) -> Result<ToolProxyResult> {
        // 1. Resource limits — duration
        let elapsed = self.session_start.elapsed();
        if elapsed > self.config.limits.max_duration {
            let entry = self.deny(tool, args_json, "session timeout exceeded");
            return Ok(entry);
        }

        // 2. Resource limits — tool call count
        let count = self.tool_call_count.fetch_add(1, Ordering::Relaxed);
        if count >= self.config.limits.max_tool_calls {
            let entry = self.deny(tool, args_json, "tool call limit exceeded");
            return Ok(entry);
        }

        // 3. Tool whitelist
        if !self.is_tool_allowed(tool) {
            let entry = self.deny(tool, args_json, "tool not in whitelist");
            return Ok(entry);
        }

        // 4. Write permission check
        if Self::is_write_tool(tool) && self.config.permission == AgentPermission::ReadOnly {
            let entry = self.deny(tool, args_json, "write denied: agent is read-only");
            return Ok(entry);
        }

        // 5. Path confinement for file-path tools
        if let Some(path_value) = Self::extract_path_arg(tool, args_json) {
            if let Err(e) = self.confine_path(&path_value) {
                let entry = self.deny(tool, args_json, &format!("path violation: {e}"));
                return Ok(entry);
            }
        }

        let entry = self.allow(tool, args_json);
        Ok(entry)
    }

    /// Return the full audit log.
    pub fn audit_log(&self) -> Vec<AgentAuditEntry> {
        self.audit_log.lock().expect("audit_log poisoned").clone()
    }

    /// Number of tool calls so far.
    pub fn tool_call_count(&self) -> u64 {
        self.tool_call_count.load(Ordering::Relaxed)
    }

    /// Session elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.session_start.elapsed()
    }

    // ── internal helpers ──────────────────────────────────────────────────

    fn is_tool_allowed(&self, tool: &str) -> bool {
        if self.config.allowed_tools.is_empty() {
            // Default: allow all known tools. Write permission is checked separately.
            ALL_KNOWN_TOOLS.contains(&tool)
        } else {
            self.config.allowed_tools.iter().any(|t| t == tool)
        }
    }

    fn is_write_tool(tool: &str) -> bool {
        matches!(
            tool,
            "save_note" | "write_note" | "delete_note" | "rename_note"
        )
    }

    /// Extract a file-path argument from the tool's JSON args.
    /// Returns `None` for tools that don't take a path.
    fn extract_path_arg(tool: &str, args_json: &str) -> Option<String> {
        match tool {
            "read_file" | "list_directory" | "write_note" | "save_note" | "delete_note"
            | "rename_note" => {
                let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
                v.get("path").and_then(|p| p.as_str()).map(String::from)
            }
            _ => None,
        }
    }

    /// Confine a path to the vault directory. Relative paths are resolved
    /// against `vault_dir`. Reuses `normalize_tool_path` logic (canonicalize + prefix check).
    fn confine_path(&self, path: &str) -> Result<()> {
        let trimmed = path.trim().trim_matches('"').trim_matches('`');
        if trimmed.is_empty() {
            return Err(anyhow!("path is empty"));
        }

        let raw = PathBuf::from(trimmed);
        // Resolve relative paths against the vault directory.
        let candidate = if raw.is_absolute() {
            raw
        } else {
            self.vault_dir.join(raw)
        };
        let vault_canonical = self.vault_dir.canonicalize().map_err(|e| {
            anyhow!(
                "cannot resolve vault dir '{}': {}",
                self.vault_dir.display(),
                e
            )
        })?;

        // Try canonicalize (resolves symlinks). If the path exists, check
        // directly. Otherwise walk up to the nearest existing ancestor.
        if let Ok(canonical) = candidate.canonicalize() {
            if !canonical.starts_with(&vault_canonical) {
                return Err(anyhow!(
                    "access denied: '{}' is outside the vault",
                    sanitize_error(trimmed)
                ));
            }
        } else {
            let mut probe = candidate.as_path();
            let mut confined = false;
            while let Some(parent) = probe.parent() {
                if parent.as_os_str().is_empty() {
                    break;
                }
                if parent.exists() {
                    if let Ok(pc) = parent.canonicalize() {
                        if !pc.starts_with(&vault_canonical) {
                            return Err(anyhow!(
                                "access denied: '{}' is outside the vault",
                                sanitize_error(trimmed)
                            ));
                        }
                        confined = true;
                    }
                    break;
                }
                probe = parent;
            }
            if !confined {
                return Err(anyhow!(
                    "access denied: cannot verify '{}' is inside the vault",
                    sanitize_error(trimmed)
                ));
            }
        }

        Ok(())
    }

    fn allow(&self, tool: &str, args_json: &str) -> ToolProxyResult {
        let entry = AgentAuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            tool: tool.to_string(),
            args_summary: Self::summarize_args(args_json),
            allowed: true,
            reason: "ok".into(),
        };
        info!(tool = tool, "agent tool call allowed");
        self.audit_log
            .lock()
            .expect("audit_log poisoned")
            .push(entry);
        ToolProxyResult {
            allowed: true,
            reason: "ok".into(),
        }
    }

    fn deny(&self, tool: &str, args_json: &str, reason: &str) -> ToolProxyResult {
        let entry = AgentAuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            tool: tool.to_string(),
            args_summary: Self::summarize_args(args_json),
            allowed: false,
            reason: reason.to_string(),
        };
        warn!(tool = tool, reason = reason, "agent tool call denied");
        self.audit_log
            .lock()
            .expect("audit_log poisoned")
            .push(entry);
        ToolProxyResult {
            allowed: false,
            reason: reason.to_string(),
        }
    }

    /// Summarize args for audit log — cap at 200 chars to avoid log bloat.
    fn summarize_args(args_json: &str) -> String {
        if args_json.len() <= 200 {
            args_json.to_string()
        } else {
            format!("{}…", &args_json[..200])
        }
    }
}

// ── Agent session ─────────────────────────────────────────────────────────

/// High-level agent session that ties together config, proxy, and lifecycle.
pub struct AgentSession {
    pub config: AgentConfig,
    proxy: Arc<ToolProxy>,
}

impl AgentSession {
    pub fn new(config: AgentConfig, vault_dir: impl Into<PathBuf>) -> Self {
        let proxy = Arc::new(ToolProxy::new(config.clone(), vault_dir));
        Self { config, proxy }
    }

    /// Check whether a tool call is permitted.
    pub fn check(&self, tool: &str, args_json: &str) -> Result<ToolProxyResult> {
        self.proxy.check_tool_call(tool, args_json)
    }

    /// Return the audit log for this session.
    pub fn audit_log(&self) -> Vec<AgentAuditEntry> {
        self.proxy.audit_log()
    }

    /// Return the proxy for sharing across threads.
    pub fn proxy(&self) -> Arc<ToolProxy> {
        self.proxy.clone()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temporary vault directory with a sample note.
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn setup() -> (PathBuf, AgentConfig) {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "vaultpilot_agent_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("hello.md"), "# Hello\nWorld").unwrap();
        let config = AgentConfig::default();
        (tmp, config)
    }

    /// RAII guard that cleans up the temp vault directory on drop.
    struct TestGuard(PathBuf);
    impl Drop for TestGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn read_only_allows_search() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("search_notes", r#"{"query":"hello"}"#)
            .unwrap();
        assert!(r.allowed);
    }

    #[test]
    fn read_only_blocks_save_note() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("save_note", r#"{"path":"new.md","content":"hi"}"#)
            .unwrap();
        assert!(!r.allowed);
        assert!(r.reason.contains("read-only"));
    }

    #[test]
    fn blocks_unknown_tool() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy.check_tool_call("rm_rf", r#"{}"#).unwrap();
        assert!(!r.allowed);
        assert!(r.reason.contains("not in whitelist"));
    }

    #[test]
    fn path_confinement_blocks_escape() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("read_file", r#"{"path":"../../etc/passwd"}"#)
            .unwrap();
        assert!(!r.allowed);
        assert!(r.reason.contains("outside the vault"));
    }

    #[test]
    fn path_confinement_allows_in_vault() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("read_file", r#"{"path":"hello.md"}"#)
            .unwrap();
        assert!(r.allowed, "in-vault path should be allowed: {:?}", r.reason);
    }

    #[test]
    fn tool_call_limit_enforced() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.limits.max_tool_calls = 2;
        let proxy = ToolProxy::new(config, &tmp);

        assert!(
            proxy
                .check_tool_call("search_notes", r#"{"query":"a"}"#)
                .unwrap()
                .allowed
        );
        assert!(
            proxy
                .check_tool_call("search_notes", r#"{"query":"b"}"#)
                .unwrap()
                .allowed
        );
        // Third call should be denied.
        let r = proxy
            .check_tool_call("search_notes", r#"{"query":"c"}"#)
            .unwrap();
        assert!(!r.allowed);
        assert!(r.reason.contains("tool call limit"));
    }

    #[test]
    fn audit_log_records_all_calls() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);

        proxy
            .check_tool_call("search_notes", r#"{"query":"x"}"#)
            .unwrap();
        proxy
            .check_tool_call("save_note", r#"{"path":"y.md"}"#)
            .unwrap();

        let log = proxy.audit_log();
        assert_eq!(log.len(), 2);
        assert!(log[0].allowed);
        assert!(!log[1].allowed);
    }

    #[test]
    fn custom_whitelist_overrides_defaults() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.allowed_tools = vec!["search_notes".into()];
        let proxy = ToolProxy::new(config, &tmp);

        assert!(
            proxy
                .check_tool_call("search_notes", r#"{"query":"a"}"#)
                .unwrap()
                .allowed
        );
        // read_file is not in the custom list.
        let r = proxy
            .check_tool_call("read_file", r#"{"path":"hello.md"}"#)
            .unwrap();
        assert!(!r.allowed);
    }

    #[test]
    fn session_delegates_to_proxy() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let session = AgentSession::new(config, &tmp);
        let r = session.check("search_notes", r#"{"query":"hi"}"#).unwrap();
        assert!(r.allowed);
        assert_eq!(session.audit_log().len(), 1);
    }
}
