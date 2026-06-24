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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
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
    /// Glob patterns for writable paths (e.g. "*.md", "daily-notes/*", "inbox/*").
    /// Only enforced when `permission` is `ReadWrite`. Empty = no write access.
    pub write_patterns: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "unnamed".into(),
            permission: AgentPermission::ReadOnly,
            limits: AgentResourceLimits::default(),
            allowed_tools: Vec::new(),
            write_patterns: Vec::new(),
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
        if Self::is_write_tool(tool) {
            if self.config.permission == AgentPermission::ReadOnly {
                let entry = self.deny(tool, args_json, "write denied: agent is read-only");
                return Ok(entry);
            }
            // Check write pattern whitelist
            if let Some(path_value) = Self::extract_path_arg(tool, args_json) {
                if !self.is_path_writable(&path_value) {
                    let entry = self.deny(
                        tool,
                        args_json,
                        &format!(
                            "write denied: path '{}' does not match write patterns",
                            sanitize_error(&path_value)
                        ),
                    );
                    return Ok(entry);
                }
            }
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
        self.audit_log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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

    /// Check if a path matches the write pattern whitelist.
    /// Patterns are glob-style: "*.md", "daily-notes/*", "inbox/*".
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
        self.config
            .write_patterns
            .iter()
            .any(|pattern| glob_match(pattern, &relative))
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
            .unwrap_or_else(|e| e.into_inner())
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
            .unwrap_or_else(|e| e.into_inner())
            .push(entry);
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

        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                on_stdout(&line);
            }
        });

        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                on_stderr(&line);
            }
        });

        // Apply timeout from resource limits — kill child on timeout to avoid zombie
        let status = tokio::select! {
            result = child.wait() => {
                result.map_err(|e| anyhow!("failed to wait for agent process: {}", e))?
            }
            _ = tokio::time::sleep(self.config.limits.max_duration) => {
                let _ = child.kill().await;
                anyhow::bail!("agent process timed out after {:?}", self.config.limits.max_duration);
            }
        };

        // Wait for output tasks to finish
        let _ = tokio::join!(stdout_task, stderr_task);

        Ok(status.code().unwrap_or(-1))
    }
}

// ── Built-in agent loop (Phase 3.2) ──────────────────────────────────────

use crate::ai;
use crate::storage::StorageContext;

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
}

/// Result of an agent execution session.
#[derive(Debug, Clone, Serialize)]
pub struct AgentResult {
    pub answer: String,
    pub steps_used: usize,
    pub tokens_used: u64,
    pub audit_log: Vec<AgentAuditEntry>,
}

/// Maximum tool-calling rounds in the agent loop.
const DEFAULT_MAX_STEPS: usize = 20;

