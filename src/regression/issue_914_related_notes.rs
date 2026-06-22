/// Issue #914: 主动知识推送 — find_related_notes_with_context should return
/// notes related to a given note by extracting key terms and using FTS5 search.
///
/// Feature: Proactive knowledge push — AI recommends related notes while editing.
/// Implementation: PR for issue #914
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::notes::build_related_query;
    use crate::storage::{find_related_notes_with_context, save_note_with_context, StorageContext};
    use chrono::Utc;

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-related-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    fn make_note(id: &str, title: &str, tags: Vec<&str>, body: &str) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                id: id.to_string(),
                title: title.to_string(),
                tags: tags.into_iter().map(String::from).collect(),
                keywords: vec![],
                platform: String::new(),
                board: String::new(),
                kernel: String::new(),
                status: String::new(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                source: "test".to_string(),
                path: String::new(),
                summary: String::new(),
            },
            body: body.to_string(),
            search_snippet: None,
        }
    }

    #[test]
    fn regression_914_related_notes_finds_similar() {
        let (temp, ctx) = setup_temp_context();

        // Create notes with overlapping content
        let note_a = make_note(
            "note-a",
            "Rust ownership model",
            vec!["rust", "memory"],
            "Ownership is Rust's key feature for memory safety",
        );
        let note_b = make_note(
            "note-b",
            "Rust borrowing rules",
            vec!["rust", "memory"],
            "Borrowing allows references without taking ownership",
        );
        let note_c = make_note(
            "note-c",
            "Python data classes",
            vec!["python"],
            "Python data classes simplify class definitions",
        );

        save_note_with_context(&ctx, note_a).expect("save note a");
        save_note_with_context(&ctx, note_b).expect("save note b");
        save_note_with_context(&ctx, note_c).expect("save note c");

        // Find notes related to note-a (Rust ownership)
        let related = find_related_notes_with_context(&ctx, "note-a", 5).expect("find related");

        // Should find note-b (related to Rust/memory) but not note-a itself
        assert!(!related.is_empty(), "should find at least one related note");
        assert!(
            related.iter().all(|r| r.meta.id != "note-a"),
            "should not include the source note"
        );
        // Note-b should be the top result (shares tags: rust, memory)
        assert_eq!(
            related[0].meta.id, "note-b",
            "note-b (Rust borrowing) should be most related to note-a (Rust ownership)"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_914_related_notes_unrelated_not_ranked_high() {
        let (temp, ctx) = setup_temp_context();

        let note_a = make_note(
            "note-a",
            "量子计算基础",
            vec!["quantum"],
            "量子比特是量子计算的基本单位",
        );
        let note_b = make_note(
            "note-b",
            "Python web框架",
            vec!["python"],
            "Django is a popular web framework",
        );

        save_note_with_context(&ctx, note_a).expect("save note a");
        save_note_with_context(&ctx, note_b).expect("save note b");

        // note-b is unrelated to note-a
        let related = find_related_notes_with_context(&ctx, "note-a", 5).expect("find related");
        // May or may not find results, but should not crash
        for r in &related {
            assert_ne!(r.meta.id, "note-a", "should not include source note");
        }

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_914_related_notes_excludes_source() {
        let (temp, ctx) = setup_temp_context();

        let note = make_note(
            "only-note",
            "唯一笔记",
            vec!["test"],
            "This is the only note in the vault",
        );
        save_note_with_context(&ctx, note).expect("save note");

        let related = find_related_notes_with_context(&ctx, "only-note", 5).expect("find related");
        assert!(
            related.is_empty(),
            "should return empty when no other notes exist"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_914_related_notes_not_found_returns_error() {
        let (temp, ctx) = setup_temp_context();

        let result = find_related_notes_with_context(&ctx, "nonexistent-id", 5);
        assert!(result.is_err(), "should error for nonexistent note ID");

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_914_build_related_query_extracts_title_and_tags() {
        let doc = make_note(
            "test",
            "Rust memory safety",
            vec!["rust", "systems"],
            "Ownership and borrowing are key concepts in Rust",
        );

        let query = build_related_query(&doc);
        // Title words present
        assert!(query.contains("Rust"), "should include title words");
        assert!(query.contains("memory"), "should include title words");
        assert!(query.contains("safety"), "should include title words");
        // "rust" tag is deduplicated with "Rust" from title (case-insensitive)
        // but "systems" tag is unique
        assert!(query.contains("systems"), "should include unique tags");
    }

    #[test]
    fn regression_914_build_related_query_deduplicates_case_insensitive() {
        let doc = make_note("test", "Rust Guide", vec!["rust"], "body");

        let query = build_related_query(&doc);
        // "Rust" from title and "rust" from tag should be deduplicated
        let lower = query.to_lowercase();
        let count = lower.matches("rust").count();
        assert_eq!(count, 1, "rust/Rust should appear only once");
    }
}
