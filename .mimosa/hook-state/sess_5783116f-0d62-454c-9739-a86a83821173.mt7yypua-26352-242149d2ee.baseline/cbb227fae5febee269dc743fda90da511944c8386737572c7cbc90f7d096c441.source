//! Regression test for issue #3077: Feed entry source-based dedup.
//!
//! Bug:      `save_note_with_context` always generates a fresh UUID when
//!           `meta.id` is empty, even when `meta.source` matches an existing
//!           note. Feed poller calls `save_note_with_context` without setting
//!           `meta.id`, so every poll creates duplicate notes for entries
//!           that are already in the vault.
//! Root cause: `save_note_with_images_with_context` did not check for existing
//!             notes by `source` before generating a new UUID.
//! Fix:      Before generating a new UUID, query the `notes` table for an
//!           existing note with the same `source` value and reuse its id
//!           (turning the save into an update/overwrite).

#[cfg(test)]
mod tests {
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        initialize_storage_with_context, notes::save_note_with_context, StorageContext,
    };
    use std::fs;

    /// Helper: create a temp dir and initialised StorageContext.
    fn setup() -> (std::path::PathBuf, StorageContext) {
        let dir = std::env::temp_dir().join(format!(
            "vp-issue-3077-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let ctx = StorageContext::for_test(&dir);
        initialize_storage_with_context(&ctx).expect("init storage");
        (dir, ctx)
    }

    /// Verify that saving two notes with the same `source` does NOT create a
    /// duplicate — the second save should reuse the existing note's ID.
    #[test]
    fn regression_3077_source_based_dedup() {
        let (_temp, ctx) = setup();

        // Arrange: create a note with a specific source (e.g. a feed entry link).
        let note1 = NoteDocument {
            meta: NoteMeta {
                title: "First entry".to_string(),
                source: "https://example.com/blog/1".to_string(),
                tags: vec!["rss".to_string(), "tech".to_string()],
                ..Default::default()
            },
            body: "Original body content.".to_string(),
            search_snippet: None,
            search_score: None,
        };

        let saved1 = save_note_with_context(&ctx, note1).expect("first save");

        // Act: save another note with the same source (simulating a re-poll).
        let note2 = NoteDocument {
            meta: NoteMeta {
                title: "First entry (updated)".to_string(),
                source: "https://example.com/blog/1".to_string(),
                tags: vec!["rss".to_string(), "tech".to_string()],
                ..Default::default()
            },
            body: "Updated body content.".to_string(),
            search_snippet: None,
            search_score: None,
        };

        let saved2 = save_note_with_context(&ctx, note2).expect("second save (dedup)");

        // Assert: the second save should have reused the first note's ID.
        assert_eq!(
            saved1.meta.id, saved2.meta.id,
            "expected same id for notes with identical source"
        );
        // The body should reflect the update (second save wins).
        assert!(
            saved2.body.contains("Updated body"),
            "expected body to reflect the second save"
        );

        // Verify only one note exists with this source in the DB.
        let (connection, _) = crate::storage::pool::open_connection(&ctx).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE source = ?1",
                ["https://example.com/blog/1"],
                |row| row.get(0),
            )
            .expect("count query");
        assert_eq!(
            count, 1,
            "expected exactly one note with this source, got {count}"
        );
    }

    /// Edge case: notes with empty source should behave as before (always new UUID).
    #[test]
    fn regression_3077_empty_source_no_dedup() {
        let (_temp, ctx) = setup();

        let note1 = NoteDocument {
            meta: NoteMeta {
                title: "No source".to_string(),
                source: String::new(),
                ..Default::default()
            },
            body: "Body A".to_string(),
            search_snippet: None,
            search_score: None,
        };

        let note2 = NoteDocument {
            meta: NoteMeta {
                title: "No source again".to_string(),
                source: String::new(),
                ..Default::default()
            },
            body: "Body B".to_string(),
            search_snippet: None,
            search_score: None,
        };

        let s1 = save_note_with_context(&ctx, note1).expect("first");
        let s2 = save_note_with_context(&ctx, note2).expect("second");

        // Empty source notes should get different IDs.
        assert_ne!(
            s1.meta.id, s2.meta.id,
            "notes with empty source must get different ids"
        );
    }

    /// Edge case: different sources produce different notes (no false dedup).
    #[test]
    fn regression_3077_different_sources_no_collision() {
        let (_temp, ctx) = setup();

        let note_a = NoteDocument {
            meta: NoteMeta {
                title: "Source A".to_string(),
                source: "https://example.com/a".to_string(),
                ..Default::default()
            },
            body: "A".to_string(),
            search_snippet: None,
            search_score: None,
        };

        let note_b = NoteDocument {
            meta: NoteMeta {
                title: "Source B".to_string(),
                source: "https://example.com/b".to_string(),
                ..Default::default()
            },
            body: "B".to_string(),
            search_snippet: None,
            search_score: None,
        };

        let sa = save_note_with_context(&ctx, note_a).expect("a");
        let sb = save_note_with_context(&ctx, note_b).expect("b");

        assert_ne!(
            sa.meta.id, sb.meta.id,
            "different sources must get different ids"
        );
    }
}
