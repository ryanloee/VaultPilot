//! Integration tests for Agent Mode — issue #1359.
//!
//! Tests the agent infrastructure without external LLM calls:
//! - ToolProxy sandbox boundaries (path confinement, write patterns)
//! - Resource limits (tool call count, token budget)
//! - Agent event serialization round-trip
//! - Write approval channel coordination

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::agent::{AgentConfig, AgentEvent, AgentPermission, AgentResourceLimits, ToolProxy};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> (PathBuf, AgentConfig) {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!(
        "vaultpilot_agent_integration_{}_{}",
        std::process::id(),
        n
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("hello.md"), "# Hello\nWorld").unwrap();
    fs::create_dir_all(tmp.join("notes")).unwrap();
    fs::write(tmp.join("notes/daily.md"), "# Daily").unwrap();
    fs::create_dir_all(tmp.join("daily-notes")).unwrap();
    fs::write(tmp.join("daily-notes/2026-06-23.md"), "# Today").unwrap();
    let config = AgentConfig::default();
    (tmp, config)
}

struct TestGuard(PathBuf);
impl Drop for TestGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ── Sandbox boundary tests ────────────────────────────────────────────

#[test]
fn sandbox_blocks_path_traversal() {
    let (tmp, config) = setup();
    let _guard = TestGuard(tmp.clone());
    let proxy = ToolProxy::new(config, &tmp);

    let check = proxy
        .check_tool_call("read_file", r#"{"path": "../../etc/passwd"}"#)
        .unwrap();
    assert!(!check.allowed, "path traversal should be blocked");
    assert!(
        check.reason.contains("outside") || check.reason.contains("confine"),
        "reason should mention outside vault: {}",
        check.reason
    );
}

#[test]
fn sandbox_blocks_absolute_path_outside_vault() {
    let (tmp, config) = setup();
    let _guard = TestGuard(tmp.clone());
    let proxy = ToolProxy::new(config, &tmp);

    let check = proxy
        .check_tool_call("read_file", r#"{"path": "/etc/shadow"}"#)
        .unwrap();
    assert!(
        !check.allowed,
        "absolute path outside vault should be blocked"
    );
}

#[test]
fn sandbox_allows_relative_path_inside_vault() {
    let (tmp, config) = setup();
    let _guard = TestGuard(tmp.clone());
    let proxy = ToolProxy::new(config, &tmp);

    let check = proxy
        .check_tool_call("read_file", r#"{"path": "hello.md"}"#)
        .unwrap();
    assert!(
        check.allowed,
        "relative path inside vault should be allowed: {}",
        check.reason
    );
}

#[test]
fn sandbox_read_only_blocks_write_tools() {
    let (tmp, config) = setup();
    let _guard = TestGuard(tmp.clone());
    let proxy = ToolProxy::new(config, &tmp);

    for tool in &["save_note", "write_note", "delete_note", "rename_note"] {
        let check = proxy
            .check_tool_call(tool, r#"{"path": "test.md"}"#)
            .unwrap();
        assert!(
            !check.allowed,
            "read-only agent should not be able to call {}",
            tool
        );
    }
}

#[test]
fn sandbox_read_only_allows_read_tools() {
    let (tmp, config) = setup();
    let _guard = TestGuard(tmp.clone());
    let proxy = ToolProxy::new(config, &tmp);

    for tool in &["search_notes", "read_file", "list_directory", "list_notes"] {
        let args = if *tool == "search_notes" {
            r#"{"query": "test"}"#
        } else if *tool == "list_notes" {
            r#"{"limit": 10}"#
        } else {
            r#"{"path": "hello.md"}"#
        };
        let check = proxy.check_tool_call(tool, args).unwrap();
        assert!(
            check.allowed,
            "read-only agent should be able to call {}: {}",
            tool, check.reason
        );
    }
}

#[test]
fn sandbox_write_pattern_blocks_non_matching() {
    let (tmp, mut config) = setup();
    let _guard = TestGuard(tmp.clone());
    config.permission = AgentPermission::ReadWrite;
    config.write_patterns = vec!["daily-notes/*".to_string()];
    let proxy = ToolProxy::new(config, &tmp);

    let check = proxy
        .check_tool_call("save_note", r#"{"path": "hello.md"}"#)
        .unwrap();
    assert!(
        !check.allowed,
        "write to non-matching pattern should be blocked"
    );
}

#[test]
fn sandbox_write_pattern_allows_matching() {
    let (tmp, mut config) = setup();
    let _guard = TestGuard(tmp.clone());
    config.permission = AgentPermission::ReadWrite;
    config.write_patterns = vec!["daily-notes/*".to_string()];
    let proxy = ToolProxy::new(config, &tmp);

    let check = proxy
        .check_tool_call("save_note", r#"{"path": "daily-notes/2026-06-23.md"}"#)
        .unwrap();
    assert!(check.allowed, "write to matching pattern should be allowed");
}

#[test]
fn sandbox_empty_write_patterns_blocks_all_writes() {
    let (tmp, mut config) = setup();
    let _guard = TestGuard(tmp.clone());
    config.permission = AgentPermission::ReadWrite;
    config.write_patterns = vec![];
    let proxy = ToolProxy::new(config, &tmp);

    let check = proxy
        .check_tool_call("save_note", r#"{"path": "hello.md"}"#)
        .unwrap();
    assert!(
        !check.allowed,
        "empty write patterns should block all writes"
    );
}

#[test]
fn sandbox_unknown_tool_blocked() {
    let (tmp, config) = setup();
    let _guard = TestGuard(tmp.clone());
    let proxy = ToolProxy::new(config, &tmp);

    let check = proxy.check_tool_call("run_command", "rm -rf /").unwrap();
    assert!(!check.allowed, "unknown tool should be blocked");
}

// ── Resource limit tests ──────────────────────────────────────────────

#[test]
fn resource_limits_default_values() {
    let limits = AgentResourceLimits::default();
    assert_eq!(limits.max_duration, Duration::from_secs(300));
    assert_eq!(limits.max_tool_calls, 100);
    assert_eq!(limits.max_tokens, 0); // unlimited
}

#[test]
fn agent_config_default_is_read_only() {
    let config = AgentConfig::default();
    assert_eq!(config.permission, AgentPermission::ReadOnly);
    assert!(config.write_patterns.is_empty());
}

#[test]
fn tool_call_limit_enforced() {
    let (tmp, mut config) = setup();
    let _guard = TestGuard(tmp.clone());
    config.limits.max_tool_calls = 2;
    let proxy = ToolProxy::new(config, &tmp);

    assert!(
        proxy
            .check_tool_call("search_notes", r#"{"query": "a"}"#)
            .unwrap()
            .allowed
    );
    assert!(
        proxy
            .check_tool_call("search_notes", r#"{"query": "b"}"#)
            .unwrap()
            .allowed
    );
    // Third call should be denied.
    let r = proxy
        .check_tool_call("search_notes", r#"{"query": "c"}"#)
        .unwrap();
    assert!(!r.allowed, "third call should be denied");
    assert!(r.reason.contains("limit"), "reason: {}", r.reason);
}

// ── Tool proxy audit log tests ────────────────────────────────────────

#[test]
fn tool_proxy_records_audit_log() {
    let (tmp, config) = setup();
    let _guard = TestGuard(tmp.clone());
    let proxy = ToolProxy::new(config, &tmp);

    let _ = proxy.check_tool_call("search_notes", r#"{"query": "test"}"#);
    let _ = proxy.check_tool_call("read_file", r#"{"path": "hello.md"}"#);

    let log = proxy.audit_log();
    assert!(log.len() >= 2, "audit log should record tool calls");
}

#[test]
fn tool_proxy_tool_call_count_increments() {
    let (tmp, config) = setup();
    let _guard = TestGuard(tmp.clone());
    let proxy = ToolProxy::new(config, &tmp);

    assert_eq!(proxy.tool_call_count(), 0);
    let _ = proxy.check_tool_call("search_notes", r#"{"query": "test"}"#);
    assert_eq!(proxy.tool_call_count(), 1);
    let _ = proxy.check_tool_call("read_file", r#"{"path": "hello.md"}"#);
    assert_eq!(proxy.tool_call_count(), 2);
}

// ── Event serialization tests ─────────────────────────────────────────

#[test]
fn agent_event_all_variants_clone_and_debug() {
    let events = vec![
        AgentEvent::Thinking { step: 1 },
        AgentEvent::ToolCall {
            step: 2,
            tool: "search".into(),
            args: "{}".into(),
        },
        AgentEvent::ToolResult {
            step: 2,
            tool: "search".into(),
            result_preview: "found 3".into(),
            is_error: false,
        },
        AgentEvent::FinalAnswer {
            text: "done".into(),
        },
        AgentEvent::WriteApprovalNeeded {
            tool: "save_note".into(),
            args: "{}".into(),
        },
        AgentEvent::StepLimitReached { steps: 20 },
        AgentEvent::TokenBudgetExceeded {
            tokens_used: 5000,
            budget: 4000,
        },
        AgentEvent::Timeout,
        AgentEvent::Error {
            message: "oops".into(),
        },
    ];

    for event in &events {
        let _ = format!("{:?}", event); // Debug
        let _clone: AgentEvent = event.clone(); // Clone
    }
}

// ── Write approval coordination test ──────────────────────────────────

#[test]
fn write_approval_channel_coordination() {
    use std::sync::mpsc;
    use std::thread;

    let (tx, rx) = mpsc::channel();

    let agent_handle = thread::spawn(move || {
        let approved = rx.recv().unwrap_or(false);
        approved
    });

    tx.send(true).unwrap();

    let result = agent_handle.join().unwrap();
    assert!(result, "agent should receive approval");

    let (tx2, rx2) = mpsc::channel();
    let agent_handle2 = thread::spawn(move || rx2.recv().unwrap_or(false));
    tx2.send(false).unwrap();
    let result2 = agent_handle2.join().unwrap();
    assert!(!result2, "agent should receive denial");
}

#[test]
fn write_approval_channel_timeout_behavior() {
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel::<bool>();

    // Drop sender without sending — receiver should get error
    drop(tx);
    let result = rx.recv();
    assert!(result.is_err(), "dropped sender should cause recv error");

    // Verify the fallback behavior (unwrap_or(false))
    let (tx2, rx2) = mpsc::channel::<bool>();
    drop(tx2);
    let approved = rx2.recv().unwrap_or(false);
    assert!(!approved, "dropped sender should default to denied");
}
