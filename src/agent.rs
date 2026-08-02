//! Agent Mode — sandboxed external AI agent integration.
//!
//! Lets VaultPilot run external AI agents (Claude Code, Codex, etc.) inside
//! the vault with strict permission and resource controls.
//!
//! # Design principles
//! - **Least privilege**: agents start read-only; write access requires explicit pattern whitelist.
//! - **Vault-scoped**: all file operations are confined to `vault_dir`.
//! - **Fail-closed**: any sandbox violation terminates the agent immediately.
//! - **Auditable**: every tool call is logged for security review.

use std::io::Read;
use std::path::{Path, PathBuf};
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
#[serde(rename_all = "snake_case")]
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
    /// Glob patterns for writable paths (e.g. "*.md", "daily-notes/*", "inbox/*").
    /// Only enforced when `permission` is `ReadWrite`. Empty = no write access.
    pub write_patterns: Vec<String>,
    /// Execution mode: Direct (default, immediate execution) or Plan (plan first).
    #[serde(default)]
    pub execution_mode: ExecutionMode,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "unnamed".into(),
            permission: AgentPermission::ReadOnly,
            limits: AgentResourceLimits::default(),
            allowed_tools: Vec::new(),
            write_patterns: Vec::new(),
            execution_mode: ExecutionMode::default(),
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
    "save_note",
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
    /// Optional real-time subscriber for tool-call events. When set, every
    /// allowed/denied call is forwarded to this sink the moment it is decided
    /// (in addition to being appended to the post-hoc `audit_log`). This powers
    /// the live "tool call transparency" panel (#1905) without changing
    /// existing callers — `None` keeps the prior behavior.
    event_sink: Option<Arc<dyn Fn(AgentAuditEntry) + Send + Sync>>,
}

impl ToolProxy {
    pub fn new(config: AgentConfig, vault_dir: impl Into<PathBuf>) -> Self {
        Self {
            config,
            vault_dir: vault_dir.into(),
            tool_call_count: AtomicU64::new(0),
            session_start: Instant::now(),
            audit_log: Mutex::new(Vec::new()),
            event_sink: None,
        }
    }

    /// Attach a real-time subscriber that receives an [`AgentAuditEntry`] for
    /// every tool call the moment it is allowed or denied. Intended for UI
    /// transparency panels (#1905). Safe to call once before any `check_tool_call`.
    pub fn with_event_sink(mut self, sink: Arc<dyn Fn(AgentAuditEntry) + Send + Sync>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    /// Emit a tool-call event to the attached sink (if any) and record it in
    /// the audit log. Centralizes entry construction so the live stream and the
    /// post-hoc log never diverge.
    fn record(&self, entry: AgentAuditEntry) {
        if let Some(sink) = &self.event_sink {
            sink(entry.clone());
        }
        self.audit_log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(entry);
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
        // Check the count without incrementing — only allowed calls should
        // consume quota (#1535).
        let count = self.tool_call_count.load(Ordering::Relaxed);
        if self.config.limits.max_tool_calls > 0 && count >= self.config.limits.max_tool_calls {
            let entry = self.deny(tool, args_json, "tool call limit exceeded");
            return Ok(entry);
        }

        // 3. Tool whitelist
        if !self.is_tool_allowed(tool) {
            let entry = self.deny(tool, args_json, "tool not in whitelist");
            return Ok(entry);
        }

        // 4. Write permission check
        if Self::is_write_tool(tool) {
            if self.config.permission == AgentPermission::ReadOnly {
                let entry = self.deny(tool, args_json, "write denied: agent is read-only");
                return Ok(entry);
            }
            // Check write pattern whitelist
            let paths = Self::extract_path_args(tool, args_json);
            if paths.is_empty() {
                let entry = self.deny(
                    tool,
                    args_json,
                    "write denied: missing required 'path' argument",
                );
                return Ok(entry);
            }
            for path_value in &paths {
                if !self.is_path_writable(path_value) {
                    let entry = self.deny(
                        tool,
                        args_json,
                        &format!(
                            "write denied: path '{}' does not match write patterns",
                            sanitize_error(path_value)
                        ),
                    );
                    return Ok(entry);
                }
            }
        }

        // 5. Path confinement for file-path tools
        let paths = Self::extract_path_args(tool, args_json);
        if paths.is_empty() {
            // Write tools already denied above. For read tools that
            // take a path (read_file, list_directory), missing path
            // is also a problem — deny it. Tools that don't take a
            // path at all (e.g. search_notes) are unaffected.
            if Self::takes_path(tool) && !Self::is_write_tool(tool) {
                let entry = self.deny(tool, args_json, "missing required 'path' argument");
                return Ok(entry);
            }
        } else {
            for path_value in &paths {
                if let Err(e) = self.confine_path(path_value) {
                    let entry = self.deny(tool, args_json, &format!("path violation: {e}"));
                    return Ok(entry);
                }
            }
        }

        // All checks passed — atomically check the limit and increment.
        // Using a CAS loop to avoid TOCTOU race (#1572).
        //
        // When max_tool_calls == 0 the limit is unlimited: skip the CAS loop
        // entirely so we never overflow the counter (#2205).
        if self.config.limits.max_tool_calls == 0 {
            let entry = self.allow(tool, args_json);
            return Ok(entry);
        }

        loop {
            let current = self.tool_call_count.load(Ordering::Acquire);
            if current >= self.config.limits.max_tool_calls {
                let entry = self.deny(tool, args_json, "tool call limit exceeded");
                return Ok(entry);
            }
            match self.tool_call_count.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
        let entry = self.allow(tool, args_json);
        Ok(entry)
    }

    /// Return the full audit log.
    pub fn audit_log(&self) -> Vec<AgentAuditEntry> {
        self.audit_log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Merge external audit entries into this proxy's audit log.
    /// Used by Plan Mode to preserve the recon-pass audit trail
    /// that was collected on a separate short-lived ToolProxy (#2425).
    pub fn merge_audit_log(&self, entries: Vec<AgentAuditEntry>) {
        self.audit_log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(entries);
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

    /// Whether the tool is expected to take a `path` argument.
    fn takes_path(tool: &str) -> bool {
        matches!(
            tool,
            "read_file"
                | "list_directory"
                | "write_note"
                | "save_note"
                | "delete_note"
                | "rename_note"
        )
    }

    /// Extract file-path arguments from the tool's JSON args.
    /// Returns an empty vector for tools that don't take a path.
    /// For `rename_note`, both the source `path` and destination `newPath`
    /// are returned so that both are checked against confine_path and
    /// is_path_writable.
    fn extract_path_args(tool: &str, args_json: &str) -> Vec<String> {
        match tool {
            "read_file" | "list_directory" | "write_note" | "save_note" | "delete_note" => {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(args_json) else {
                    return vec![];
                };
                match v.get("path").and_then(|p| p.as_str()) {
                    Some(p) => vec![p.to_string()],
                    None => vec![],
                }
            }
            "rename_note" => {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(args_json) else {
                    return vec![];
                };
                let mut paths = Vec::new();
                if let Some(p) = v.get("path").and_then(|p| p.as_str()) {
                    paths.push(p.to_string());
                }
                if let Some(p) = v.get("newPath").and_then(|p| p.as_str()) {
                    paths.push(p.to_string());
                }
                paths
            }
            _ => vec![],
        }
    }

    /// Normalize a path by eliminating `.` and `..` components.
    fn normalize_path_components(path: &str) -> String {
        // Normalize Windows backslashes to forward slashes so glob matching
        // works cross-platform (#2576). Path::canonicalize() returns paths
        // with backslashes on Windows, but normalize_path_components splits
        // on '/' and glob_match treats '\\' as a path separator.
        let normalized = path.replace('\\', "/");
        let is_absolute = normalized.starts_with('/');
        let mut components: Vec<&str> = Vec::new();
        for component in normalized.split('/') {
            match component {
                "." | "" => {}
                ".." => {
                    components.pop();
                }
                _ => components.push(component),
            }
        }
        let mut result = components.join("/");
        if is_absolute {
            result.insert(0, '/');
        }
        result
    }

    /// Check if a path matches the write pattern whitelist.
    /// Patterns are glob-style: "inbox/*", "daily-notes/*", "inbox/*".
    ///
    /// # TOCTOU note
    /// This function uses lexical `strip_prefix` without resolving symlinks.
    /// It is a coarse pre-check only. The subsequent `confine_path` call
    /// performs full canonicalization (`normalize_tool_path`) which resolves
    /// symlinks and enforces the vault boundary. A path that passes this
    /// function but fails `confine_path` will be denied; a path that fails
    /// this pre-check but passes `confine_path` is denied unnecessarily but
    /// never incorrectly allowed. The true security boundary is
    /// `confine_path`, not this function.
    fn is_path_writable(&self, path: &str) -> bool {
        if self.config.write_patterns.is_empty() {
            return false;
        }
        let trimmed = path.trim().trim_matches('"').trim_matches('`');
        let relative =
            if let Ok(stripped) = std::path::Path::new(trimmed).strip_prefix(&self.vault_dir) {
                stripped.to_string_lossy().to_string()
            } else {
                trimmed.to_string()
            };
        let normalized = Self::normalize_path_components(&relative);
        self.config
            .write_patterns
            .iter()
            .any(|pattern| glob_match(pattern, &normalized))
    }

    /// Confine a path to the vault directory. Relative paths are resolved
    /// against `vault_dir`. Delegates to `normalize_tool_path` which
    /// handles canonicalization and TOCTOU prevention. (#2258)
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
            self.vault_dir.join(trimmed)
        };

        // Delegate to the canonical implementation that correctly returns
        // the canonicalized (symlink-resolved) path, preventing TOCTOU
        // between the security check and subsequent I/O. (#2258)
        let candidate_str = candidate.to_string_lossy().to_string();
        match crate::normalize_tool_path(&candidate_str, &self.vault_dir) {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow!("{}", sanitize_error(&e.to_string()))),
        }
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
        self.record(entry);
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
        self.record(entry);
        ToolProxyResult {
            allowed: false,
            reason: reason.to_string(),
        }
    }

    /// Summarize args for audit log — cap at 200 chars to avoid log bloat.
    fn summarize_args(args_json: &str) -> String {
        let chars: Vec<char> = args_json.chars().collect();
        if chars.len() <= 200 {
            args_json.to_string()
        } else {
            let truncated: String = chars[..200].iter().collect();
            format!("{truncated}…")
        }
    }
}

// ── Agent session ─────────────────────────────────────────────────────────

/// High-level agent session that ties together config, proxy, and lifecycle.
///
/// Phase 2: supports spawning external agent processes with stdout/stderr streaming.
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

    /// Run an external command as an agent subprocess.
    /// Streams stdout/stderr to the provided callbacks.
    /// Returns the exit code on completion.
    pub async fn run_command(
        &self,
        command: &str,
        args: &[String],
        vault_dir: &std::path::Path,
        mut on_stdout: impl FnMut(&str) + Send + 'static,
        mut on_stderr: impl FnMut(&str) + Send + 'static,
    ) -> Result<i32> {
        use tokio::io::{AsyncBufReadExt, BufReader};
        use tokio::process::Command;

        let mut cmd = Command::new(command);
        cmd.kill_on_drop(true);
        cmd.args(args);
        cmd.current_dir(vault_dir);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Set environment variables for the agent
        cmd.env("VAULTPILOT_VAULT_DIR", vault_dir);
        cmd.env("VAULTPILOT_AGENT_NAME", &self.config.name);
        cmd.env(
            "VAULTPILOT_PERMISSION",
            format!("{:?}", self.config.permission),
        );

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn agent process '{}': {}", command, e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("stdout was not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("stderr was not piped"))?;

        let mut stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                on_stdout(&line);
            }
        });

        let mut stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                on_stderr(&line);
            }
        });

        // Apply timeout from resource limits — kill child on timeout to avoid zombie
        let status = tokio::select! {
            result = child.wait() => {
                match result {
                    Ok(status) => status,
                    Err(e) => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        stdout_task.abort();
                        stderr_task.abort();
                        let _ = tokio::join!(stdout_task, stderr_task);
                        anyhow::bail!("failed to wait for agent process: {}", e);
                    }
                }
            }
            _ = tokio::time::sleep(self.config.limits.max_duration) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                // Abort spawned I/O tasks to avoid leaking (#1573)
                stdout_task.abort();
                stderr_task.abort();
                let _ = tokio::join!(stdout_task, stderr_task);
                anyhow::bail!("agent process timed out after {:?}", self.config.limits.max_duration);
            }
        };

        // Wait for output tasks to finish with timeout.
        // If the child spawned grandchildren that inherited our pipes, the pipes
        // never close and the I/O tasks would hang forever. (#2177)
        let io_timeout = Duration::from_secs(5);
        tokio::select! {
            _ = &mut stdout_task => {
                // stdout finished first — abort the still-running stderr
                // task to avoid leaking the JoinHandle (#2404).
                stderr_task.abort();
                let _ = stderr_task.await;
            },
            _ = &mut stderr_task => {
                // stderr finished first — abort the still-running stdout
                // task to avoid leaking the JoinHandle (#2404).
                stdout_task.abort();
                let _ = stdout_task.await;
            },
            _ = tokio::time::sleep(io_timeout) => {
                stdout_task.abort();
                stderr_task.abort();
                let _ = tokio::join!(stdout_task, stderr_task);
            }
        }

        Ok(status.code().unwrap_or(-1))
    }
}

