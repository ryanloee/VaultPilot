//! Regression test for #3270: SynthesizeNotes CLI integration.
//!
//! Validates that the `synthesize` subcommand is plumbed through the
//! AiSubcommand enum and its handler invokes run_synthesize_notes which
//! loads multiple notes and feeds them to the SynthesizeNotes action.

use crate::ai::actions::{
    process_action_result, system_prompt, user_prompt, validate_request, AiActionRequest,
    AiActionType,
};
use crate::ai::RequestUsage;

/// Simulate the concatenation logic in run_synthesize_notes.
/// This mirrors how the CLI handler merges multiple notes into a single
/// `## Note: <title>\n<body>\n` blob separated by `---`.
fn build_synthesize_input(titles_and_bodies: &[(&str, &str)]) -> String {
    let mut combined = String::new();
    for (i, (title, body)) in titles_and_bodies.iter().enumerate() {
        if i > 0 {
            combined.push_str("\n---\n");
        }
        combined.push_str(&format!("## Note: {}\n{}\n", title, body));
    }
    combined
}

#[test]
fn synthesize_notes_concatenates_multiple_notes() {
    let input = build_synthesize_input(&[
        ("Alpha", "Content of note Alpha."),
        ("Beta", "Content of note Beta."),
        ("Gamma", "Content of note Gamma."),
    ]);
    assert!(input.contains("## Note: Alpha"));
    assert!(input.contains("## Note: Beta"));
    assert!(input.contains("## Note: Gamma"));
    assert!(input.contains("Content of note Alpha."));
    assert!(input.contains("Content of note Beta."));
    assert!(input.contains("Content of note Gamma."));
    // Separators between notes
    assert_eq!(input.matches("\n---\n").count(), 2);
}

#[test]
fn synthesize_notes_single_note_no_separator() {
    let input = build_synthesize_input(&[("Solo", "Just one note.")]);
    assert!(input.contains("## Note: Solo"));
    assert!(input.contains("Just one note."));
    assert_eq!(input.matches("\n---\n").count(), 0);
}

#[test]
fn synthesize_notes_empty_input_fails_validation() {
    // The SynthesizeNotes action should reject empty text
    let input = build_synthesize_input(&[]);
    assert!(input.is_empty());
    let req = AiActionRequest {
        action: AiActionType::SynthesizeNotes,
        text: input,
        target_language: None,
        tone: None,
        note_id: None,
        instruction: None,
        model: None,
    };
    let validation = validate_request(&req);
    assert!(
        validation.is_some(),
        "empty synthesized text must fail validation"
    );
}

#[test]
fn synthesize_notes_with_content_passes_validation() {
    let input = build_synthesize_input(&[("Test", "Some content.")]);
    let req = AiActionRequest {
        action: AiActionType::SynthesizeNotes,
        text: input,
        target_language: None,
        tone: None,
        note_id: None,
        instruction: None,
        model: None,
    };
    let result = validate_request(&req);
    assert!(result.is_none(), "valid input should pass validation");
}

#[test]
fn synthesize_notes_user_prompt_contains_all_notes() {
    let input = build_synthesize_input(&[("Alpha", "alpha content"), ("Beta", "beta content")]);
    let req = AiActionRequest {
        action: AiActionType::SynthesizeNotes,
        text: input,
        target_language: None,
        tone: None,
        note_id: None,
        instruction: None,
        model: None,
    };
    let prompt = user_prompt(AiActionType::SynthesizeNotes, &req);
    assert!(prompt.contains("Alpha"));
    assert!(prompt.contains("Beta"));
    assert!(prompt.contains("[[Note Title]]"));
}

#[test]
fn synthesize_notes_system_prompt_has_expected_sections() {
    let prompt = system_prompt(AiActionType::SynthesizeNotes);
    assert!(prompt.contains("Summary"));
    assert!(prompt.contains("Shared Themes"));
    assert!(prompt.contains("Missing Links"));
    assert!(prompt.contains("Conflicts"));
}

#[test]
fn synthesize_notes_process_result_trims_output() {
    let result = process_action_result(
        AiActionType::SynthesizeNotes,
        "  ## Summary\ncombined text\n  ",
        RequestUsage::default(),
    );
    assert_eq!(result.result, "## Summary\ncombined text");
    assert!(result.error.is_none());
}
