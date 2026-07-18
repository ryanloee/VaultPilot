//! Agent session recovery — detect unhealthy sessions and recommend reset (#3103).
//!
//! Implements three frustration signals based on Google Cloud AI Agent Trends 2026
//! and the Context Company frustration-detection framework:
//!
//! 1. **Repetition detection**: consecutive assistant responses are near-duplicates.
//! 2. **Ineffectiveness detection**: user corrections accumulate with no vault references.
//! 3. **Loop detection**: agent re-reads the same file excessively in one session.
//!
//! When a session is flagged as unhealthy, the frontend can surface a recovery prompt
//! offering to reset the context while preserving the most recent turns.

use crate::models::ChatSession;

/// Thresholds — tuned conservatively to avoid false positives on normal usage.
const REPETITION_WINDOW: usize = 3;
const REPETITION_SIMILARITY_THRESHOLD: f64 = 0.85;
const INEFFECTIVENESS_CORRECTION_MIN: usize = 2;
const LOOP_SAME_FILE_MAX: usize = 5;
const LOOP_WINDOW: usize = 12;

/// Result of a session health check.
#[derive(Debug, Clone, PartialEq)]
pub enum HealthSignal {
    /// Session is healthy.
    Healthy,
    /// Session shows signs of repetition.
    Repetition,
    /// User corrected agent repeatedly without useful vault output.
    Ineffective,
    /// Agent looped on the same file too many times.
    Loop,
}

/// Analyse a [`ChatSession`] and return an unhealthy signal if detected.
pub fn check_session_health(session: &ChatSession) -> Option<HealthSignal> {
    if session.turns.len() < 3 {
        return None; // too short to judge
    }

    if let Some(signal) = detect_repetition(session) {
        return Some(signal);
    }
    if let Some(signal) = detect_ineffectiveness(session) {
        return Some(signal);
    }
    if let Some(signal) = detect_loop(session) {
        return Some(signal);
    }
    None
}

// ── Repetition detection ────────────────────────────────────────────────────

fn detect_repetition(session: &ChatSession) -> Option<HealthSignal> {
    let assistant_turns: Vec<&str> = session
        .turns
        .iter()
        .filter(|t| t.role == "assistant")
        .map(|t| t.text.as_str())
        .collect();

    if assistant_turns.len() < REPETITION_WINDOW {
        return None;
    }

    // Look at the most recent REPETITION_WINDOW assistant turns
    let recent = &assistant_turns[assistant_turns.len() - REPETITION_WINDOW..];

    for i in 0..recent.len() - 1 {
        for j in i + 1..recent.len() {
            if simple_text_similarity(recent[i], recent[j]) >= REPETITION_SIMILARITY_THRESHOLD {
                return Some(HealthSignal::Repetition);
            }
        }
    }
    None
}

/// Simple token-overlap similarity (0.0–1.0).  Fast heuristic; production can
/// upgrade to embedding cosine-similarity when the vector store is available.
fn simple_text_similarity(a: &str, b: &str) -> f64 {
    let tokens_a: Vec<&str> = a.split_whitespace().collect();
    let tokens_b: Vec<&str> = b.split_whitespace().collect();

    if tokens_a.is_empty() || tokens_b.is_empty() {
        return 0.0;
    }

    // Use HashSet for O(n+m) intersection
    let set_a: std::collections::HashSet<&str> = tokens_a.iter().copied().collect();
    let set_b: std::collections::HashSet<&str> = tokens_b.iter().copied().collect();

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    intersection as f64 / union as f64
}

// ── Ineffectiveness detection ───────────────────────────────────────────────

fn detect_ineffectiveness(session: &ChatSession) -> Option<HealthSignal> {
    // Count user correction turns — turns where the user says short phrases
    // that indicate frustration (纠正/不对/重新/again/wrong)
    let correction_count = session
        .turns
        .iter()
        .filter(|t| t.role == "user")
        .filter(|t| is_correction_turn(&t.text))
        .count();

    if correction_count < INEFFECTIVENESS_CORRECTION_MIN {
        return None;
    }

    // Check whether recent assistant turns lack vault references
    let assistant_turns: Vec<&str> = session
        .turns
        .iter()
        .filter(|t| t.role == "assistant")
        .map(|t| t.text.as_str())
        .collect();

    // Look at the last 4 assistant turns — if none contain vault references
    // (wiki-links, note titles in backticks, save-note citations), flag it
    let recent_count = assistant_turns.len().min(4);
    let recent = &assistant_turns[assistant_turns.len() - recent_count..];
    let vault_ref_count = recent
        .iter()
        .filter(|t| contains_vault_reference(t))
        .count();

    if vault_ref_count == 0 {
        return Some(HealthSignal::Ineffective);
    }
    None
}