// ── Built-in agent loop (Phase 3.2) ──────────────────────────────────────

use crate::ai;
use crate::storage::StorageContext;

/// Estimate token count when the provider omits usage data (#2542).
/// Falls back to a rough text-length estimate (4 bytes ≈ 1 token) to prevent
/// silent budget bypass when AI providers return null/missing token counts.
fn estimate_tokens(count: Option<usize>, text_len: usize) -> u64 {
    count.unwrap_or_else(|| std::cmp::max(1, text_len / 4)) as u64
}

/// Progress event emitted during agent execution.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// LLM is processing.
    Thinking { step: usize },
    /// Agent is calling a tool.
    ToolCall {
        step: usize,
        tool: String,
        args: String,
    },
    /// Tool execution completed.
    ToolResult {
        step: usize,
        tool: String,
        result_preview: String,
        is_error: bool,
    },
    /// Agent produced the final answer.
    FinalAnswer { text: String },
    /// Write operation needs user approval.
    WriteApprovalNeeded { tool: String, args: String },
    /// Step limit reached before completion.
    StepLimitReached { steps: usize },
    /// Token budget exceeded.
    TokenBudgetExceeded { tokens_used: u64, budget: u64 },
    /// Agent session timed out.
    Timeout,
    /// Error occurred.
    Error { message: String },
    /// A structured execution plan has been generated for user approval (Plan Mode, #2107).
    PlanProposed { plan: ExecutionPlan },
    /// Agent session health degraded — repetition, looping, or no useful output
    /// detected. Frontends should offer a "reset context" option (#3103).
    UnhealthyDetected { reason: String, suggestion: String },
}

/// Result of an agent execution session.
#[derive(Debug, Clone, Serialize)]
pub struct AgentResult {
    pub answer: String,
    pub steps_used: usize,
    pub tokens_used: u64,
    pub audit_log: Vec<AgentAuditEntry>,
}

// ── Plan Mode (#2107) ──────────────────────────────────────────────────────
//
// Plan Mode lets the agent analyse a complex task with a read-only first pass,
// then present a structured step list to the user for approval before any
// mutation is allowed. This forms a two-tier approval system together with the
// existing operation-level WriteApprovalDialog (#1453):
//
//   task-level (plan approve/reject/edit)  →  operation-level (per-write diff)
//
// All plan types are `Serialize + Deserialize` so they can cross the wire to
// the WinUI (C#) and Android (React Native) clients via the HTTP bridge.

/// Whether a task executes immediately or enters Plan Mode first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Execute immediately — the normal autonomous agent loop (#1346).
    #[default]
    Direct,
    /// Generate a plan first; the user must approve it before execution.
    Plan,
    /// Auto-detect: heuristically switch to Plan Mode when the prompt implies
    /// major or destructive changes (batch rewrites, deletions, schema changes).
    /// Resolved via [`should_auto_plan`] at execution time (#3375).
    Auto,
}

/// Heuristic check: does `prompt` imply a major or destructive change that
/// should trigger Plan Mode automatically?
///
/// Detects patterns in both English and Chinese:
/// - Bulk deletions ("delete all", "删除所有")
/// - Batch rewrites ("rewrite all notes", "批量改写")
/// - Structural/schema changes ("restructure", "重构")
/// - Bulk merge/move/archive operations ("merge all", "合并所有")
///
/// Returns `true` when any high-confidence pattern matches. False positives
/// are acceptable (user can reject the plan); false negatives are not
/// dangerous because the user can always pass `--plan` explicitly.
pub fn should_auto_plan(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();

    /// Patterns that strongly indicate a major or destructive change.
    /// Curated to minimise false positives on single-note operations.
    const MAJOR_CHANGE_PATTERNS: &[&str] = &[
        // ── English: bulk deletions ────────────────────────────────────
        "delete all",
        "delete every",
        "remove all",
        "remove every",
        "drop all",
        "purge all",
        "erase all",
        // ── English: batch rewrites / updates ──────────────────────────
        "batch rewrite",
        "batch update",
        "bulk rewrite",
        "bulk update",
        "rewrite all",
        "update all notes",
        "reorganize all",
        // ── English: merge / consolidate / archive ─────────────────────
        "merge all",
        "consolidate all",
        "combine all",
        "archive all",
        "move all notes",
        // ── English: schema / structural changes ───────────────────────
        "schema change",
        "restructure",
        "add column",
        "remove column",
        "change schema",
        // ── Chinese: bulk deletions ────────────────────────────────────
        "删除所有",
        "删除全部",
        "批量删除",
        "清空所有",
        "清空全部",
        // ── Chinese: batch rewrites / updates ──────────────────────────
        "批量改写",
        "批量更新",
        "批量重写",
        "重写所有",
        "重写全部",
        // ── Chinese: merge / consolidate / archive ─────────────────────
        "合并所有",
        "合并全部",
        "归档所有",
        "归档全部",
        "移动所有",
        "移动全部",
        "批量移动",
        // ── Chinese: schema / structural changes ───────────────────────
        "重构所有",
        "重组所有",
        "批量重构",
    ];

    MAJOR_CHANGE_PATTERNS.iter().any(|p| lower.contains(p))
}

/// High-level category of a plan step.
///
/// Matches the `[Search]` / `[Read]` / `[Generate]` / `[Write]` tags used in
/// the plan card display on all three platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepKind {
    /// Retrieve notes via search_notes / list_notes.
    Search,
    /// Read a specific file or note for context.
    Read,
    /// Generate content (draft, summary, answer) — usually the step before Write.
    Generate,
    /// Persist content to the vault via save_note.
    Write,
    /// A step that does not fit the categories above.
    #[default]
    Custom,
}

impl PlanStepKind {
    /// Render the bracketed display tag, e.g. `[Search]`.
    pub fn display_tag(&self) -> &'static str {
        match self {
            Self::Search => "[Search]",
            Self::Read => "[Read]",
            Self::Generate => "[Generate]",
            Self::Write => "[Write]",
            Self::Custom => "[Custom]",
        }
    }

    /// Normalize a free-form model-supplied string into a known kind.
    /// Unknown values fall back to [`PlanStepKind::Custom`].
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "search" | "search_notes" | "list" | "list_notes" => Self::Search,
            "read" | "read_file" | "list_directory" => Self::Read,
            "generate" | "draft" | "summarize" => Self::Generate,
            "write" | "save" | "save_note" => Self::Write,
            _ => Self::Custom,
        }
    }
}

/// Execution status of a single plan step.
///
/// Used for cross-step state tracking so the UI can show live progress as the
/// approved plan is executed by [`run_agent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    /// Not yet started.
    #[default]
    Pending,
    /// Currently executing.
    InProgress,
    /// Finished successfully.
    Done,
    /// Skipped (e.g. user disabled it in a partial-approve, or an earlier
    /// step failed and the plan was aborted).
    Skipped,
    /// Execution failed.
    Failed,
}

fn default_step_estimated_tool_calls() -> u64 {
    1
}
fn default_true() -> bool {
    true
}

/// A single step in an execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// 1-based display index (set when the plan is finalized).
    #[serde(default)]
    pub index: usize,
    /// High-level category — drives the display tag.
    #[serde(default)]
    pub kind: PlanStepKind,
    /// Human-readable description of what the step does and why.
    pub description: String,
    /// Underlying vault tool this step maps to (search_notes, read_file,
    /// save_note, …). `None` for purely generative steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Estimated tool invocations this step contributes.
    #[serde(default = "default_step_estimated_tool_calls")]
    pub estimated_tool_calls: u64,
    /// Whether the user has enabled this step (partial approve).
    /// Disabled steps are skipped during execution.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Current execution status (cross-step state tracking).
    #[serde(default)]
    pub status: PlanStepStatus,
}

impl PlanStep {
    /// Create a new pending, enabled step.
    pub fn new(kind: PlanStepKind, description: impl Into<String>, tool: Option<&str>) -> Self {
        Self {
            index: 0,
            kind,
            description: description.into(),
            tool: tool.map(str::to_string),
            estimated_tool_calls: 1,
            enabled: true,
            status: PlanStepStatus::Pending,
        }
    }
}

/// A structured execution plan produced by Plan Mode.
///
/// Serialized form is consumed by the CLI, WinUI, and Android clients.
/// All fields are plain serializable types so the struct can travel across the
/// HTTP bridge unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    /// The original task / prompt the plan addresses.
    pub task: String,
    /// Ordered list of steps.
    pub steps: Vec<PlanStep>,
    /// Aggregate estimated tool calls across all steps.
    pub estimated_tool_calls: u64,
    /// Aggregate estimated tokens (rough heuristic from the model).
    pub estimated_tokens: u64,
    /// RFC3339 timestamp of when the plan was generated.
    pub generated_at: String,
}

impl ExecutionPlan {
    /// Sum of `estimated_tool_calls` over all steps.
    pub fn total_estimated_tool_calls(&self) -> u64 {
        self.steps.iter().map(|s| s.estimated_tool_calls).sum()
    }

    /// Number of steps that are enabled (used by partial approve).
    pub fn enabled_step_count(&self) -> usize {
        self.steps.iter().filter(|s| s.enabled).count()
    }

    /// Render a human-readable plain-text summary of the plan.
    ///
    /// Reuses the same display format on every platform:
    ///
    /// ```text
    /// ## Execution Plan
    /// 1. [Search] Search vault notes about X (expected 5 notes)
    /// 2. [Read] Read #note-A, #note-B as context
    /// 3. [Write] Save as vault note /Mail/Draft-2026-06-27.md
    /// Estimated tool calls: 4  Estimated tokens: ~3k
    /// ```
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("## Execution Plan\n");
        for step in &self.steps {
            let check = if step.enabled { "[x]" } else { "[ ]" };
            out.push_str(&format!(
                "{}. {} {} {}\n",
                step.index.max(1),
                check,
                step.kind.display_tag(),
                step.description,
            ));
        }
        let tokens_label = format_tokens(self.estimated_tokens);
        out.push_str(&format!(
            "Estimated tool calls: {}  Estimated tokens: {}\n",
            self.estimated_tool_calls, tokens_label,
        ));
        out
    }
}

/// Format a token estimate using the `~3k` shorthand from the issue example.
fn format_tokens(n: u64) -> String {
    if n >= 1000 {
        let k = (n as f64) / 1000.0;
        if (k.fract() - 0.0).abs() < f64::EPSILON {
            format!("~{}k", k as u64)
        } else {
            format!("~{:.1}k", k)
        }
    } else {
        format!("~{n}")
    }
}

/// The user's decision on a generated plan.
///
/// Carries enough information to reconcile the plan into a final executable
/// step list via [`ExecutionPlan::apply_decision`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PlanDecision {
    /// Approve the plan as-is and execute every enabled step.
    Approve,
    /// Reject the plan and cancel the task — no further token consumption.
    Reject,
    /// Execute only the subset of steps whose `enabled` flag is true
    /// (partial approve). Disabled steps are marked `Skipped`.
    PartialApprove,
    /// The user edited the plan; `steps` holds the fully revised list.
    /// The caller should adopt these steps verbatim.
    Edit { steps: Vec<PlanStep> },
}

impl ExecutionPlan {
    /// Reconcile `decision` into a final plan ready for execution.
    ///
    /// - `Approve` / `PartialApprove`: honours each step's `enabled` flag,
    ///   marking disabled steps as `Skipped` and re-indexing the survivors.
    /// - `Reject`: returns a plan whose every step is `Skipped` (the caller
    ///   should treat this as cancellation and not execute).
    /// - `Edit`: replaces `steps` with the user's revised list (disabled steps
    ///   still honoured), then re-indexes.
    ///
    /// The returned plan always has contiguous 1-based `index` values over the
    /// enabled steps so that progress events line up with what the user saw.
    pub fn apply_decision(&self, decision: &PlanDecision) -> ExecutionPlan {
        let (steps, rejected) = match decision {
            PlanDecision::Edit { steps } => (steps.clone(), false),
            PlanDecision::Reject => (self.steps.clone(), true),
            PlanDecision::Approve | PlanDecision::PartialApprove => (self.steps.clone(), false),
        };

        let mut next_index = 1usize;
        let finalized: Vec<PlanStep> = steps
            .into_iter()
            .map(|mut s| {
                if rejected || !s.enabled {
                    s.status = PlanStepStatus::Skipped;
                    s.index = 0; // skipped steps are not numbered
                } else {
                    s.index = next_index;
                    next_index += 1;
                }
                s
            })
            .collect();

        ExecutionPlan {
            task: self.task.clone(),
            steps: finalized,
            estimated_tool_calls: self.estimated_tool_calls,
            estimated_tokens: self.estimated_tokens,
            generated_at: self.generated_at.clone(),
        }
    }