/// Run an autonomous agent loop: prompt → LLM → tool call → execute → repeat.
///
/// The agent uses `select_tool_call` to decide which tool to invoke, executes
/// it through the sandboxed `ToolProxy`, and feeds results back to the LLM
/// until it produces a final answer or hits a resource limit.
///
/// `on_event` is called for every significant progress change.
/// For write operations, `on_event` receives `WriteApprovalNeeded` — the
/// callback should return `true` to approve or `false` to deny.
pub async fn run_agent(
    settings: &crate::models::AppSettings,
    context: &StorageContext,
    prompt: &str,
    config: AgentConfig,
    mut on_event: impl FnMut(&AgentEvent) -> bool,
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

    for step in 0..max_steps {
        // Timeout check
        if proxy.elapsed() > config.limits.max_duration {
            on_event(&AgentEvent::Timeout);
            break;
        }

        on_event(&AgentEvent::Thinking { step: step + 1 });

        // Ask LLM what tool to call
        let selection = ai::select_tool_call(settings, prompt, &[], &[], &tool_transcripts)
            .await
            .map_err(|e| {
                anyhow!(
                    "LLM call failed at step {}: {}",
                    step + 1,
                    sanitize_error(&e.to_string())
                )
            })?;

        total_tokens += selection.usage.input_tokens.unwrap_or(0) as u64
            + selection.usage.output_tokens.unwrap_or(0) as u64;

        // Token budget check
        if token_budget > 0 && total_tokens > token_budget {
            on_event(&AgentEvent::TokenBudgetExceeded {
                tokens_used: total_tokens,
                budget: token_budget,
            });
            break;
        }

        match selection.tool_call {
            ai::AssistantToolCall::None => {
                // LLM decided no more tools needed — generate final answer
                let answer = if tool_transcripts.is_empty() {
                    crate::ai::answer_question(settings, prompt, &[], &[], &[])
                        .await
                        .map_err(|e| {
                            anyhow!("final answer failed: {}", sanitize_error(&e.to_string()))
                        })?
                } else {
                    crate::ai::answer_after_tools(settings, prompt, &tool_transcripts, &[], &[])
                        .await
                        .map_err(|e| {
                            anyhow!("final answer failed: {}", sanitize_error(&e.to_string()))
                        })?
                };
                total_tokens += answer.usage.input_tokens.unwrap_or(0) as u64
                    + answer.usage.output_tokens.unwrap_or(0) as u64;
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

                // ToolProxy sandbox check
                let check = proxy.check_tool_call(tool_name, &args_summary)?;
                if !check.allowed {
                    tool_transcripts.push(format!(
                        "TOOL: {}\nSTATUS: denied\nINPUT:\n{}\nOUTPUT:\ntool error: {}",
                        tool_name, args_summary, check.reason
                    ));
                    continue;
                }

                // Write approval callback
                if ToolProxy::is_write_tool(tool_name) {
                    let approved = on_event(&AgentEvent::WriteApprovalNeeded {
                        tool: tool_name.to_string(),
                        args: args_summary.clone(),
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

                // Execute the tool
                let (result, is_error) = execute_tool(context, settings, &tool_call).await;
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

    // Exited loop without a final answer — generate one from accumulated results
    on_event(&AgentEvent::StepLimitReached { steps: max_steps });
    let answer = if tool_transcripts.is_empty() {
        crate::ai::answer_question(settings, prompt, &[], &[], &[])
            .await
            .map_err(|e| anyhow!("final answer failed: {}", sanitize_error(&e.to_string())))?
    } else {
        crate::ai::answer_after_tools(settings, prompt, &tool_transcripts, &[], &[])
            .await
            .map_err(|e| anyhow!("final answer failed: {}", sanitize_error(&e.to_string())))?
    };
    total_tokens += answer.usage.input_tokens.unwrap_or(0) as u64
        + answer.usage.output_tokens.unwrap_or(0) as u64;
    Ok(AgentResult {
        answer: answer.answer,
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
                        (format!("Found {} notes:\n{}", docs.len(), summary), false)
                    }
                }
                Err(e) => (format!("tool error: {}", e), true),
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
                Err(e) => (format!("tool error: {}", e), true),
            }
        }
        ai::AssistantToolCall::ListDirectory { path } => {
            let vault_root = Path::new(&settings.vault_dir);
            match list_directory_for_agent(path, vault_root) {
                Ok(output) => (output, false),
                Err(e) => (format!("tool error: {}", e), true),
            }
        }
        ai::AssistantToolCall::ReadFile { path } => {
            let vault_root = Path::new(&settings.vault_dir);
            match read_file_for_agent(path, vault_root) {
                Ok(output) => (output, false),
                Err(e) => (format!("tool error: {}", e), true),
            }
        }
        ai::AssistantToolCall::SaveNote { draft } => {
            use crate::storage::save_note_with_images_async;
            let note = crate::models::NoteDocument {
                meta: crate::models::NoteMeta {
                    id: uuid::Uuid::new_v4().to_string(),
                    title: draft.title.clone(),
                    path: format!("{}.md", slugify(&draft.title)),
                    tags: draft.tags.clone(),
                    keywords: draft.keywords.clone(),
                    created_at: chrono::Utc::now().to_rfc3339(),
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
                Err(e) => (format!("tool error: save_note failed: {}", e), true),
            }
        }
    }
}

fn list_directory_for_agent(path: &str, vault_root: &Path) -> Result<String> {
    let directory = crate::normalize_tool_path(path, vault_root)?;
    if !directory.exists() {
        return Err(anyhow!("path does not exist: {}", path));
    }
    if !directory.is_dir() {
        return Err(anyhow!("path is not a directory: {}", path));
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(format!("{}{}", name, if is_dir { "/" } else { "" }));
    }
    entries.sort();
    Ok(entries.join("\n"))
}

fn read_file_for_agent(path: &str, vault_root: &Path) -> Result<String> {
    let file_path = crate::normalize_tool_path(path, vault_root)?;
    if !file_path.exists() {
        return Err(anyhow!("file does not exist: {}", path));
    }
    let content = std::fs::read_to_string(&file_path)?;
    // Cap at 50KB to prevent token explosion
    const MAX_READ: usize = 50 * 1024;
    if content.len() > MAX_READ {
        let truncated: String = content.chars().take(MAX_READ).collect();
        Ok(format!(
            "{}\n[... truncated at {} chars]",
            truncated, MAX_READ
        ))
    } else {
        Ok(content)
    }
}

fn slugify(title: &str) -> String {
    let mut slug: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // Collapse consecutive dashes
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let cleaned = slug.trim_matches('-').to_string();
    if cleaned.is_empty() {
        let mut hasher = DefaultHasher::new();
        title.hash(&mut hasher);
        format!("note-{:08x}", hasher.finish())
    } else {
        cleaned
    }
}

fn tool_display_name(tool: &ai::AssistantToolCall) -> &'static str {
    match tool {
        ai::AssistantToolCall::None => "none",
        ai::AssistantToolCall::SearchNotes { .. } => "search_notes",
        ai::AssistantToolCall::ListNotes { .. } => "list_notes",
        ai::AssistantToolCall::ListDirectory { .. } => "list_directory",
        ai::AssistantToolCall::ReadFile { .. } => "read_file",
        ai::AssistantToolCall::SaveNote { .. } => "save_note",
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
        ai::AssistantToolCall::SaveNote { draft } => {
            format!("title={}", draft.title)
        }
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
                star_pi = pi;
                star_ti = ti;
                pi += 2;
            } else {
                star_pi = pi;
                star_ti = ti;
                pi += 1;
            }
        } else if star_pi != usize::MAX {
            // Backtrack to the last star
            if star_pi + 1 < pattern.len() && pattern[star_pi + 1] == '*' {
                // ** matches everything
                star_ti += 1;
                ti = star_ti;
                pi = star_pi + 2;
            } else if text[ti] != '/' && text[ti] != '\\' {
                // * doesn't match path separators
                star_ti += 1;
                ti = star_ti;
                pi = star_pi + 1;
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
        assert_eq!(slugify("Hello World"), "Hello-World");
        assert_eq!(slugify("test.md"), "test-md");
        assert_eq!(slugify("hello"), "hello");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("Hello! @#$% World"), "Hello-World");
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

    // ── extract_path_arg ─────────────────────────────────────────

    #[test]
    fn extract_path_arg_read_file() {
        let path = ToolProxy::extract_path_arg("read_file", r#"{"path":"/notes/test.md"}"#);
        assert_eq!(path, Some("/notes/test.md".into()));
    }

    #[test]
    fn extract_path_arg_save_note() {
        let path =
            ToolProxy::extract_path_arg("save_note", r#"{"path":"daily/2024.md","body":"x"}"#);
        assert_eq!(path, Some("daily/2024.md".into()));
    }

    #[test]
    fn extract_path_arg_unknown_tool_returns_none() {
        let path = ToolProxy::extract_path_arg("search_notes", r#"{"query":"test"}"#);
        assert_eq!(path, None);
    }

    #[test]
    fn extract_path_arg_missing_path_field() {
        let path = ToolProxy::extract_path_arg("read_file", r#"{"limit":5}"#);
        assert_eq!(path, None);
    }

    #[test]
    fn extract_path_arg_invalid_json() {
        let path = ToolProxy::extract_path_arg("read_file", "not json");
        assert_eq!(path, None);
    }

    #[test]
    fn extract_path_arg_empty_json() {
        let path = ToolProxy::extract_path_arg("read_file", "{}");
        assert_eq!(path, None);
    }

    // Use setup from the parent module
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

    struct TestGuard(PathBuf);
    impl Drop for TestGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // Regression: #1507 — run_command timeout must kill child process
    #[tokio::test]
    async fn run_command_timeout_kills_child() {
        let (tmp, mut config) = setup();
        let _guard = TestGuard(tmp.clone());
        config.limits.max_duration = Duration::from_millis(200);
        let session = AgentSession::new(config, &tmp);

        let result = session
            .run_command("sleep", &["60".into()], &tmp, |_| {}, |_| {})
            .await;

        assert!(result.is_err(), "should return timeout error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("timed out"),
            "error should mention timeout: {}",
            err
        );
    }
}
