//! Regression tests for #1342 — Agent Mode Phase 3.2 built-in agent loop.

use crate::agent::{
    AgentAuditEntry, AgentConfig, AgentEvent, AgentPermission, AgentResourceLimits, AgentResult,
};

// ── AgentEvent tests ──────────────────────────────────────────────────

#[test]
fn agent_event_is_clone_and_debug() {
    let event = AgentEvent::Thinking { step: 1 };
    let _cloned = event.clone();
    let _debug = format!("{:?}", event);
}

#[test]
fn agent_event_tool_call_fields() {
    let event = AgentEvent::ToolCall {
        step: 3,
        tool: "search_notes".into(),
        args: "query=test".into(),
    };
    match event {
        AgentEvent::ToolCall { step, tool, args } => {
            assert_eq!(step, 3);
            assert_eq!(tool, "search_notes");
            assert_eq!(args, "query=test");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn agent_event_step_limit_reached() {
    let event = AgentEvent::StepLimitReached { steps: 20 };
    match event {
        AgentEvent::StepLimitReached { steps } => assert_eq!(steps, 20),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn agent_event_token_budget_exceeded() {
    let event = AgentEvent::TokenBudgetExceeded {
        tokens_used: 5000,
        budget: 4000,
    };
    match event {
        AgentEvent::TokenBudgetExceeded {
            tokens_used,
            budget,
        } => {
            assert_eq!(tokens_used, 5000);
            assert_eq!(budget, 4000);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn agent_event_write_approval_needed() {
    let event = AgentEvent::WriteApprovalNeeded {
        tool: "save_note".into(),
        args: "title=Test".into(),
    };
    match event {
        AgentEvent::WriteApprovalNeeded { tool, args } => {
            assert_eq!(tool, "save_note");
            assert_eq!(args, "title=Test");
        }
        _ => panic!("wrong variant"),
    }
}

// ── AgentResult tests ─────────────────────────────────────────────────

#[test]
fn agent_result_serializes() {
    let result = AgentResult {
        answer: "test answer".into(),
        steps_used: 3,
        tokens_used: 1234,
        audit_log: vec![],
    };
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("test answer"));
    assert!(json.contains("1234"));
}

#[test]
fn agent_result_with_audit_entries() {
    let result = AgentResult {
        answer: "ok".into(),
        steps_used: 1,
        tokens_used: 100,
        audit_log: vec![AgentAuditEntry {
            timestamp: "2026-01-01T00:00:00Z".into(),
            tool: "search_notes".into(),
            args_summary: "query=test".into(),
            allowed: true,
            reason: "ok".into(),
        }],
    };
    assert_eq!(result.audit_log.len(), 1);
    assert!(result.audit_log[0].allowed);
}

// ── AgentConfig tests ─────────────────────────────────────────────────

#[test]
fn agent_config_default_is_read_only() {
    let config = AgentConfig::default();
    assert_eq!(config.permission, AgentPermission::ReadOnly);
    assert!(config.write_patterns.is_empty());
    assert!(config.allowed_tools.is_empty());
}

#[test]
fn agent_config_resource_limits_default() {
    let limits = AgentResourceLimits::default();
    assert_eq!(limits.max_duration.as_secs(), 300);
    assert_eq!(limits.max_tool_calls, 100);
    assert_eq!(limits.max_tokens, 0); // unlimited
}

#[test]
fn agent_config_serializes_roundtrip() {
    let config = AgentConfig {
        name: "test-agent".into(),
        permission: AgentPermission::ReadWrite,
        limits: AgentResourceLimits {
            max_duration: std::time::Duration::from_secs(60),
            max_tool_calls: 10,
            max_tokens: 5000,
        },
        allowed_tools: vec!["search_notes".into(), "read_file".into()],
        write_patterns: vec!["*.md".into(), "inbox/*".into()],
    };
    let json = serde_json::to_string(&config).unwrap();
    let back: AgentConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "test-agent");
    assert_eq!(back.permission, AgentPermission::ReadWrite);
    assert_eq!(back.limits.max_tool_calls, 10);
    assert_eq!(back.write_patterns.len(), 2);
}