    /// Whether the (post-decision) plan has any executable steps.
    pub fn has_executable_steps(&self) -> bool {
        self.steps
            .iter()
            .any(|s| s.enabled && s.status != PlanStepStatus::Skipped)
    }
}

/// Maximum number of read-only recon steps the plan-generation pass may run.
const PLAN_RECON_MAX_STEPS: usize = 3;

/// Read-only tools the plan recon pass is constrained to.
const PLAN_RECON_TOOLS: &[&str] = &["search_notes", "list_notes", "list_directory", "read_file"];

/// Generate a structured execution plan for `prompt` using a read-only first
/// pass (#2107).
///
/// Flow:
/// 1. Run a **bounded, read-only** mini-agent loop (`PLAN_RECON_MAX_STEPS`)
///    that may only call recon tools. This gathers context without mutating the
///    vault. No write approval is requested because writes are impossible.
/// 2. Ask the model to convert the task + recon transcript into a structured
///    [`ExecutionPlan`] (JSON).
///
/// `on_event` receives the same [`AgentEvent`]s as [`run_agent`] so the UI can
/// show live recon progress. It is ignored for write approvals (the recon pass
/// cannot write), but kept for API symmetry.
pub async fn generate_execution_plan(
    settings: &crate::models::AppSettings,
    context: &StorageContext,
    prompt: &str,
    images: &[String],
    history: &[crate::models::ConversationTurn],
    config: AgentConfig,
    mut on_event: impl FnMut(&AgentEvent),
) -> Result<(ExecutionPlan, Vec<AgentAuditEntry>)> {
    // Force the recon pass to be read-only and constrained to recon tools.
    let recon_config = AgentConfig {
        permission: AgentPermission::ReadOnly,
        allowed_tools: PLAN_RECON_TOOLS.iter().map(|s| s.to_string()).collect(),
        // Keep a tight budget — this is just reconnaissance.
        limits: AgentResourceLimits {
            max_tool_calls: PLAN_RECON_MAX_STEPS as u64,
            max_duration: config
                .limits
                .max_duration
                .min(std::time::Duration::from_secs(60)),
            max_tokens: config.limits.max_tokens,
        },
        ..config.clone()
    };

    let recon_max_duration = recon_config.limits.max_duration;
    let proxy = ToolProxy::new(recon_config, &settings.vault_dir);
    let mut tool_transcripts: Vec<String> = Vec::new();

    for step in 0..PLAN_RECON_MAX_STEPS {
        if proxy.elapsed() > recon_max_duration {
            break;
        }
        on_event(&AgentEvent::Thinking { step: step + 1 });
        let remaining = recon_max_duration.saturating_sub(proxy.elapsed());
        let selection = match tokio::time::timeout(
            remaining,
            ai::select_tool_call(settings, prompt, images, history, &tool_transcripts),
        )
        .await
        {
            Err(_) => break,
            Ok(inner) => inner.map_err(|e| {
                anyhow!(
                    "plan recon LLM call failed at step {}: {}",
                    step + 1,
                    sanitize_error(&e.to_string())
                )
            })?,
        };

        match selection.tool_call {
            ai::AssistantToolCall::None => break,
            tool_call => {
                let tool_name = tool_display_name(&tool_call);
                let args_summary = tool_args_summary(&tool_call);
                let args_json = tool_args_json(&tool_call);

                let check = proxy.check_tool_call(tool_name, &args_json)?;
                if !check.allowed {
                    // Recon tool denied (e.g. path confinement) — record and stop.
                    tool_transcripts.push(format!(
                        "TOOL: {}\nSTATUS: denied\nINPUT:\n{}\nOUTPUT:\ntool error: {}",
                        tool_name, args_summary, check.reason
                    ));
                    break;
                }

                on_event(&AgentEvent::ToolCall {
                    step: step + 1,
                    tool: tool_name.to_string(),
                    args: args_summary.clone(),
                });

                let remaining = recon_max_duration.saturating_sub(proxy.elapsed());
                let (result, is_error) = match tokio::time::timeout(
                    remaining,
                    execute_tool(context, settings, &tool_call),
                )
                .await
                {
                    Ok(res) => res,
                    Err(_) => (format!("tool error: {tool_name} timed out"), true),
                };
                let preview = truncate_preview(&result, 200);
                on_event(&AgentEvent::ToolResult {
                    step: step + 1,
                    tool: tool_name.to_string(),
                    result_preview: preview,
                    is_error,
                });

                tool_transcripts.push(format!(
                    "TOOL: {}\nSTATUS: {}\nINPUT:\n{}\nOUTPUT:\n{}",
                    tool_name,
                    if is_error { "error" } else { "ok" },
                    args_summary,
                    result
                ));
            }
        }
    }

    // Convert task + recon transcript into a structured plan.
    let plan_response = ai::generate_plan(settings, prompt, &tool_transcripts).await?;
    let plan = parse_execution_plan(&plan_response.text, prompt);
    let recon_audit_log = proxy.audit_log();
    Ok((plan, recon_audit_log))
}

/// Parse a model plan response into an [`ExecutionPlan`].
///
/// Tolerant of markdown fences and surrounding prose. If the model output
/// cannot be parsed at all, falls back to a single Custom step describing the
/// task so the user still sees *something* to approve.
pub fn parse_execution_plan(model_text: &str, task: &str) -> ExecutionPlan {
    #[derive(serde::Deserialize)]
    struct RawStep {
        kind: Option<String>,
        tool: Option<String>,
        description: Option<String>,
        #[serde(default = "default_step_estimated_tool_calls")]
        estimated_tool_calls: u64,
    }
    #[derive(serde::Deserialize)]
    struct RawPlan {
        #[serde(default)]
        steps: Vec<RawStep>,
        #[serde(default)]
        estimated_tokens: u64,
    }

    let json_text = extract_first_json_object(model_text);
    let parsed = json_text
        .as_deref()
        .and_then(|j| serde_json::from_str::<RawPlan>(j).ok());

    let (raw_steps, estimated_tokens) = match parsed {
        Some(p) if !p.steps.is_empty() => (p.steps, p.estimated_tokens),
        _ => {
            // Fallback: single Custom step so the user still gets a plan card.
            return fallback_plan(task);
        }
    };

    let mut steps: Vec<PlanStep> = raw_steps
        .into_iter()
        .map(|raw| {
            let kind = raw
                .kind
                .as_deref()
                .map(PlanStepKind::from_str_lossy)
                .unwrap_or_default();
            let description = raw
                .description
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| task.to_string());
            let tool = raw.tool.filter(|t| !t.trim().is_empty());
            PlanStep {
                index: 0,
                kind,
                description,
                tool,
                estimated_tool_calls: raw.estimated_tool_calls.max(1),
                enabled: true,
                status: PlanStepStatus::Pending,
            }
        })
        .collect();

    // Assign 1-based indices.
    for (i, step) in steps.iter_mut().enumerate() {
        step.index = i + 1;
    }

    let estimated_tool_calls = steps.iter().map(|s| s.estimated_tool_calls).sum();

    ExecutionPlan {
        task: task.to_string(),
        steps,
        estimated_tool_calls,
        estimated_tokens,
        generated_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Build a minimal fallback plan when the model output is unparseable.
fn fallback_plan(task: &str) -> ExecutionPlan {
    ExecutionPlan {
        task: task.to_string(),
        steps: vec![PlanStep::new(
            PlanStepKind::Custom,
            format!("Complete the task: {task}"),
            None,
        )],
        estimated_tool_calls: 1,
        estimated_tokens: 0,
        generated_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Extract the first balanced top-level JSON object from `text`.
///
/// Tolerates markdown fences (```` ```json ... ``` ````) and surrounding prose.
/// Returns `None` if no balanced object is found.
fn extract_first_json_object(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    // Find the first '{'.
    let mut start = bytes.iter().position(|&b| b == b'{')?;
    // Walk, tracking string state and brace depth.
    let mut depth: i64 = 0;
    let mut in_string = false;
    let mut backslash_run = 0usize;
    let mut last_close = None;
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                backslash_run += 1;
            } else {
                if c == b'"' && backslash_run.is_multiple_of(2) {
                    in_string = false;
                }
                backslash_run = 0;
            }
        } else if c == b'"' {
            in_string = true;
            backslash_run = 0;
        } else if c == b'{' {
            if depth == 0 {
                start = i;
            }
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                last_close = Some(i);
                break;
            }
        }
        i += 1;
    }
    let end = last_close?;
    let candidate = &text[start..=end];
    // Validate that it round-trips.
    serde_json::from_str::<serde_json::Value>(candidate)
        .ok()
        .map(|_| candidate.to_string())
}
/// Maximum tool-calling rounds in the agent loop.
const DEFAULT_MAX_STEPS: usize = 20;

/// Health tracker that detects agent degradation patterns (#3103):
/// repetition (same tool/call over and over), looping (same file/args),
/// and silent failure (no useful operations despite multiple attempts).
///
/// `successful_ops` counts every non-error result (preserving the original
/// intent of #3103). `useful_ops` additionally requires the result to be
/// non-trivial — non-error AND not empty/whitespace-only AND at least
/// `USEFUL_RESULT_MIN_CHARS` characters after trimming (#3118).
///
/// This split is necessary because the original `successful_ops == 0`
/// condition made the silent_failure branch unreachable: every non-error
/// result incremented `successful_ops`, so the only way to keep it at 0
/// was to have every step error — but in that case the error_spiral check
/// (branch 2) already fires at step 3, preventing the silent_failure branch
/// from ever being reached (#3118).
#[derive(Debug, Clone)]
struct SessionHealthTracker {
    /// Map from (tool_name, args_preview) to consecutive call count.
    recent: std::collections::HashMap<String, u32>,
    /// Consecutive steps ending in tool errors.
    consecutive_errors: u32,
    /// Count of successful (non-error) tool results.
    successful_ops: u32,
    /// Count of *useful* results: non-error AND non-trivial output (#3118).
    /// Used by the silent_failure branch so that sequences of
    /// "successful-but-empty" calls (e.g. agent receives "ok" / "" / short
    /// acks without making real progress) still trigger the detector.
    useful_ops: u32,
    /// Steps processed so far.
    total_steps: u32,
    /// Whether unhealthy event already emitted this session.
    unhealthy_emitted: bool,
}

/// Minimum trimmed length a non-error tool result must have to count as a
/// "useful" operation for silent_failure detection (#3118).
///
/// Conservative threshold: catches trivial acks like "", "ok", "1", "[]" —
/// which indicate the agent made a tool call that produced no real progress —
/// while still counting real results (file contents, search hits, etc.).
const USEFUL_RESULT_MIN_CHARS: usize = 5;

impl SessionHealthTracker {
    fn new() -> Self {
        Self {
            recent: std::collections::HashMap::new(),
            consecutive_errors: 0,
            successful_ops: 0,
            useful_ops: 0,
            total_steps: 0,
            unhealthy_emitted: false,
        }
    }

    /// Record a tool call and check for unhealthy patterns.
    /// Returns Some(reason, suggestion) if unhealthy, None otherwise.
    ///
    /// `result_text` is the tool's full output string (used to detect the
    /// silent_failure pattern where every call is "successful" but produces
    /// no useful content — #3118).
    fn record_and_check(
        &mut self,
        tool_name: &str,
        args_summary: &str,
        is_error: bool,
        result_text: &str,
    ) -> Option<(String, String)> {
        if self.unhealthy_emitted {
            return None;
        }

        self.total_steps += 1;

        // 1. Repetition / looping detection: same tool + args 4+ times consecutively
        let key = format!("{}::{}", tool_name, args_summary);
        let count = self.recent.entry(key.clone()).or_insert(0);
        *count += 1;
        if *count >= 4 {
            self.unhealthy_emitted = true;
            return Some((
                format!(
                    "Repetition detected: {} called with same arguments {} times consecutively",
                    tool_name, count
                ),
                "Agent has been calling the same tool repeatedly. Consider restarting with a more specific prompt.".to_string(),
            ));
        }

        // Reset counts for all OTHER keys to detect only consecutive repetition
        self.recent.retain(|k, _| k == &key);

        // 2. Error spiral: 3+ consecutive errors with zero successes
        if is_error {
            self.consecutive_errors += 1;
            if self.consecutive_errors >= 3 && self.successful_ops == 0 && self.total_steps >= 3 {
                self.unhealthy_emitted = true;
                return Some((
                    format!(
                        "{} consecutive tool errors with no successful operations",
                        self.consecutive_errors
                    ),
                    "Agent hasn't completed any successful operation. Reset context and try a different prompt, or check tool permissions.".to_string(),
                ));
            }
        } else {
            self.consecutive_errors = 0;
            self.successful_ops += 1;
            // Only count genuinely useful output: non-trivial length after
            // trimming. This is the signal that lets the silent_failure
            // branch actually fire on sequences of "successful-but-empty"
            // acks (e.g. agent repeatedly gets "" or "ok" back) — #3118.
            if result_text.trim().len() >= USEFUL_RESULT_MIN_CHARS {
                self.useful_ops += 1;
            }
        }

        // 3. Silent failure: 6+ steps with zero *useful* operations (#3118).
        //
        // Previously this used `successful_ops == 0`, but that was unreachable:
        // - all errors  → error_spiral (branch 2) returns at step 3
        // - any success → successful_ops > 0 → condition never true
        // Using `useful_ops` instead catches the intended case where the
        // agent is not erroring out but is also not producing any meaningful
        // output (the original #3103 spec's "silent failure" mode).
        if self.useful_ops == 0 && self.total_steps >= 6 {
            self.unhealthy_emitted = true;
            return Some((
                format!(
                    "{} total steps with zero useful operations",
                    self.total_steps
                ),
                "Agent is producing no useful output. Check whether the tools it needs are available, its permissions are too restrictive, or the prompt is too vague.".to_string(),
            ));
        }

        None
    }
}

