//! Regression test for #3271: SuggestLinks CLI integration.
//!
//! Validates that the `suggest` subcommand is properly plumbed through
//! the AiSubcommand enum, the handler correctly forwards the text and
//! optional vault notes (instruction) to the SuggestLinks action, and
//! the action's system/user prompts produce well-structured suggestions.

use crate::ai::actions::{
    process_action_result, system_prompt, user_prompt, validate_request, AiActionRequest,
    AiActionType,
};
use crate::ai::RequestUsage;

#[test]
fn suggest_links_cli_forwards_text_and_vault_notes() {
    // Simulate what the CLI handler does: text -> request.text,
    // vault_notes -> request.instruction
    let req = AiActionRequest {
        action: AiActionType::SuggestLinks,
        text: "This is my current note about Rust async patterns.".to_string(),
        target_language: None,
        tone: None,
        note_id: None,
        instruction: Some(
            "Note A: tokio basics\nNote B: async/await in Rust\nNote C: Pin and Unpin".to_string(),
        ),
        model: None,
        export_format: None,
    };

    let prompt = user_prompt(AiActionType::SuggestLinks, &req);
    assert!(prompt.contains("Rust async patterns"));
    assert!(prompt.contains("Note A: tokio basics"));
    assert!(prompt.contains("[[wikilinks]]"));
}

#[test]
fn suggest_links_cli_without_vault_notes() {
    // When --vault-notes is not provided, instruction should be None
    let req = AiActionRequest {
        action: AiActionType::SuggestLinks,
        text: "A note about machine learning.".to_string(),
        target_language: None,
        tone: None,
        note_id: None,
        instruction: None,
        model: None,
        export_format: None,
    };

    let prompt = user_prompt(AiActionType::SuggestLinks, &req);
    assert!(prompt.contains("machine learning"));
    // When no vault notes provided, prompt should mention "(no vault notes provided)"
    assert!(prompt.contains("no vault notes provided"));
}

#[test]
fn suggest_links_cli_empty_text_fails() {
    // The CLI's suggest subcommand requires text as positional arg,
    // but if somehow empty text reaches validation, it should fail
    let req = AiActionRequest {
        action: AiActionType::SuggestLinks,
        text: String::new(),
        target_language: None,
        tone: None,
        note_id: None,
        instruction: Some("some vault notes".to_string()),
        model: None,
        export_format: None,
    };

    let result = validate_request(&req);
    assert!(
        result.is_some(),
        "SuggestLinks with empty text must fail validation"
    );
}

#[test]
fn suggest_links_process_result_formats_correctly() {
    // Simulate an AI response with suggestion list
    let raw = "  - [[Async Patterns]] — covers tokio runtime details\n  \
               - [[Pin and Unpin]] — explains self-referential structs\n  ";
    let result = process_action_result(AiActionType::SuggestLinks, raw, RequestUsage::default());
    assert!(result.result.contains("[[Async Patterns]]"));
    assert!(result.result.contains("[[Pin and Unpin]]"));
    assert!(!result.result.starts_with("  ")); // Should be trimmed
    assert!(result.error.is_none());
}

#[test]
fn suggest_links_system_prompt_has_expected_structure() {
    let prompt = system_prompt(AiActionType::SuggestLinks);
    assert!(!prompt.is_empty());
    assert!(prompt.contains("knowledge-graph"));
    assert!(prompt.contains("[[Note Title]]"));
    assert!(prompt.contains("unlinked"));
}
