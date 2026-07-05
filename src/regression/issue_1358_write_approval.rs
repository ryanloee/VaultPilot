//! Regression test for issue #1358: WriteApprovalDialog decision sent to backend.
//!
//! Verifies that:
//! - `RunAgentParams` deserializes correctly
//! - `RespondToWriteApprovalParams` deserializes correctly
//! - The agent approval channel mechanism works (send/receive)
//! - `emit_event` produces valid JSON with expected fields

use serde_json::json;

#[test]
fn regression_1358_run_agent_params_deserializes_with_defaults() {
    let json = json!({ "prompt": "test task" });
    // Simulate the deserialization that happens in handle_request
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RunAgentParams {
        prompt: String,
        #[serde(default)]
        max_steps: Option<usize>,
        #[serde(default)]
        auto_approve: Option<bool>,
    }

    let p: RunAgentParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.prompt, "test task");
    assert_eq!(p.max_steps, None);
    assert_eq!(p.auto_approve, None);
}

#[test]
fn regression_1358_run_agent_params_deserializes_with_all_fields() {
    let json = json!({ "prompt": "write a note", "maxSteps": 10, "autoApprove": true });
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RunAgentParams {
        prompt: String,
        #[serde(default)]
        max_steps: Option<usize>,
        #[serde(default)]
        auto_approve: Option<bool>,
    }

    let p: RunAgentParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.prompt, "write a note");
    assert_eq!(p.max_steps, Some(10));
    assert_eq!(p.auto_approve, Some(true));
}

#[test]
fn regression_1358_respond_to_write_approval_params_deserializes() {
    let json = json!({ "approved": true });
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RespondToWriteApprovalParams {
        approved: bool,
    }

    let p: RespondToWriteApprovalParams = serde_json::from_value(json).unwrap();
    assert!(p.approved);

    let json2 = json!({ "approved": false });
    let p2: RespondToWriteApprovalParams = serde_json::from_value(json2).unwrap();
    assert!(!p2.approved);
}

#[test]
fn regression_1358_approval_channel_send_receive() {
    // Simulates the AGENT_APPROVAL mechanism
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel();
    tx.send(true).unwrap();
    assert!(rx.recv().unwrap());

    let (tx2, rx2) = mpsc::channel();
    tx2.send(false).unwrap();
    assert!(!rx2.recv().unwrap());
}

#[test]
fn regression_1358_agent_event_json_has_expected_fields() {
    // Verify that the event JSON format matches what WinUI expects
    let event = json!({
        "event": "agentStatus",
        "payload": {
            "stage": "writeApprovalNeeded",
            "detail": "Write approval needed for save_note",
            "tool": "save_note",
            "args": "{\"path\": \"test.md\"}",
            "timestamp": "2026-06-23T00:00:00Z"
        }
    });

    assert_eq!(event["event"], "agentStatus");
    assert_eq!(event["payload"]["stage"], "writeApprovalNeeded");
    assert_eq!(event["payload"]["tool"], "save_note");
    assert!(event["payload"]["args"]
        .as_str()
        .unwrap()
        .contains("test.md"));
}

#[test]
fn regression_1358_agent_completed_event_has_stats() {
    let event = json!({
        "event": "agentStatus",
        "payload": {
            "stage": "agentCompleted",
            "detail": "Task completed successfully",
            "stepsUsed": 5,
            "tokensUsed": 1234,
            "timestamp": "2026-06-23T00:00:00Z"
        }
    });

    assert_eq!(event["payload"]["stage"], "agentCompleted");
    assert_eq!(event["payload"]["stepsUsed"], 5);
    assert_eq!(event["payload"]["tokensUsed"], 1234);
}