/// Run an autonomous agent loop: prompt → LLM → tool call → execute → repeat.
///
/// The agent uses `select_tool_call` to decide which tool to invoke, executes
/// it through the sandboxed `ToolProxy`, and feeds results back to the LLM
/// until it produces a final answer or hits a resource limit.
///
/// `on_event` is called for every significant progress change.
/// For write operations, `on_event` receives `WriteApprovalNeeded` — the
/// callback should return `true` to approve or `false` to deny.
/// For Plan Mode, `on_event` receives `PlanProposed` with the generated plan.
///
/// `on_plan_decision` is called when Plan Mode generates a plan and needs the
/// user to approve, reject, or edit it. Only invoked when `config.execution_mode`
/// is `ExecutionMode::Plan`.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent(
    settings: &crate::models::AppSettings,
    context: &StorageContext,
    prompt: &str,
    images: &[String],
    history: &[crate::models::ConversationTurn],
    config: AgentConfig,
    mut on_event: impl FnMut(&AgentEvent) -> bool,
    mut on_plan_decision: impl FnMut(&ExecutionPlan) -> PlanDecision,
) -> Result<AgentResult> {
    let proxy = ToolProxy::new(config.clone(), &settings.vault_dir);
    let max_steps = if config.limits.max_tool_calls > 0
        && (config.limits.max_tool_calls as usize) < DEFAULT_MAX_STEPS
    {
        config.limits.max_tool_calls as usize
    } else {
        DEFAULT_MAX_STEPS
    };
    let token_budget = config.limits.max_tokens;

    let mut tool_transcripts: Vec<String> = Vec::new();
    let mut total_tokens: u64 = 0;
    let mut health_tracker = SessionHealthTracker::new();

    // Track why the loop exited so we can emit the correct event
    // and avoid spurious StepLimitReached / extra LLM calls (#1689).
    enum ExitReason {
        StepLimit,
        Timeout,
        TokenBudget,
    }
    let mut exit_reason = ExitReason::StepLimit;
    let mut actual_steps = 0usize;

    // ── Plan Mode (#2107, #3375): generate plan, get user decision, then execute ─
    // #3375: ExecutionMode::Auto resolves to Plan when should_auto_plan detects
    //        major/destructive changes in the prompt; otherwise falls through to Direct.
    let enter_plan_mode = config.execution_mode == ExecutionMode::Plan
        || (config.execution_mode == ExecutionMode::Auto && should_auto_plan(prompt));
    if enter_plan_mode {
        let (plan, recon_audit_log) = generate_execution_plan(
            settings,
            context,
            prompt,
            images,
            history,
            config.clone(),
            |event| {
                on_event(event);
            },
        )
        .await?;

        on_event(&AgentEvent::PlanProposed { plan: plan.clone() });

        let decision = on_plan_decision(&plan);
        let finalized = plan.apply_decision(&decision);

        if !finalized.has_executable_steps() {
            return Ok(AgentResult {
                answer: if matches!(decision, PlanDecision::Reject) {
                    "[Plan was rejected — no steps to execute]".to_string()
                } else {
                    "[Plan has no executable steps after editing]".to_string()
                },
                steps_used: 0,
                tokens_used: 0,
                audit_log: recon_audit_log,
            });
        }

        // Preserve recon-pass audit entries in the main proxy so they
        // are included in the final AgentResult (#2425).
        proxy.merge_audit_log(recon_audit_log);
    }

    for step in 0..max_steps {
        // Timeout check
        if proxy.elapsed() > config.limits.max_duration {
            on_event(&AgentEvent::Timeout);
            exit_reason = ExitReason::Timeout;
            break;
        }

        on_event(&AgentEvent::Thinking { step: step + 1 });

        // Ask LLM what tool to call (with per-step timeout — #1574, #2142)
        let remaining = config.limits.max_duration.saturating_sub(proxy.elapsed());
        let selection = match tokio::time::timeout(
            remaining,
            ai::select_tool_call(settings, prompt, images, history, &tool_transcripts),
        )
        .await
        {
            Err(_) => {
                if proxy.elapsed() >= config.limits.max_duration {
                    // Session-level timeout — go through graceful path (#2142)
                    on_event(&AgentEvent::Timeout);
                    exit_reason = ExitReason::Timeout;
                    break;
                }
                return Err(anyhow!("LLM call timed out at step {}", step + 1));
            }
            Ok(inner) => inner.map_err(|e| {
                anyhow!(
                    "LLM call failed at step {}: {}",
                    step + 1,
                    sanitize_error(&e.to_string())
                )
            })?,
        };

        total_tokens += estimate_tokens(selection.usage.input_tokens, 0)
            + estimate_tokens(selection.usage.output_tokens, 0);

        // Token budget check
        if token_budget > 0 && total_tokens > token_budget {
            on_event(&AgentEvent::TokenBudgetExceeded {
                tokens_used: total_tokens,
                budget: token_budget,
            });
            exit_reason = ExitReason::TokenBudget;
            break;
        }

        match selection.tool_call {
            ai::AssistantToolCall::None => {
                // LLM decided no more tools needed — generate final answer (with timeout — #1574, #2142)
                let remaining = config.limits.max_duration.saturating_sub(proxy.elapsed());
                let answer = if tool_transcripts.is_empty() {
                    match tokio::time::timeout(
                        remaining,
                        crate::ai::answer_question(settings, prompt, &[], images, history),
                    )
                    .await
                    {
                        Err(_) if proxy.elapsed() >= config.limits.max_duration => {
                            // Session-level timeout — graceful fallback (#2142)
                            return Ok(AgentResult {
                                answer:
                                    "[Agent session timed out before generating a final answer]"
                                        .to_string(),
                                steps_used: step + 1,
                                tokens_used: total_tokens,
                                audit_log: proxy.audit_log(),
                            });
                        }
                        Err(_) => {
                            return Err(anyhow!("final answer LLM call timed out"));
                        }
                        Ok(inner) => inner.map_err(|e| {
                            anyhow!("final answer failed: {}", sanitize_error(&e.to_string()))
                        })?,
                    }
                } else {
                    match tokio::time::timeout(
                        remaining,
                        crate::ai::answer_after_tools(
                            settings,
                            prompt,
                            &tool_transcripts,
                            &[],
                            history,
                        ),
                    )
                    .await
                    {
                        Err(_) if proxy.elapsed() >= config.limits.max_duration => {
                            // Session-level timeout — graceful fallback (#2142)
                            let fallback = format!(
                                "[Agent session timed out; {} tool result(s) were collected but a final answer could not be generated]",
                                tool_transcripts.len()
                            );
                            return Ok(AgentResult {
                                answer: fallback,
                                steps_used: step + 1,
                                tokens_used: total_tokens,
                                audit_log: proxy.audit_log(),
                            });
                        }
                        Err(_) => {
                            return Err(anyhow!("final answer LLM call timed out"));
                        }
                        Ok(inner) => inner.map_err(|e| {
                            anyhow!("final answer failed: {}", sanitize_error(&e.to_string()))
                        })?,
                    }
                };
                total_tokens += estimate_tokens(answer.usage.input_tokens, answer.answer.len())
                    + estimate_tokens(answer.usage.output_tokens, answer.answer.len());
                on_event(&AgentEvent::FinalAnswer {
                    text: answer.answer.clone(),
                });
                return Ok(AgentResult {
                    answer: answer.answer,
                    steps_used: step + 1,
                    tokens_used: total_tokens,
                    audit_log: proxy.audit_log(),
                });
            }
            tool_call => {
                let tool_name = tool_display_name(&tool_call);
                let args_summary = tool_args_summary(&tool_call);
                // Build proper JSON args for sandbox checks (#1604).
                // `check_tool_call` / `extract_path_args` expect valid JSON
                // but `tool_args_summary` returns human-readable "key=value".
                let args_json = tool_args_json(&tool_call);

                // ToolProxy sandbox check
                let check = proxy.check_tool_call(tool_name, &args_json)?;
                if !check.allowed {
                    tool_transcripts.push(format!(
                        "TOOL: {}\nSTATUS: denied\nINPUT:\n{}\nOUTPUT:\ntool error: {}",
                        tool_name, args_summary, check.reason
                    ));
                    continue;
                }

                // Write approval callback
                if ToolProxy::is_write_tool(tool_name) {
                    // Build approval args: full JSON with content + current content for diff
                    let approval_args = if tool_name == "save_note" {
                        let mut args_value: serde_json::Value =
                            match serde_json::from_str(&args_json) {
                                Ok(v) => v,
                                Err(e) => {
                                    warn!(
                                        tool = tool_name,
                                        error = %e,
                                        "failed to parse approval args JSON for '{}': {}",
                                        tool_name, e,
                                    );
                                    serde_json::Value::Null
                                }
                            };
                        if let Some(obj) = args_value.as_object_mut() {
                            if let ai::AssistantToolCall::SaveNote { note_id, .. } = &tool_call {
                                match crate::storage::load_note_async(context, note_id).await {
                                    Ok(existing) => {
                                        obj.insert(
                                            "currentContent".to_string(),
                                            serde_json::json!(existing.body),
                                        );
                                        obj.insert(
                                            "currentTitle".to_string(),
                                            serde_json::json!(existing.meta.title),
                                        );
                                        obj.insert(
                                            "currentPath".to_string(),
                                            serde_json::json!(existing.meta.path),
                                        );
                                    }
                                    _ => {
                                        obj.insert(
                                            "currentContent".to_string(),
                                            serde_json::json!(""),
                                        );
                                    }
                                }
                            }
                        }
                        args_value.to_string()
                    } else {
                        args_json.clone()
                    };

                    let approved = on_event(&AgentEvent::WriteApprovalNeeded {
                        tool: tool_name.to_string(),
                        args: approval_args,
                    });
                    if !approved {
                        tool_transcripts.push(format!(
                            "TOOL: {}\nSTATUS: denied\nINPUT:\n{}\nOUTPUT:\ntool error: write denied by user",
                            tool_name, args_summary
                        ));
                        continue;
                    }
                }

                on_event(&AgentEvent::ToolCall {
                    step: step + 1,
                    tool: tool_name.to_string(),
                    args: args_summary.clone(),
                });

                // Execute the tool (with timeout — #1602)
                let remaining = config.limits.max_duration.saturating_sub(proxy.elapsed());
                let (result, is_error) = match tokio::time::timeout(
                    remaining,
                    execute_tool(context, settings, &tool_call),
                )
                .await
                {
                    Ok(res) => res,
                    Err(_) => (
                        format!(
                            "tool error: {} timed out after {}s",
                            tool_name,
                            remaining.as_secs()
                        ),
                        true,
                    ),
                };
                let preview = truncate_preview(&result, 200);

                on_event(&AgentEvent::ToolResult {
                    step: step + 1,
                    tool: tool_name.to_string(),
                    result_preview: preview,
                    is_error,
                });

                // ── Session health check (#3103) ──────────────────────────
                if let Some((reason, suggestion)) =
                    health_tracker.record_and_check(tool_name, &args_summary, is_error, &result)
                {
                    on_event(&AgentEvent::UnhealthyDetected { reason, suggestion });
                }

                tool_transcripts.push(format!(
                    "TOOL: {}\nSTATUS: {}\nINPUT:\n{}\nOUTPUT:\n{}",
                    tool_name,
                    if is_error { "error" } else { "ok" },
                    args_summary,
                    result
                ));
            }
        }
        actual_steps = step + 1;
    }

    // Exited loop without a final answer — generate one from accumulated results
    // Only emit StepLimitReached and make a final LLM call if the loop actually
    // exhausted all steps. For timeout / token-budget exits the correct event
    // was already emitted above and an extra LLM call would waste tokens (#1689).
    if matches!(exit_reason, ExitReason::Timeout | ExitReason::TokenBudget) {
        let is_timeout = matches!(exit_reason, ExitReason::Timeout);
        // Try to generate a final answer from accumulated tool transcripts
        // even when we hit a limit, so the user gets a useful response (#1845).
        if !tool_transcripts.is_empty() {
            let remaining = config
                .limits
                .max_duration
                .saturating_sub(proxy.elapsed())
                .min(std::time::Duration::from_secs(10));
            if remaining > std::time::Duration::from_secs(1) {
                let answer_result = tokio::time::timeout(
                    remaining,
                    crate::ai::answer_after_tools(
                        settings,
                        prompt,
                        &tool_transcripts,
                        &[],
                        history,
                    ),
                )
                .await;
                if let Ok(Ok(answer)) = answer_result {
                    total_tokens += estimate_tokens(answer.usage.input_tokens, answer.answer.len())
                        + estimate_tokens(answer.usage.output_tokens, answer.answer.len());
                    return Ok(AgentResult {
                        answer: answer.answer,
                        steps_used: actual_steps,
                        tokens_used: total_tokens,
                        audit_log: proxy.audit_log(),
                    });
                }
            }
        }
        // Fallback: return a descriptive message instead of empty string
        let reason = if is_timeout {
            "Agent session timed out"
        } else {
            "Agent token budget exceeded"
        };
        let fallback = if tool_transcripts.is_empty() {
            format!("[{reason} before any tool results were available]")
        } else {
            format!("[{reason}; {} tool result(s) were collected but a final answer could not be generated]",
                tool_transcripts.len())
        };
        return Ok(AgentResult {
            answer: fallback,
            steps_used: actual_steps,
            tokens_used: total_tokens,
            audit_log: proxy.audit_log(),
        });
    }

    on_event(&AgentEvent::StepLimitReached { steps: max_steps });
    // Final LLM call with per-step timeout (#1574)
    // Graceful fallback on timeout/error, consistent with timeout/token-budget paths (#1902).
    let remaining = config
        .limits
        .max_duration
        .saturating_sub(proxy.elapsed())
        .min(std::time::Duration::from_secs(10));
    let answer_result = if remaining > std::time::Duration::from_secs(1) {
        if tool_transcripts.is_empty() {
            tokio::time::timeout(
                remaining,
                crate::ai::answer_question(settings, prompt, &[], images, history),
            )
            .await
            .ok()
            .and_then(|r| r.ok())
        } else {
            tokio::time::timeout(
                remaining,
                crate::ai::answer_after_tools(settings, prompt, &tool_transcripts, &[], history),
            )
            .await
            .ok()
            .and_then(|r| r.ok())
        }
    } else {
        None
    };
    if let Some(answer) = answer_result {
        total_tokens += estimate_tokens(answer.usage.input_tokens, answer.answer.len())
            + estimate_tokens(answer.usage.output_tokens, answer.answer.len());
        return Ok(AgentResult {
            answer: answer.answer,
            steps_used: max_steps,
            tokens_used: total_tokens,
            audit_log: proxy.audit_log(),
        });
    }
    // Fallback: return a descriptive message instead of propagating an error (#1902).
    let fallback = if tool_transcripts.is_empty() {
        "[Step limit reached before any tool results were available]".to_string()
    } else {
        format!(
            "[Step limit reached; {} tool result(s) were collected but a final answer could not be generated]",
            tool_transcripts.len()
        )
    };
    Ok(AgentResult {
        answer: fallback,
        steps_used: max_steps,
        tokens_used: total_tokens,
        audit_log: proxy.audit_log(),
    })
}

