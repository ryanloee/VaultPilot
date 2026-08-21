//! Regression test for issue #1354: .expect() replaced with graceful error handling in agent.rs

use crate::agent::{AgentConfig, AgentPermission, AgentResourceLimits, ToolProxy};

#[test]
fn regression_1354_audit_log_survives_normal_operation() {
    let config = AgentConfig {
        name: "test".into(),
        permission: AgentPermission::ReadOnly,
        limits: AgentResourceLimits::default(),
        allowed_tools: vec![],
        write_patterns: vec![],
        execution_mode: Default::default(),
    };
    let proxy = ToolProxy::new(config, "/tmp");

    let _ = proxy.check_tool_call("search_notes", r#"{"query":"test","limit":5}"#);
    let _ = proxy.check_tool_call("list_notes", r#"{"limit":10}"#);

    let log = proxy.audit_log();
    assert_eq!(log.len(), 2);
    assert!(log[0].allowed);
    assert_eq!(log[0].tool, "search_notes");
}

#[test]
fn regression_1354_deny_entry_recorded_in_audit_log() {
    let config = AgentConfig {
        name: "test".into(),
        permission: AgentPermission::ReadOnly,
        limits: AgentResourceLimits::default(),
        allowed_tools: vec!["search_notes".into()],
        write_patterns: vec![],
        execution_mode: Default::default(),
    };
    let proxy = ToolProxy::new(config, "/tmp");

    let result = proxy.check_tool_call("save_note", r#"{"title":"t","body":"b"}"#);
    assert!(result.is_ok());
    assert!(!result.unwrap().allowed);

    let log = proxy.audit_log();
    assert_eq!(log.len(), 1);
    assert!(!log[0].allowed);
}

#[test]
fn regression_1354_multiple_tool_calls_audit_log_ordering() {
    let config = AgentConfig {
        name: "test".into(),
        permission: AgentPermission::ReadOnly,
        limits: AgentResourceLimits::default(),
        allowed_tools: vec![],
        write_patterns: vec![],
        execution_mode: Default::default(),
    };
    let proxy = ToolProxy::new(config, "/tmp");

    let _ = proxy.check_tool_call("search_notes", r#"{"query":"a","limit":1}"#);
    let _ = proxy.check_tool_call("read_file", r#"{"path":"test.md"}"#);
    let _ = proxy.check_tool_call("list_directory", r#"{"path":"."}"#);

    let log = proxy.audit_log();
    assert_eq!(log.len(), 3);
    assert_eq!(log[0].tool, "search_notes");
    assert_eq!(log[1].tool, "read_file");
    assert_eq!(log[2].tool, "list_directory");
}
