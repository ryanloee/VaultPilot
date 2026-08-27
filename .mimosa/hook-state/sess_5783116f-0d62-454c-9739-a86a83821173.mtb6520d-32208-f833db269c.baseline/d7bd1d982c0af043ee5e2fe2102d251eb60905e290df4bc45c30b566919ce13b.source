//! Regression test for #3258 — Kanban group_by on multi-value fields
//!
//! When `group_by` is set to a multi-value NoteMeta field (`tags`,
//! `keywords`, or `collections`), each note must be expanded into one
//! `(key, row)` pair per element so it appears in every matching swimlane,
//! rather than being bucketed under a single composite key like `"rust, ai"`.

mod tests {
    use crate::bases::build_kanban_groups;
    use crate::bases::DEFAULT_KANBAN_UNGROUPED;

    fn row(id: &str) -> crate::bases::BaseRow {
        crate::bases::BaseRow {
            note_id: id.into(),
            title: String::new(),
            values: Vec::new(),
        }
    }

    #[test]
    fn issue_3258_multi_tag_note_appears_in_both_declared_columns() {
        // Note with tags: [rust, ai] should appear in BOTH "rust" and "ai" columns.
        let pairs = vec![
            ("rust".to_string(), row("n1")),
            ("ai".to_string(), row("n1")),   // same note, second tag
            ("rust".to_string(), row("n2")), // pure rust note
        ];
        let order = vec!["rust".to_string(), "ai".to_string()];
        let groups = build_kanban_groups(pairs, Some(&order));

        assert_eq!(
            groups.len(),
            2,
            "only two declared columns, no spurious composite"
        );
        assert_eq!(groups[0].key, "rust");
        assert_eq!(groups[0].notes.len(), 2, "n1 + n2 in rust");
        assert_eq!(groups[1].key, "ai");
        assert_eq!(groups[1].notes.len(), 1, "n1 appears in ai too");

        let rust_ids: Vec<&str> = groups[0].notes.iter().map(|r| r.note_id.as_str()).collect();
        assert!(rust_ids.contains(&"n1"));
        assert!(rust_ids.contains(&"n2"));

        let ai_ids: Vec<&str> = groups[1].notes.iter().map(|r| r.note_id.as_str()).collect();
        assert!(ai_ids.contains(&"n1"));
    }

    #[test]
    fn issue_3258_empty_tags_note_lands_in_ungrouped() {
        // A note with empty tags array → ungrouped bucket.
        let pairs = vec![
            (DEFAULT_KANBAN_UNGROUPED.to_string(), row("n1")),
            ("rust".to_string(), row("n2")),
        ];
        let order = vec!["rust".to_string()];
        let groups = build_kanban_groups(pairs, Some(&order));

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, "rust");
        assert_eq!(groups[0].notes.len(), 1);
        assert_eq!(groups[1].key, DEFAULT_KANBAN_UNGROUPED);
        assert_eq!(groups[1].notes.len(), 1);
        assert_eq!(groups[1].notes[0].note_id, "n1");
    }

    #[test]
    fn issue_3258_mixed_single_and_multi_tag_notes_correct_counts() {
        // Mixed scenario: single-tag, multi-tag, and empty tags.
        let pairs = vec![
            ("rust".to_string(), row("n1")),                   // pure rust
            ("rust".to_string(), row("n2")),                   // multi: rust+ai → appears in rust
            ("ai".to_string(), row("n2")),                     // multi: rust+ai → appears in ai
            ("ai".to_string(), row("n3")),                     // pure ai
            (DEFAULT_KANBAN_UNGROUPED.to_string(), row("n4")), // empty tags
        ];
        let order = vec!["rust".to_string(), "ai".to_string()];
        let groups = build_kanban_groups(pairs, Some(&order));

        assert_eq!(groups.len(), 3, "rust, ai, + ungrouped");
        assert_eq!(groups[0].key, "rust");
        assert_eq!(groups[0].notes.len(), 2, "n1 + n2");
        assert_eq!(groups[1].key, "ai");
        assert_eq!(groups[1].notes.len(), 2, "n2 + n3");
        assert_eq!(groups[2].key, DEFAULT_KANBAN_UNGROUPED);
        assert_eq!(groups[2].notes.len(), 1, "n4");
    }
}