/// Execute a tool call against the vault storage layer.
/// Returns (output, is_error).
async fn execute_tool(
    context: &StorageContext,
    settings: &crate::models::AppSettings,
    tool_call: &ai::AssistantToolCall,
) -> (String, bool) {
    use crate::storage::{load_context_notes_async, load_recent_notes_for_overview_async};

    // Wraps anyhow/io/rusqlite errors in a tool-error message with
    // sanitization so internal paths/credentials don't leak to the LLM.
    fn tool_err(e: &anyhow::Error) -> String {
        format!("tool error: {}", vaultpilot_lib::sanitize_error(&e.to_string()))
    }

    fn task_join_err(e: &tokio::task::JoinError) -> String {
        format!("tool error: task join failed: {}", vaultpilot_lib::sanitize_error(&e.to_string()))
    }

    match tool_call {
        ai::AssistantToolCall::None => ("no tool selected".into(), false),
        ai::AssistantToolCall::SearchNotes { query, limit } => {
            match load_context_notes_async(context, query, &[], limit.saturating_mul(3).max(8))
                .await
            {
                Ok(docs) => {
                    let summary = docs
                        .iter()
                        .take(*limit)
                        .map(|d| format!("- {} ({})", d.meta.title, d.meta.path))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if summary.is_empty() {
                        ("No matching notes found.".into(), false)
                    } else {
                        (
                            format!("Found {} notes:\n{}", docs.len().min(*limit), summary),
                            false,
                        )
                    }
                }
                Err(e) => (tool_err(&e), true),
            }
        }
        ai::AssistantToolCall::ListNotes { limit } => {
            match load_recent_notes_for_overview_async(context, *limit).await {
                Ok(docs) => {
                    let summary = docs
                        .iter()
                        .map(|d| format!("- {} ({})", d.meta.title, d.meta.path))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if summary.is_empty() {
                        ("No notes in vault.".into(), false)
                    } else {
                        (format!("{} notes:\n{}", docs.len(), summary), false)
                    }
                }
                Err(e) => (tool_err(&e), true),
            }
        }
        ai::AssistantToolCall::ListDirectory { path } => {
            let vault_root = PathBuf::from(&settings.vault_dir);
            let path_owned = path.clone();
            match tokio::task::spawn_blocking(move || {
                list_directory_for_agent(&path_owned, &vault_root)
            })
            .await
            {
                Ok(Ok(output)) => (output, false),
                Ok(Err(e)) => (tool_err(&e), true),
                Err(e) => (task_join_err(&e), true),
            }
        }
        ai::AssistantToolCall::ReadFile { path } => {
            let vault_root = PathBuf::from(&settings.vault_dir);
            let path_owned = path.clone();
            match tokio::task::spawn_blocking(move || read_file_for_agent(&path_owned, &vault_root))
                .await
            {
                Ok(Ok(output)) => (output, false),
                Ok(Err(e)) => (tool_err(e), true),
                Err(e) => (task_join_err(&e), true),
            }
        }
        ai::AssistantToolCall::SaveNote { draft, note_id } => {
            use crate::storage::save_note_with_images_async;
            // Load existing note for backup and to preserve original created_at (#3350)
            let existing_note = crate::storage::load_note_async(context, note_id).await.ok();
            if let Some(ref existing) = existing_note {
                crate::orchestration::write::WRITE_TRACKER.record_backup(existing);
            }
            // Preserve created_at from existing note; fall back to now() for new notes
            let created_at = existing_note
                .as_ref()
                .map(|n| n.meta.created_at.clone())
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
            let short_id: String = note_id.chars().take(8).collect();
            // Sanitize: keep only word chars so LLM-supplied note_id cannot
            // inject path separators or special chars into the filename.
            let safe_id: String = short_id
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            let short_id = if safe_id.is_empty() { "note" } else { &safe_id };
            let max_slug_len = 255usize.saturating_sub(short_id.len()).saturating_sub(4); // "-" + ".md"
            let slug = slugify(&draft.title);
            let mut byte_count = 0usize;
            let slug: String = slug
                .chars()
                .take_while(|c| {
                    let n = c.len_utf8();
                    if byte_count + n <= max_slug_len {
                        byte_count += n;
                        true
                    } else {
                        false
                    }
                })
                .collect();
            let note = crate::models::NoteDocument {
                meta: crate::models::NoteMeta {
                    id: note_id.clone(),
                    title: draft.title.clone(),
                    path: format!("{}-{}.md", slug, short_id),
                    tags: draft.tags.clone(),
                    keywords: draft.keywords.clone(),
                    platform: draft.platform.clone(),
                    board: draft.board.clone(),
                    kernel: draft.kernel.clone(),
                    status: draft.status.clone(),
                    source: draft.source.clone(),
                    summary: draft.summary.clone(),
                    created_at,
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    ..Default::default()
                },
                body: draft.body.clone(),
                ..Default::default()
            };
            match save_note_with_images_async(context, note, &[]).await {
                Ok(saved) => (
                    format!("Note saved: {} at {}", saved.meta.title, saved.meta.path),
                    false,
                ),
                Err(e) => (tool_err(&e), true),
            }
        }
        ai::AssistantToolCall::Custom { name, args } => {
            let tool = settings
                .custom_tools
                .iter()
                .find(|t| t.name == *name && t.enabled);
            match tool {
                Some(tool) => {
                    let vault_dir = PathBuf::from(&settings.vault_dir);
                    let args_str =
                        serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string());
                    match tool.execute(&args_str, &vault_dir).await {
                        Ok(output) => (output, false),
                        Err(e) => (
                            format!(
                                "tool error: custom tool '{}' failed: {}",
                                name,
                                vaultpilot_lib::sanitize_error(&e.to_string())
                            ),
                            true,
                        ),
                    }
                }
                None => (
                    format!("tool error: custom tool '{}' not found or disabled", name),
                    true,
                ),
            }
        }
    }
}

