//! Regression tests for #2804: CLI diff command between two notes.
//!
//! Tests that the vp diff command correctly computes and renders
//! line-level diffs between note bodies using the built-in Myers
//! diff algorithm.

#[cfg(test)]
mod tests {
    use crate::diff::{compute_diff, render_unified_diff, DiffLine};

    /// Two identical notes should produce an empty diff.
    #[test]
    fn diff_identical_notes() {
        let body = "Line 1\nLine 2\nLine 3\n";
        let result = compute_diff(body, body, 3);
        assert!(result.is_empty(), "identical texts should have no hunks");
        assert_eq!(result.additions, 0);
        assert_eq!(result.deletions, 0);
    }

    /// A single-line change should produce one hunk with context.
    #[test]
    fn diff_single_line_change() {
        let old = "Line 1\nLine 2\nLine 3\n";
        let new = "Line 1\nLine 2 changed\nLine 3\n";
        let result = compute_diff(old, new, 3);
        assert!(!result.is_empty());
        assert_eq!(result.additions, 1);
        assert_eq!(result.deletions, 1);
        assert_eq!(result.hunks.len(), 1);

        let hunk = &result.hunks[0];
        let has_context_1 = hunk
            .lines
            .iter()
            .any(|l| matches!(l, DiffLine::Context(s) if s == "Line 1"));
        let has_context_3 = hunk
            .lines
            .iter()
            .any(|l| matches!(l, DiffLine::Context(s) if s == "Line 3"));
        let has_delete = hunk
            .lines
            .iter()
            .any(|l| matches!(l, DiffLine::Delete(s) if s == "Line 2"));
        let has_insert = hunk
            .lines
            .iter()
            .any(|l| matches!(l, DiffLine::Insert(s) if s == "Line 2 changed"));
        assert!(has_context_1, "should have context Line 1");
        assert!(has_context_3, "should have context Line 3");
        assert!(has_delete, "should delete Line 2");
        assert!(has_insert, "should insert Line 2 changed");
    }

    /// Adding lines should produce insert-only hunks.
    #[test]
    fn diff_added_lines() {
        let old = "Line 1\n";
        let new = "Line 1\nLine 2 new\nLine 3 new\n";
        let result = compute_diff(old, new, 3);
        assert_eq!(result.additions, 2);
        assert_eq!(result.deletions, 0);
        assert!(!result.is_empty());
    }

    /// Deleting lines should produce delete-only hunks.
    #[test]
    fn diff_deleted_lines() {
        let old = "Line 1\nLine 2\nLine 3\n";
        let new = "Line 1\n";
        let result = compute_diff(old, new, 3);
        assert_eq!(result.additions, 0);
        assert_eq!(result.deletions, 2);
        assert!(!result.is_empty());
    }

    /// Unicode content should diff correctly (no byte-index panics).
    #[test]
    fn diff_unicode_content() {
        let old = "你好世界\nこんにちは\nBonjour\n";
        let new = "你好世界\nこんにちは世界\nBonjour\n";
        let result = compute_diff(old, new, 3);
        assert_eq!(result.additions, 1);
        assert_eq!(result.deletions, 1);
        assert!(!result.is_empty());
    }

    /// Unified diff rendering should include proper headers.
    #[test]
    fn unified_diff_headers() {
        let old = "Line 1\nLine 2\n";
        let new = "Line 1\nLine 2 modified\n";
        let result = compute_diff(old, new, 3);
        let rendered = render_unified_diff(&result, "a/note1", "b/note2");
        assert!(rendered.contains("--- a/note1"));
        assert!(rendered.contains("+++ b/note2"));
        assert!(rendered.contains("@@"));
    }

    /// Colored diff should contain ANSI escape codes.
    #[test]
    fn colored_diff_has_ansi() {
        let old = "Line 1\nLine 2\n";
        let new = "Line 1\nLine 2 changed\n";
        let result = compute_diff(old, new, 3);
        let rendered = crate::diff::render_colored_diff(&result);
        assert!(rendered.contains("\x1b["));
    }

    /// Empty (no changes) colored diff should produce nothing.
    #[test]
    fn colored_diff_empty() {
        let text = "Line 1\nLine 2\n";
        let result = compute_diff(text, text, 3);
        let rendered = crate::diff::render_colored_diff(&result);
        assert!(rendered.is_empty());
    }
}
