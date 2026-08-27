//! Regression tests for AI Chat @-mention context injection (#3548).
//!
//! Tests the parsing layer (`parse_at_mentions`) and the truncation logic
//! (`truncate_note_content`) without requiring a vault or network access.
//! The end-to-end injection path (`inject_mention_context`) is covered by
//! an inline test in `orchestration/mention.rs`.

#[cfg(test)]
mod tests {
    use crate::orchestration::mention::{
        parse_at_mentions, truncate_note_content, MAX_MENTION_NOTES, MAX_NOTE_CONTENT_CHARS,
        MENTION_CONTEXT_HEADER,
    };

    #[test]
    fn regression_3548_simple_word_mention() {
        assert_eq!(parse_at_mentions("@rust"), vec!["rust"]);
    }

    #[test]
    fn regression_3548_bracketed_multi_word() {
        assert_eq!(
            parse_at_mentions("@[Meeting Notes July]"),
            vec!["Meeting Notes July"]
        );
    }

    #[test]
    fn regression_3548_multiple_mentions_in_sentence() {
        let text = "Compare @rust with @[Python Language] for this task";
        assert_eq!(parse_at_mentions(text), vec!["rust", "Python Language"]);
    }

    #[test]
    fn regression_3548_email_not_false_positive() {
        assert!(parse_at_mentions("user@example.com").is_empty());
        assert!(parse_at_mentions("contact: foo.bar@baz.com").is_empty());
    }

    #[test]
    fn regression_3548_cjk_mention_works() {
        // Preceding CJK char should NOT block mention detection
        assert_eq!(parse_at_mentions("关于@笔记"), vec!["笔记"]);
        assert_eq!(parse_at_mentions("@[会议记录]"), vec!["会议记录"]);
    }

    #[test]
    fn regression_3548_deduplication() {
        let mentions = parse_at_mentions("@a @[A] @a");
        assert_eq!(mentions, vec!["a", "A"]); // "a" and "A" are different
    }

    #[test]
    fn regression_3548_truncation_at_boundary() {
        let body = "x".repeat(MAX_NOTE_CONTENT_CHARS);
        assert_eq!(truncate_note_content(&body), body);

        let too_long = "x".repeat(MAX_NOTE_CONTENT_CHARS + 1);
        let result = truncate_note_content(&too_long);
        assert!(result.contains("截断"));
    }

    #[test]
    fn regression_3548_constants_sensible() {
        // Verify the limits are within reasonable bounds at runtime
        let max_notes = MAX_MENTION_NOTES;
        let max_chars = MAX_NOTE_CONTENT_CHARS;
        assert!(
            max_notes >= 3,
            "should allow at least 3 notes, got {max_notes}"
        );
        assert!(
            max_chars >= 1000,
            "should allow at least 1000 chars per note, got {max_chars}"
        );
        assert!(!MENTION_CONTEXT_HEADER.is_empty());
    }
}