fn list_directory_for_agent(path: &str, vault_root: &Path) -> Result<String> {
    // Issue #2023: Cap entries to avoid wasting tokens on huge directories.
    const MAX_AGENT_DIR_ENTRIES: usize = 100;

    let directory = crate::normalize_tool_path(path, vault_root)?;
    if !directory.exists() {
        return Err(anyhow!("path does not exist: {}", path));
    }
    if !directory.is_dir() {
        return Err(anyhow!("path is not a directory: {}", path));
    }
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    // Issue #2022: Collect per-entry errors instead of failing the whole listing.
    for entry in std::fs::read_dir(&directory)? {
        match entry {
            Ok(e) => {
                let name = e.file_name().to_string_lossy().to_string();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                entries.push(format!("{}{}", name, if is_dir { "/" } else { "" }));
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    entries.sort();

    let total = entries.len();
    let truncated = total > MAX_AGENT_DIR_ENTRIES;
    entries.truncate(MAX_AGENT_DIR_ENTRIES);

    let mut output = entries.join("\n");
    if truncated {
        output.push_str(&format!(
            "\n\n(Showing first {} of {} entries. Use a subdirectory path to see more.)",
            MAX_AGENT_DIR_ENTRIES, total
        ));
    }
    if !errors.is_empty() {
        output.push_str(&format!(
            "\n\n⚠ {} entries could not be read due to permission or I/O errors:\n{}",
            errors.len(),
            errors
                .iter()
                .take(10)
                .map(|e| format!("- {}", e))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    Ok(output)
}

fn read_file_for_agent(path: &str, vault_root: &Path) -> Result<String> {
    let file_path = crate::normalize_tool_path(path, vault_root)?;
    if !file_path.exists() {
        return Err(anyhow!("file does not exist: {}", path));
    }
    // Read file with size limit to prevent OOM (TOCTOU-safe: no pre-check of metadata)
    const MAX_FILE_SIZE: u64 = 1024 * 1024; // 1 MB
    let mut file = std::fs::File::open(&file_path)?;
    let mut buf = Vec::new();
    file.by_ref().take(MAX_FILE_SIZE).read_to_end(&mut buf)?;
    // Probe one extra byte to check if file exceeds the size limit.
    let mut probe = [0u8; 1];
    let remaining = file.read(&mut probe)?;
    if remaining > 0 {
        return Err(anyhow!(
            "file too large (>{} bytes): {}",
            MAX_FILE_SIZE,
            path
        ));
    }
    // Convert bytes to UTF-8. Using read_to_end + from_utf8 instead of
    // read_to_string + take avoids misleading "invalid UTF-8" errors
    // when the 1 MB boundary splits a multi-byte character (#2141).
    let content =
        String::from_utf8(buf).map_err(|_| anyhow!("file is not valid UTF-8: {}", path))?;
    // Cap at 50KB to prevent token explosion
    const MAX_READ: usize = 50 * 1024;
    if content.len() > MAX_READ {
        // Find the largest char boundary at or before MAX_READ to avoid
        // panicking on a mid-char slice (#1536).
        let mut end = MAX_READ;
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = &content[..end];
        Ok(format!("{}\n[... truncated at {} bytes]", truncated, end))
    } else {
        Ok(content)
    }
}

fn slugify(title: &str) -> String {
    // Delegate to the single canonical slugify in crate::utils.
    // (#3167 — remove duplicate, unify with utils.rs canonical impl)
    crate::utils::slugify(title)
}

fn tool_display_name(tool: &ai::AssistantToolCall) -> &str {
    match tool {
        ai::AssistantToolCall::None => "none",
        ai::AssistantToolCall::SearchNotes { .. } => "search_notes",
        ai::AssistantToolCall::ListNotes { .. } => "list_notes",
        ai::AssistantToolCall::ListDirectory { .. } => "list_directory",
        ai::AssistantToolCall::ReadFile { .. } => "read_file",
        ai::AssistantToolCall::SaveNote { .. } => "save_note",
        ai::AssistantToolCall::Custom { name, .. } => name.as_str(),
    }
}

fn tool_args_summary(tool: &ai::AssistantToolCall) -> String {
    match tool {
        ai::AssistantToolCall::None => "{}".into(),
        ai::AssistantToolCall::SearchNotes { query, limit } => {
            format!("query={} limit={}", query, limit)
        }
        ai::AssistantToolCall::ListNotes { limit } => format!("limit={}", limit),
        ai::AssistantToolCall::ListDirectory { path } => format!("path={}", path),
        ai::AssistantToolCall::ReadFile { path } => format!("path={}", path),
        ai::AssistantToolCall::SaveNote { draft, .. } => {
            format!("title={} body_len={}", draft.title, draft.body.len())
        }
        ai::AssistantToolCall::Custom { name, args } => {
            format!("name={} args={}", name, args)
        }
    }
}

/// Build a proper JSON args string for a tool call, suitable for
/// `check_tool_call` / `extract_path_arg` which expect valid JSON (#1604).
fn tool_args_json(tool: &ai::AssistantToolCall) -> String {
    match tool {
        ai::AssistantToolCall::None => "{}".into(),
        ai::AssistantToolCall::SearchNotes { query, limit } => {
            serde_json::json!({"query": query, "limit": limit}).to_string()
        }
        ai::AssistantToolCall::ListNotes { limit } => {
            serde_json::json!({"limit": limit}).to_string()
        }
        ai::AssistantToolCall::ListDirectory { path } => {
            serde_json::json!({"path": path}).to_string()
        }
        ai::AssistantToolCall::ReadFile { path } => serde_json::json!({"path": path}).to_string(),
        ai::AssistantToolCall::SaveNote { draft, note_id } => {
            let short_id: String = note_id.chars().take(8).collect();
            serde_json::json!({"path": format!("{}-{}.md", slugify(&draft.title), short_id),
                              "title": draft.title,
                              "body": truncate_preview(&draft.body, 500)})
            .to_string()
        }
        ai::AssistantToolCall::Custom { name: _, args } => args.to_string(),
    }
}

fn truncate_preview(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = chars[..max_chars].iter().collect();
        format!("{truncated}…")
    }
}

// ── Glob matching ────────────────────────────────────────────────────────

/// Simple glob pattern matching. Supports:
/// - `*` matches any sequence of characters (except path separator)
/// - `**` matches any sequence including path separators
/// - `?` matches a single character
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    glob_match_inner(&pattern_chars, &text_chars)
}

fn glob_match_inner(pattern: &[char], text: &[char]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi = usize::MAX; // sentinel: no star matched yet
    let mut star_ti = 0;
    let mut star_resume_pi = 0; // where to resume pattern after * on backtrack
                                // Separate backtrack state for ** so it's not overwritten by a subsequent
                                // single * that later fails on a path separator (#2088).
    let mut star_star_pi = usize::MAX;
    let mut star_star_ti = 0;
    let mut star_star_resume_pi = 0;

    while ti < text.len() {
        if pi < pattern.len()
            && (pattern[pi] == text[ti]
                || (pattern[pi] == '?' && text[ti] != '/' && text[ti] != '\\'))
        {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            // Handle ** (matches everything including /)
            if pi + 1 < pattern.len() && pattern[pi + 1] == '*' {
                star_star_pi = pi;
                star_star_ti = ti;
                // Save the original star position before advancing pi,
                // so backtracking works correctly for bare ** patterns.
                star_pi = pi;
                star_ti = ti;
                pi += 2;
                // ** at a segment boundary: skip the following '/' so that
                // ** can match zero path segments (e.g. "**/suffix" ~ "suffix")
                if pi < pattern.len() && pattern[pi] == '/' {
                    pi += 1;
                }
                star_star_resume_pi = pi;
                star_resume_pi = pi;
            } else {
                star_pi = pi;
                star_ti = ti;
                pi += 1;
                star_resume_pi = pi;
            }
        } else if star_pi != usize::MAX {
            // Backtrack to the last star
            if star_pi + 1 < pattern.len() && pattern[star_pi + 1] == '*' {
                // ** matches everything
                star_ti += 1;
                ti = star_ti;
                pi = star_resume_pi;
            } else if text[ti] != '/' && text[ti] != '\\' {
                // * doesn't match path separators
                star_ti += 1;
                ti = star_ti;
                pi = star_pi + 1;
            } else if star_star_pi != usize::MAX {
                // * backtrack hit a path separator — fall back to **
                // backtrack state instead of giving up (#2088).
                star_star_ti += 1;
                ti = star_star_ti;
                pi = star_star_resume_pi;
                // Carry the ** state forward as the current star state
                // so further backtrack attempts continue from **.
                star_pi = star_star_pi;
                star_ti = star_star_ti;
                star_resume_pi = star_star_resume_pi;
            } else {
                return false;
            }
        } else {
            return false;
        }
    }

    // Consume trailing stars
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }

    pi == pattern.len()
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
    fn event_sink_receives_live_tool_call_events() {
        // #1905 — the transparency panel needs a real-time stream of tool calls,
        // not just the post-hoc audit log. Verify `with_event_sink` forwards an
        // `AgentAuditEntry` the moment each call is allowed/denied.
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());

        let events: Arc<Mutex<Vec<AgentAuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let proxy = ToolProxy::new(config.clone(), &tmp).with_event_sink(Arc::new(move |e| {
            sink.lock().unwrap().push(e);
        }));

        proxy
            .check_tool_call("search_notes", r#"{"query":"x"}"#)
            .unwrap();
        proxy
            .check_tool_call("save_note", r#"{"path":"y.md"}"#)
            .unwrap();

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 2, "sink should receive one event per call");
        assert!(captured[0].allowed);
        assert_eq!(captured[0].tool, "search_notes");
        assert!(!captured[1].allowed);
        assert_eq!(captured[1].tool, "save_note");
        // Live stream must mirror the post-hoc audit log.
        assert_eq!(captured.len(), proxy.audit_log().len());
        drop(captured);

        // Without a sink, behavior is unchanged (no panic, log still recorded).
        let proxy2 = ToolProxy::new(config, &tmp);
        proxy2
            .check_tool_call("search_notes", r#"{"query":"z"}"#)
            .unwrap();
        assert_eq!(proxy2.audit_log().len(), 1);
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
    #[test]
    fn write_pattern_allows_matching_path() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.permission = AgentPermission::ReadWrite;
        config.write_patterns = vec!["*.md".into(), "daily-notes/*".into()];
        let proxy = ToolProxy::new(config, &tmp);

        // Create the files so path confinement works
        std::fs::write(tmp.join("test.md"), "").unwrap();
        std::fs::create_dir_all(tmp.join("daily-notes")).unwrap();
        std::fs::write(tmp.join("daily-notes/2024-01-01.md"), "").unwrap();

        let r = proxy
            .check_tool_call("save_note", r#"{"path":"test.md"}"#)
            .unwrap();
        assert!(r.allowed, "*.md should match test.md: {:?}", r.reason);

        let r = proxy
            .check_tool_call("save_note", r#"{"path":"daily-notes/2024-01-01.md"}"#)
            .unwrap();
        assert!(r.allowed, "daily-notes/* should match: {:?}", r.reason);
    }

    #[test]
    fn write_pattern_blocks_non_matching_path() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.permission = AgentPermission::ReadWrite;
        config.write_patterns = vec!["*.md".into()];
        let proxy = ToolProxy::new(config, &tmp);

        std::fs::write(tmp.join("secret.txt"), "").unwrap();

        let r = proxy
            .check_tool_call("save_note", r#"{"path":"secret.txt"}"#)
            .unwrap();
        assert!(!r.allowed, "*.md should not match secret.txt");
        assert!(r.reason.contains("does not match write patterns"));
    }

    #[test]
    fn write_pattern_empty_blocks_all_writes() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.permission = AgentPermission::ReadWrite;
        config.write_patterns = vec![];
        let proxy = ToolProxy::new(config, &tmp);

        std::fs::write(tmp.join("test.md"), "").unwrap();

        let r = proxy
            .check_tool_call("save_note", r#"{"path":"test.md"}"#)
            .unwrap();
        assert!(!r.allowed, "empty write_patterns should block all writes");
    }

    #[test]
    fn glob_match_basic_patterns() {
        assert!(super::glob_match("*.md", "test.md"));
        assert!(super::glob_match("*.md", "hello.md"));
        assert!(!super::glob_match("*.md", "test.txt"));
        assert!(super::glob_match("daily-notes/*", "daily-notes/2024.md"));
        assert!(!super::glob_match("daily-notes/*", "other/2024.md"));
        assert!(super::glob_match("inbox/*.md", "inbox/quick.md"));
        assert!(!super::glob_match("inbox/*.md", "inbox/quick.txt"));
    }

    // Regression: #1326 — summarize_args must not panic on CJK UTF-8 boundary
    #[test]
    fn summarize_args_cjk_no_panic() {
        let cjk: String = "中".repeat(300); // 300 chars (> 200 limit), 900 bytes
        let result = ToolProxy::summarize_args(&cjk);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 201); // 200 chars + ellipsis
    }

    #[test]
    fn summarize_args_ascii_still_works() {
        let short = "{\"query\":\"hello\"}";
        assert_eq!(ToolProxy::summarize_args(short), short);
    }

    // Regression: #1326 — glob_match ** must match paths with /
    #[test]
    fn glob_match_double_star_matches_paths() {
        assert!(super::glob_match("**", "a/b"));
        assert!(super::glob_match("**", "a/b/c/d"));
        assert!(super::glob_match("**", ""));
        assert!(super::glob_match("**", "single"));
        assert!(super::glob_match("prefix/**", "prefix/a/b"));
        assert!(super::glob_match("**/suffix", "a/b/suffix"));
        assert!(super::glob_match("a/**/b", "a/x/y/b"));
    }

    // Regression: #1529 — ** must match zero path segments
    #[test]
    fn glob_match_double_star_matches_zero_segments() {
        assert!(super::glob_match("**/suffix", "suffix"));
        assert!(super::glob_match("a/**/b", "a/b"));
        assert!(super::glob_match("**/a/b", "a/b"));
    }

    // Regression: #1326 — glob_match ? must not match path separators
    #[test]
    fn glob_match_question_mark_rejects_slash() {
        assert!(super::glob_match("?", "a"));
        assert!(super::glob_match("?", "中"));
        assert!(!super::glob_match("?", "/"));
        assert!(!super::glob_match("?", "\\"));
        assert!(!super::glob_match("a/?/b", "a//b"));
    }
}

// ── Additional unit tests for pure functions ──────────────────────────────

#[cfg(test)]
mod pure_function_tests {
    use super::*;
    use std::fs;
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn slugify_basic() {
        // Delegated to crate::utils::slugify (#3167) — now lowercase + deunicode
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("test.md"), "test-md");
        assert_eq!(slugify("hello"), "hello");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("Hello! @#$% World"), "hello-world");
        assert_eq!(slugify("path/to/file"), "path-to-file");
    }

    #[test]
    fn slugify_consecutive_dashes_collapsed() {
        assert_eq!(slugify("a---b"), "a-b");
        assert_eq!(slugify("--test--"), "test");
    }

    #[test]
    fn slugify_preserves_alphanumeric_and_dashes() {
        assert_eq!(slugify("my-note_v2"), "my-note_v2");
        assert_eq!(slugify("2024-01-15"), "2024-01-15");
    }

    #[test]
    fn slugify_empty_string() {
        // Empty/special-char-only inputs now produce a hash-based fallback
        assert!(slugify("").starts_with("note-"));
        assert!(slugify("---").starts_with("note-"));
    }

    #[test]
    fn slugify_fallback_is_deterministic() {
        // Regression test for #2901: the fallback hash must be stable across
        // Rust builds (SHA-256, not DefaultHasher).
        let a = slugify("");
        let b = slugify("---");
        let c = slugify("!!!");
        assert!(a.starts_with("note-"), "should start with note-: {a}");
        assert!(b.starts_with("note-"), "should start with note-: {b}");
        assert!(c.starts_with("note-"), "should start with note-: {c}");
        // Same input always produces same output (deterministic hash)
        assert_eq!(
            slugify(""),
            slugify(""),
            "empty string must be deterministic"
        );
        assert_eq!(
            slugify("---"),
            slugify("---"),
            "special chars must be deterministic"
        );
        assert_eq!(
            slugify("!!!"),
            slugify("!!!"),
            "exclamation must be deterministic"
        );
        // Different inputs produce different fallbacks (collision check)
        assert_ne!(a, b, "different inputs should produce different hashes");
        assert_ne!(b, c, "different inputs should produce different hashes");
    }

    #[test]
    fn tool_display_name_all_variants() {
        assert_eq!(tool_display_name(&ai::AssistantToolCall::None), "none");
        assert_eq!(
            tool_display_name(&ai::AssistantToolCall::SearchNotes {
                query: "q".into(),
                limit: 5
            }),
            "search_notes"
        );
        assert_eq!(
            tool_display_name(&ai::AssistantToolCall::ListNotes { limit: 10 }),
            "list_notes"
        );
        assert_eq!(
            tool_display_name(&ai::AssistantToolCall::ListDirectory { path: "/".into() }),
            "list_directory"
        );
        assert_eq!(
            tool_display_name(&ai::AssistantToolCall::ReadFile {
                path: "test.md".into()
            }),
            "read_file"
        );
    }

    #[test]
    fn tool_args_summary_search_notes() {
        let tc = ai::AssistantToolCall::SearchNotes {
            query: "rust".into(),
            limit: 5,
        };
        assert_eq!(tool_args_summary(&tc), "query=rust limit=5");
    }

    #[test]
    fn tool_args_summary_list_notes() {
        let tc = ai::AssistantToolCall::ListNotes { limit: 20 };
        assert_eq!(tool_args_summary(&tc), "limit=20");
    }

    #[test]
    fn tool_args_summary_list_directory() {
        let tc = ai::AssistantToolCall::ListDirectory {
            path: "docs/".into(),
        };
        assert_eq!(tool_args_summary(&tc), "path=docs/");
    }

    #[test]
    fn tool_args_summary_read_file() {
        let tc = ai::AssistantToolCall::ReadFile {
            path: "notes/todo.md".into(),
        };
        assert_eq!(tool_args_summary(&tc), "path=notes/todo.md");
    }

    #[test]
    fn tool_args_summary_none() {
        assert_eq!(tool_args_summary(&ai::AssistantToolCall::None), "{}");
    }

    #[test]
    fn truncate_preview_short_string() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn truncate_preview_exact_boundary() {
        assert_eq!(truncate_preview("12345", 5), "12345");
    }

    #[test]
    fn truncate_preview_long_string() {
        let result = truncate_preview("hello world", 5);
        assert!(result.ends_with('…'));
        assert_eq!(result, "hello…");
    }

    #[test]
    fn truncate_preview_cjk_boundary() {
        // CJK chars are multi-byte but single char
        let cjk = "中".repeat(100);
        let result = truncate_preview(&cjk, 50);
        assert!(result.ends_with('…'));
        assert_eq!(result.chars().count(), 51); // 50 + ellipsis
    }

    #[test]
    fn truncate_preview_empty_string() {
        assert_eq!(truncate_preview("", 10), "");
    }

    #[test]
    fn agent_resource_limits_default() {
        let limits = AgentResourceLimits::default();
        assert_eq!(limits.max_duration, Duration::from_secs(300));
        assert_eq!(limits.max_tool_calls, 100);
        assert_eq!(limits.max_tokens, 0);
    }

    #[test]
    fn agent_config_default() {
        let config = AgentConfig::default();
        assert_eq!(config.name, "unnamed");
        assert_eq!(config.permission, AgentPermission::ReadOnly);
        assert!(config.allowed_tools.is_empty());
        assert!(config.write_patterns.is_empty());
    }

    #[test]
    fn agent_permission_default_is_read_only() {
        assert_eq!(AgentPermission::default(), AgentPermission::ReadOnly);
    }

    #[test]
    fn tool_proxy_result_fields() {
        let r = ToolProxyResult {
            allowed: true,
            reason: "ok".into(),
        };
        assert!(r.allowed);
        assert_eq!(r.reason, "ok");
    }

    #[test]
    fn tool_proxy_session_elapsed() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);
        // elapsed should be very small right after creation
        assert!(proxy.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn tool_proxy_tool_call_count_starts_at_zero() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);
        assert_eq!(proxy.tool_call_count(), 0);
    }

    #[test]
    fn tool_proxy_tool_call_count_increments() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);
        proxy
            .check_tool_call("search_notes", r#"{"query":"a"}"#)
            .unwrap();
        assert_eq!(proxy.tool_call_count(), 1);
        proxy
            .check_tool_call("search_notes", r#"{"query":"b"}"#)
            .unwrap();
        assert_eq!(proxy.tool_call_count(), 2);
    }

    #[test]
    fn tool_proxy_read_file_allows_in_vault_subdir() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        std::fs::create_dir_all(tmp.join("notes")).unwrap();
        std::fs::write(tmp.join("notes/todo.md"), "# Todo").unwrap();
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("read_file", r#"{"path":"notes/todo.md"}"#)
            .unwrap();
        assert!(r.allowed, "subdir path should be allowed: {:?}", r.reason);
    }

    #[test]
    fn tool_proxy_list_directory_allows_in_vault() {
        let (tmp, config) = setup();
        let _guard = TestGuard(tmp.clone());
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("list_directory", r#"{"path":"."}"#)
            .unwrap();
        assert!(
            r.allowed,
            "list vault root should be allowed: {:?}",
            r.reason
        );
    }

    #[test]
    fn tool_proxy_custom_whitelist_blocks_unlisted() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.allowed_tools = vec!["search_notes".into()];
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("list_notes", r#"{"limit":10}"#)
            .unwrap();
        assert!(!r.allowed, "unlisted tool should be blocked");
    }

    #[test]
    fn tool_proxy_write_pattern_with_subdir() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.permission = AgentPermission::ReadWrite;
        config.write_patterns = vec!["daily-notes/*".into()];
        std::fs::create_dir_all(tmp.join("daily-notes")).unwrap();
        std::fs::write(tmp.join("daily-notes/2024-06-23.md"), "").unwrap();
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("save_note", r#"{"path":"daily-notes/2024-06-23.md"}"#)
            .unwrap();
        assert!(r.allowed, "daily-notes/* should match: {:?}", r.reason);
    }

    #[test]
    fn tool_proxy_write_pattern_wildcard_all() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.permission = AgentPermission::ReadWrite;
        config.write_patterns = vec!["**".into()];
        std::fs::create_dir_all(tmp.join("any/path")).unwrap();
        std::fs::write(tmp.join("any/path/file.md"), "").unwrap();
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("save_note", r#"{"path":"any/path/file.md"}"#)
            .unwrap();
        assert!(r.allowed, "** should match any path: {:?}", r.reason);
    }

    #[test]
    fn normalize_path_components_removes_dots() {
        assert_eq!(
            ToolProxy::normalize_path_components("inbox/../inbox/test.md"),
            "inbox/test.md"
        );
        assert_eq!(ToolProxy::normalize_path_components("./foo/bar"), "foo/bar");
        assert_eq!(ToolProxy::normalize_path_components("a/b/../c"), "a/c");
        assert_eq!(ToolProxy::normalize_path_components("a/./b"), "a/b");
    }

    #[test]
    fn normalize_path_components_preserves_absolute_paths() {
        // Absolute paths must retain their leading slash (regression test for #2266)
        assert_eq!(ToolProxy::normalize_path_components("/foo/bar"), "/foo/bar");
        assert_eq!(ToolProxy::normalize_path_components("/foo/../bar"), "/bar");
        assert_eq!(
            ToolProxy::normalize_path_components("/foo/./bar"),
            "/foo/bar"
        );
        assert_eq!(ToolProxy::normalize_path_components("/"), "/");
        // Path traversal beyond root is safely clamped
        assert_eq!(
            ToolProxy::normalize_path_components("/../../../etc/passwd"),
            "/etc/passwd"
        );
    }

    #[test]
    fn write_pattern_allows_normalized_path() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.permission = AgentPermission::ReadWrite;
        config.write_patterns = vec!["inbox/*".into()];
        let proxy = ToolProxy::new(config, &tmp);
        let r = proxy
            .check_tool_call("save_note", r#"{"path":"inbox/../inbox/test.md"}"#)
            .unwrap();
        assert!(
            r.allowed,
            "inbox/* should match inbox/../inbox/test.md after normalization: {:?}",
            r.reason
        );
    }

    // ── extract_path_args ─────────────────────────────────────────

    #[test]
    fn extract_path_args_read_file() {
        let paths = ToolProxy::extract_path_args("read_file", r#"{"path":"/notes/test.md"}"#);
        assert_eq!(paths, vec!["/notes/test.md"]);
    }

    #[test]
    fn extract_path_args_save_note() {
        let paths =
            ToolProxy::extract_path_args("save_note", r#"{"path":"daily/2024.md","body":"x"}"#);
        assert_eq!(paths, vec!["daily/2024.md"]);
    }

    #[test]
    fn extract_path_args_rename_note() {
        let paths =
            ToolProxy::extract_path_args("rename_note", r#"{"path":"old.md","newPath":"new.md"}"#);
        assert_eq!(paths, vec!["old.md", "new.md"]);
    }

    #[test]
    fn extract_path_args_unknown_tool_returns_empty() {
        let paths = ToolProxy::extract_path_args("search_notes", r#"{"query":"test"}"#);
        assert!(paths.is_empty());
    }

    #[test]
    fn extract_path_args_missing_path_field() {
        let paths = ToolProxy::extract_path_args("read_file", r#"{"limit":5}"#);
        assert!(paths.is_empty());
    }

    #[test]
    fn extract_path_args_invalid_json() {
        let paths = ToolProxy::extract_path_args("read_file", "not json");
        assert!(paths.is_empty());
    }

    #[test]
    fn extract_path_args_empty_json() {
        let paths = ToolProxy::extract_path_args("read_file", "{}");
        assert!(paths.is_empty());
    }

    // Local copy of setup() — pure_function_tests is a sibling (not a child)
    // of `mod tests`, so it cannot reuse tests::setup(). Use a distinct temp
    // dir prefix to avoid colliding with mod tests' dir name when tests run in
    // parallel (both used `vaultpilot_agent_test_{pid}_{n}` and the
    // remove_dir_all in one module wiped the other module's live directory).
    fn setup() -> (PathBuf, AgentConfig) {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp =
            std::env::temp_dir().join(format!("vaultpilot_agent_pft_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("hello.md"), "# Hello\nWorld").unwrap();
        let config = AgentConfig::default();
        (tmp, config)
    }

    struct TestGuard(PathBuf);
    impl Drop for TestGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Resolve an absolute path to the `sleep` binary so the timeout/kill
    /// regression test does not depend on PATH lookup. Returns None if no
    /// `sleep` can be found anywhere on the system.
    fn resolve_sleep_binary() -> Option<String> {
        let which = std::process::Command::new("which").arg("sleep").output();
        if let Ok(out) = which {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let p = s.trim();
                if !p.is_empty() && Path::new(p).exists() {
                    return Some(p.to_string());
                }
            }
        }
        for candidate in ["/usr/bin/sleep", "/bin/sleep"] {
            if Path::new(candidate).exists() {
                return Some(candidate.to_string());
            }
        }
        None
    }

    // Regression: #1507 — run_command timeout must kill child process
    #[tokio::test]
    async fn run_command_timeout_kills_child() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.limits.max_duration = Duration::from_millis(200);
        let session = AgentSession::new(config, &tmp);

        // Use an absolute path to `sleep` to maximise the chance of a
        // successful spawn. If no `sleep` exists on this platform we cannot
        // exercise the timeout/kill path, so skip rather than fail.
        let sleep = match resolve_sleep_binary() {
            Some(p) => p,
            None => {
                eprintln!("skipping run_command_timeout_kills_child: no `sleep` binary found");
                return;
            }
        };

        let result = session
            .run_command(&sleep, &["60".into()], &tmp, |_| {}, |_| {})
            .await;

        // Some CI runners intermittently cannot spawn the helper process at
        // all (ENOENT even for an absolute path). That is an environment
        // limitation, not a regression — we cannot verify the timeout path
        // without a running child, so skip instead of reporting a spurious
        // failure. Only a real timeout error (or a success / non-timeout
        // error) counts as a meaningful result.
        match result {
            Err(e) if e.to_string().contains("timed out") => {}
            Err(e) if e.to_string().contains("failed to spawn") => {
                eprintln!("skipping run_command_timeout_kills_child: could not spawn helper ({e})");
                return;
            }
            Ok(code) => panic!("expected timeout error, but process exited with code {code}"),
            Err(e) => panic!("expected timeout error, got: {e}"),
        }
    }

    // ---- Plan Mode unit tests (#2107) ----

    #[test]
    fn plan_kind_display_tags_match_spec() {
        assert_eq!(PlanStepKind::Search.display_tag(), "[Search]");
        assert_eq!(PlanStepKind::Read.display_tag(), "[Read]");
        assert_eq!(PlanStepKind::Generate.display_tag(), "[Generate]");
        assert_eq!(PlanStepKind::Write.display_tag(), "[Write]");
        assert_eq!(PlanStepKind::Custom.display_tag(), "[Custom]");
    }

    #[test]
    fn plan_kind_from_str_lossy_maps_known_and_unknown() {
        assert_eq!(PlanStepKind::from_str_lossy("search"), PlanStepKind::Search);
        assert_eq!(
            PlanStepKind::from_str_lossy("LIST_NOTES"),
            PlanStepKind::Search
        );
        assert_eq!(
            PlanStepKind::from_str_lossy("read_file"),
            PlanStepKind::Read
        );
        assert_eq!(
            PlanStepKind::from_str_lossy("draft"),
            PlanStepKind::Generate
        );
        assert_eq!(
            PlanStepKind::from_str_lossy("save_note"),
            PlanStepKind::Write
        );
        assert_eq!(
            PlanStepKind::from_str_lossy("totally-unknown"),
            PlanStepKind::Custom
        );
    }

    #[test]
    fn format_tokens_uses_k_shorthand() {
        assert_eq!(format_tokens(0), "~0");
        assert_eq!(format_tokens(500), "~500");
        assert_eq!(format_tokens(1000), "~1k");
        assert_eq!(format_tokens(3000), "~3k");
        assert_eq!(format_tokens(3500), "~3.5k");
        // Large values — fractional detection at high ranges.
        assert_eq!(format_tokens(999999), "~1000.0k");
        // Exact ten-thousands.
        assert_eq!(format_tokens(10_000), "~10k");
    }

    #[test]
    fn parse_execution_plan_parses_fenced_json() {
        let model_text = "Here is the plan:\n```json\n{\"steps\":[{\"kind\":\"search\",\"description\":\"find notes\",\"tool\":\"search_notes\",\"estimated_tool_calls\":2},{\"kind\":\"write\",\"description\":\"save draft\",\"tool\":\"save_note\",\"estimated_tool_calls\":1}],\"estimated_tokens\":3000}\n```\n";
        let plan = parse_execution_plan(model_text, "draft a report");
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].kind, PlanStepKind::Search);
        assert_eq!(plan.steps[0].index, 1);
        assert_eq!(plan.steps[0].tool.as_deref(), Some("search_notes"));
        assert_eq!(plan.steps[1].kind, PlanStepKind::Write);
        assert_eq!(plan.steps[1].index, 2);
        assert_eq!(plan.estimated_tool_calls, 3);
        assert_eq!(plan.estimated_tokens, 3000);
    }

    #[test]
    fn parse_execution_plan_falls_back_on_garbage() {
        let plan = parse_execution_plan("I cannot do that.", "do something");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].kind, PlanStepKind::Custom);
        assert!(plan.steps[0].description.contains("do something"));
    }

    #[test]
    fn extract_first_json_object_handles_prose_and_fences() {
        // Surrounding prose.
        let j = extract_first_json_object("Here: {\"a\":1}");
        assert_eq!(j.as_deref(), Some("{\"a\":1}"));
        // Fenced.
        let j = extract_first_json_object("```json\n{\"a\":2}\n```");
        assert_eq!(j.as_deref(), Some("{\"a\":2}"));
        // Nested braces inside strings must not confuse the walker.
        let j = extract_first_json_object(r#"{"s":"{ not a close }"}"#);
        assert_eq!(j.as_deref(), Some(r#"{"s":"{ not a close }"}"#));
        // No object at all.
        assert!(extract_first_json_object("plain text").is_none());
    }

    #[test]
    fn render_markdown_matches_spec_format() {
        let mut steps = vec![
            PlanStep::new(PlanStepKind::Search, "find notes", Some("search_notes")),
            PlanStep::new(PlanStepKind::Read, "read context", Some("read_file")),
            PlanStep::new(PlanStepKind::Write, "save draft", Some("save_note")),
        ];
        // Disable the middle step to exercise the [ ] checkbox path.
        steps[1].enabled = false;
        for (i, s) in steps.iter_mut().enumerate() {
            s.index = i + 1;
        }
        let plan = ExecutionPlan {
            task: "t".into(),
            steps,
            estimated_tool_calls: 3,
            estimated_tokens: 3000,
            generated_at: "now".into(),
        };
        let md = plan.render_markdown();
        assert!(md.starts_with("## Execution Plan\n"));
        assert!(md.contains("1. [x] [Search] find notes"));
        assert!(md.contains("2. [ ] [Read] read context"));
        assert!(md.contains("3. [x] [Write] save draft"));
        assert!(md.contains("Estimated tool calls: 3  Estimated tokens: ~3k"));
    }

    #[test]
    fn apply_decision_reject_skips_all() {
        let plan = sample_plan();
        let finalized = plan.apply_decision(&PlanDecision::Reject);
        assert!(finalized
            .steps
            .iter()
            .all(|s| s.status == PlanStepStatus::Skipped));
        assert!(!finalized.has_executable_steps());
    }

    #[test]
    fn apply_decision_partial_approve_skips_disabled() {
        let mut plan = sample_plan();
        plan.steps[1].enabled = false; // disable the read step
        let finalized = plan.apply_decision(&PlanDecision::PartialApprove);
        assert_eq!(finalized.steps[1].status, PlanStepStatus::Skipped);
        assert_eq!(finalized.steps[1].index, 0);
        assert_eq!(finalized.steps[0].status, PlanStepStatus::Pending);
        assert_eq!(finalized.steps[0].index, 1);
        // Disabled step skipped; the write step re-indexes to 2.
        assert_eq!(finalized.steps[2].status, PlanStepStatus::Pending);
        assert_eq!(finalized.steps[2].index, 2);
        assert!(finalized.has_executable_steps());
    }

    #[test]
    fn apply_decision_edit_adopts_revised_steps() {
        let plan = sample_plan();
        let revised = vec![PlanStep::new(PlanStepKind::Generate, "new step", None)];
        let finalized = plan.apply_decision(&PlanDecision::Edit { steps: revised });
        assert_eq!(finalized.steps.len(), 1);
        assert_eq!(finalized.steps[0].kind, PlanStepKind::Generate);
        assert_eq!(finalized.steps[0].index, 1);
    }

    /// Helper: a 3-step plan (Search → Read → Write), all enabled.
    fn sample_plan() -> ExecutionPlan {
        let steps = vec![
            PlanStep::new(PlanStepKind::Search, "s", Some("search_notes")),
            PlanStep::new(PlanStepKind::Read, "r", Some("read_file")),
            PlanStep::new(PlanStepKind::Write, "w", Some("save_note")),
        ];
        ExecutionPlan {
            task: "t".into(),
            steps,
            estimated_tool_calls: 3,
            estimated_tokens: 1000,
            generated_at: "now".into(),
        }
    }

    // ── Regression: estimate_tokens (#2542) ─────────────────────────

    #[test]
    fn regression_2542_estimate_tokens_uses_provided_count() {
        assert_eq!(estimate_tokens(Some(100), 400), 100);
    }

    #[test]
    fn regression_2542_estimate_tokens_fallback_count_none() {
        // 400 text_len / 4 = 100
        assert_eq!(estimate_tokens(None, 400), 100);
    }

    #[test]
    fn regression_2542_estimate_tokens_fallback_min_one() {
        // text_len=0, max(1, 0/4)=1
        assert_eq!(estimate_tokens(None, 0), 1);
    }

    #[test]
    fn regression_2542_estimate_tokens_fallback_cjk() {
        // CJK 4 bytes per char; 12 bytes / 4 = 3 tokens
        assert_eq!(estimate_tokens(None, 12), 3);
    }

    // ── #3118: SessionHealthTracker silent_failure reachability ─────────────
    // These tests exercise the real (private) struct, not a recreation.

    #[test]
    fn regression_3118_silent_failure_fires_on_successful_but_empty_results() {
        // The exact case the original #3103 silent_failure branch was supposed
        // to catch but never could: 6+ non-error tool calls, each returning
        // trivial/empty output ("ok", "", "1") → useful_ops stays at 0 →
        // silent_failure fires at step 6.
        let mut t = SessionHealthTracker::new();
        let mut fired: Option<String> = None;
        let trivial_calls = [
            ("read_file", "/x.md", "ok"),
            ("read_file", "/x.md", ""),
            ("search_notes", "x", "[]"),
            ("read_file", "/x.md", "ok"),
            ("list_notes", "all", ""),
            ("search_notes", "x", "1"),
        ];
        for (i, (tool, args, result)) in trivial_calls.iter().enumerate() {
            if let Some((reason, _suggestion)) = t.record_and_check(tool, args, false, result) {
                fired = Some(reason);
                assert_eq!(
                    i, 5,
                    "silent_failure should fire exactly at step 6, not earlier"
                );
                break;
            }
        }
        let reason =
            fired.expect("silent_failure must fire after 6 successful-but-empty steps (#3118)");
        assert!(
            reason.contains("zero useful operations"),
            "Reason should mention 'zero useful operations', got: {reason}"
        );
    }

    #[test]
    fn regression_3118_silent_failure_does_not_fire_with_useful_output() {
        // 6+ steps where every result is genuinely useful (length >= 5) must
        // NOT trigger silent_failure — useful_ops increments each step.
        let mut t = SessionHealthTracker::new();
        let useful_calls = [
            ("read_file", "/a.md", "# Title\nbody"),
            ("read_file", "/b.md", "another file"),
            ("search_notes", "x", "3 matches"),
            ("read_file", "/c.md", "more content"),
            ("list_notes", "all", "10 notes"),
            ("search_notes", "x", "no exact match"),
            ("read_file", "/d.md", "even more text"),
        ];
        for (tool, args, result) in useful_calls.iter() {
            let fired = t.record_and_check(tool, args, false, result);
            assert!(
                fired.is_none(),
                "Useful non-error output must not trigger any unhealthy signal, got: {fired:?}"
            );
        }
    }

    #[test]
    fn regression_3118_silent_failure_never_reached_when_all_errors() {
        // The pre-#3118 bug: all-error input would in theory satisfy
        // successful_ops==0 but error_spiral (branch 2) fires first at step 3.
        // Verify that the production code emits error_spiral, not silent_failure.
        let mut t = SessionHealthTracker::new();
        let mut fired_reason: Option<String> = None;
        let error_calls = [
            ("read_file", "/missing.md", "tool error: not found"),
            ("read_file", "/missing.md", "tool error: not found"),
            ("read_file", "/missing.md", "tool error: not found"),
            ("search_notes", "x", "tool error: invalid"),
            ("read_file", "/missing.md", "tool error: not found"),
            ("list_notes", "all", "tool error: failed"),
        ];
        for (tool, args, result) in error_calls.iter() {
            if let Some((reason, _suggestion)) = t.record_and_check(tool, args, true, result) {
                fired_reason = Some(reason);
                break;
            }
        }
        let reason = fired_reason.expect("must fire on all-error input");
        assert!(
            reason.contains("consecutive tool errors"),
            "All-error input must trigger error_spiral, not silent_failure. Got: {reason}"
        );
    }

    #[test]
    fn regression_3118_useful_threshold_boundary() {
        // USEFUL_RESULT_MIN_CHARS is 5. Pin the boundary:
        //   - 4-char non-error result  → NOT useful
        //   - 5-char non-error result  → useful
        //
        // Distinct args (different file paths) so the repetition detector
        // (branch 1) never fires.
        let mut t = SessionHealthTracker::new();
        // 6 calls where only ONE has length >= 5 → useful_ops = 1 → silent_failure NOT fired.
        let calls = [
            ("read_file", "/a.md", "abcd"),  // 4 chars → not useful
            ("read_file", "/b.md", "abcd"),  // 4 chars → not useful
            ("read_file", "/c.md", "abcd"),  // 4 chars → not useful
            ("read_file", "/d.md", "abcde"), // 5 chars → useful
            ("read_file", "/e.md", "abcd"),  // 4 chars → not useful
            ("read_file", "/f.md", "abcd"),  // 4 chars → not useful
        ];
        for (tool, args, result) in calls.iter() {
            let fired = t.record_and_check(tool, args, false, result);
            assert!(
                fired.is_none(),
                "Boundary test: one 5-char result makes useful_ops=1 → silent_failure must NOT fire. Got: {fired:?}"
            );
        }
        // Now verify the all-4-char case DOES fire silent_failure at step 6.
        let mut t2 = SessionHealthTracker::new();
        let mut fired = false;
        let calls_all_short = [
            ("read_file", "/a.md", "abcd"),
            ("read_file", "/b.md", "abcd"),
            ("read_file", "/c.md", "abcd"),
            ("read_file", "/d.md", "abcd"),
            ("read_file", "/e.md", "abcd"),
            ("read_file", "/f.md", "abcd"),
        ];
        for (tool, args, result) in calls_all_short.iter() {
            if t2.record_and_check(tool, args, false, result).is_some() {
                fired = true;
            }
        }
        assert!(
            fired,
            "6 non-error steps with all 4-char results must trigger silent_failure (#3118)"
        );
    }
}