/// Heuristic: does the user message look like a correction/frustration signal?
fn is_correction_turn(text: &str) -> bool {
    let lower = text.to_lowercase();
    let correction_patterns = [
        "不对",
        "错误",
        "重新",
        "纠正",
        "还是不对",
        "no",
        "wrong",
        "incorrect",
        "again",
        "try again",
        "not right",
        "别",
    ];
    // Short messages (≤ 30 chars) that contain a correction keyword
    if lower.chars().count() <= 30 {
        correction_patterns.iter().any(|p| lower.contains(p))
    } else {
        false
    }
}

/// Does the assistant response contain a vault reference?
fn contains_vault_reference(text: &str) -> bool {
    // Wiki-link pattern: [[note]]
    let has_wikilink = text.contains("[[") && text.contains("]]");
    // Saved-note citation or vault operation mention
    let has_vault_op = text.contains("save_note")
        || text.contains("saved")
        || text.contains("创建了笔记")
        || text.contains("更新了笔记")
        || text.contains("read_file")
        || text.contains("search_notes");

    has_wikilink || has_vault_op
}

// ── Loop detection ──────────────────────────────────────────────────────────

fn detect_loop(session: &ChatSession) -> Option<HealthSignal> {
    let turns = &session.turns;

    // Look at the last LOOP_WINDOW turns for excessive same-tool invocation
    let start = if turns.len() > LOOP_WINDOW {
        turns.len() - LOOP_WINDOW
    } else {
        0
    };

    // Extract file references from assistant thinking traces and tool calls
    // The thinking_trace field contains the agent's internal reasoning which
    // often includes tool invocations like read_file("path/to/file.md")
    let file_mentions: Vec<&str> = turns[start..]
        .iter()
        .filter(|t| t.role == "assistant")
        .filter_map(|t| {
            t.thinking_trace
                .as_ref()
                .map(|trace| extract_file_from_trace(&trace.summary))
        })
        .flatten()
        .collect();

    if file_mentions.is_empty() {
        return None;
    }

    // Count the most-referenced file
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for file in &file_mentions {
        *counts.entry(file).or_default() += 1;
    }

    let max_count = counts.values().max().copied().unwrap_or(0);
    if max_count >= LOOP_SAME_FILE_MAX {
        return Some(HealthSignal::Loop);
    }
    None
}

/// Extract a file path from an agent thinking trace.
/// Looks for read_file("path") or read_file('path') patterns.
fn extract_file_from_trace(trace_text: &str) -> Option<&str> {
    // Simple pattern: read_file("...") or read_file('...')
    for delimiter in ['"', '\''] {
        let pattern = format!("read_file({}", delimiter);
        if let Some(pos) = trace_text.find(&pattern) {
            let start = pos + pattern.len();
            if let Some(end) = trace_text[start..].find(delimiter) {
                return Some(&trace_text[start..start + end]);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChatSession, ChatTurn, ThinkingTrace};

    fn make_turn(role: &str, text: &str) -> ChatTurn {
        ChatTurn {
            role: role.to_string(),
            text: text.to_string(),
            ..Default::default()
        }
    }

    fn assistant_with_trace(text: &str, trace: &str) -> ChatTurn {
        ChatTurn {
            role: "assistant".to_string(),
            text: text.to_string(),
            thinking_trace: Some(ThinkingTrace {
                summary: trace.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn session_with_turns(turns: Vec<ChatTurn>) -> ChatSession {
        ChatSession {
            turns,
            ..Default::default()
        }
    }

    // ── simple_text_similarity ──────────────────────────────────────────────

    #[test]
    fn identical_texts_have_similarity_1() {
        let s = simple_text_similarity("hello world", "hello world");
        assert!((s - 1.0).abs() < 0.01);
    }

    #[test]
    fn disjoint_texts_have_similarity_0() {
        let s = simple_text_similarity("hello world", "foo bar baz");
        assert!((s - 0.0).abs() < 0.01);
    }

    #[test]
    fn partial_overlap_50_percent() {
        let s = simple_text_similarity("a b c d", "a b e f");
        // tokens: {a,b,c,d} ∩ {a,b,e,f} = {a,b} → 2/6 ≈ 0.333
        assert!(s > 0.32 && s < 0.34);
    }

    // ── Repetition detection ────────────────────────────────────────────────

    #[test]
    fn healthy_session_no_repetition() {
        let session = session_with_turns(vec![
            make_turn("user", "hi"),
            make_turn("assistant", "Hello! How can I help?"),
            make_turn("user", "tell me about Rust"),
            make_turn(
                "assistant",
                "Rust is a systems language focused on safety...",
            ),
            make_turn("user", "and async?"),
            make_turn(
                "assistant",
                "Async Rust uses futures and tokio for concurrency...",
            ),
        ]);
        let result = check_session_health(&session);
        assert!(result.is_none(), "healthy session flagged: {:?}", result);
    }

    #[test]
    fn repetitive_assistant_detected() {
        let repeated =
            "I found some notes about this topic. They contain information about various files.";
        let session = session_with_turns(vec![
            make_turn("user", "query"),
            make_turn("assistant", repeated),
            make_turn("user", "more"),
            make_turn("assistant", repeated),
            make_turn("user", "again"),
            make_turn("assistant", repeated),
        ]);
        let result = check_session_health(&session);
        assert_eq!(result, Some(HealthSignal::Repetition));
    }

    #[test]
    fn short_session_not_checked() {
        let session = session_with_turns(vec![
            make_turn("user", "hello"),
            make_turn("assistant", "hi"),
        ]);
        assert_eq!(check_session_health(&session), None);
    }

    // ── Ineffectiveness detection ───────────────────────────────────────────

    #[test]
    fn multiple_corrections_no_vault_ref_detected() {
        let session = session_with_turns(vec![
            make_turn("user", "search notes"),
            make_turn(
                "assistant",
                "I think the answer is 42 because of the guide.",
            ),
            make_turn("user", "不对"),
            make_turn("assistant", "Let me reconsider. The answer is still 42."),
            make_turn("user", "还是不对"),
            make_turn("assistant", "After more thought, it's definitely 42."),
        ]);
        let result = check_session_health(&session);
        assert_eq!(result, Some(HealthSignal::Ineffective));
    }

    #[test]
    fn corrections_with_vault_ref_are_healthy() {
        let session = session_with_turns(vec![
            make_turn("user", "search notes"),
            make_turn("assistant", "I think the answer is 42."),
            make_turn("user", "不对"),
            make_turn(
                "assistant",
                "Let me search_notes. Found [[doc]] — answer is 77.",
            ),
            make_turn("user", "还是不对"),
            make_turn(
                "assistant",
                "I read_file and found `note-x` which confirms answer is 99.",
            ),
        ]);
        let result = check_session_health(&session);
        // Repetition may fire first if the correction messages trigger it,
        // but ineffectiveness should NOT fire because vault refs exist.
        assert!(
            result != Some(HealthSignal::Ineffective),
            "vault refs present but flagged ineffective"
        );
    }

    #[test]
    fn single_correction_insufficient() {
        let session = session_with_turns(vec![
            make_turn("user", "query"),
            make_turn("assistant", "I think 42."),
            make_turn("user", "不对"),
            make_turn("assistant", "Okay, it's 77."),
        ]);
        let result = check_session_health(&session);
        // Only 1 correction → shouldn't fire ineffectiveness
        assert!(
            result != Some(HealthSignal::Ineffective),
            "single correction flagged"
        );
    }

    // ── Loop detection ──────────────────────────────────────────────────────

    #[test]
    fn excessive_same_file_read_detected() {
        let trace = r#"I'll read_file("notes/daily.md") to check the contents."#;
        let responses = [
            "I'll check the daily notes for details...",
            "Reading the daily notes again for confirmation...",
            "Let me re-read the daily notes to verify...",
            "Checking daily.md one more time...",
            "I need to review the daily notes once more...",
        ];
        let mut turns = vec![make_turn("user", "start")];
        for (i, resp) in responses.iter().enumerate() {
            if i > 0 {
                turns.push(make_turn("user", &format!("more {}", i)));
            }
            turns.push(assistant_with_trace(resp, trace));
        }
        let session = session_with_turns(turns);
        let result = check_session_health(&session);
        assert_eq!(result, Some(HealthSignal::Loop));
    }

    #[test]
    fn normal_file_reads_not_looping() {
        let trace1 = r#"I'll read_file("notes/a.md")"#;
        let trace2 = r#"I'll read_file("notes/b.md")"#;
        let trace3 = r#"I'll read_file("notes/c.md")"#;
        let session = session_with_turns(vec![
            make_turn("user", "start"),
            assistant_with_trace("reading a", trace1),
            make_turn("user", "next"),
            assistant_with_trace("reading b", trace2),
            make_turn("user", "more"),
            assistant_with_trace("reading c", trace3),
        ]);
        let result = check_session_health(&session);
        assert_eq!(result, None, "diverse files flagged as loop");
    }

    // ── is_correction_turn ──────────────────────────────────────────────────

    #[test]
    fn short_correction_keywords_detected() {
        assert!(is_correction_turn("不对"));
        assert!(is_correction_turn("wrong"));
        assert!(is_correction_turn("还是不对"));
        assert!(is_correction_turn("try again"));
    }

    #[test]
    fn long_messages_with_keywords_are_not_corrections() {
        // A legitimate long question containing "no" shouldn't be flagged
        let long =
            "Can you explain why there is no automatic backup for vaults that use cloud storage?";
        assert!(!is_correction_turn(long));
    }
}
