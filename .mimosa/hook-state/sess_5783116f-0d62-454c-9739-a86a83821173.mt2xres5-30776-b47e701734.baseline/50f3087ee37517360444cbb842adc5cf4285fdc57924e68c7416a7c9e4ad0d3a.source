//! Regression test for issue #3084: Feed source-based dedup overwrites created_at.
//!
//! Bug:      When `save_note_with_images_with_context` resolves an existing
//!           note via source-based dedup (#3077), it recomputes `created_at`
//!           as `now` (since the feed poller passes `..Default::default()`
//!           which leaves `created_at` empty). This overwrites the original
//!           creation date on every re-poll of a feed entry.
//! Root cause: `created_at` was computed unconditionally at line 115-119
//!             *after* source-based dedup (lines 69-105) had already resolved
//!             to an existing note ID, but without any check to preserve the
//!             existing note's original `created_at`.
//! Fix:      After source-based dedup matches an existing note, load its
//!           `created_at` from the database and use that value instead of
//!           recomputing from the incoming (empty) `note.meta.created_at`.

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
            "vp-issue-3084-test-{}-{}",
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

    /// Verify that re-saving a note with the same source preserves the
    /// original `created_at` instead of resetting it to "now".
    #[test]
    fn regression_3084_source_dedup_preserves_created_at() {
        let (_temp, ctx) = setup();

        // Arrange: save a note with a specific source and a fixed created_at.
        let fixed_created = "2026-01-15T10:00:00+00:00".to_string();
        let note1 = NoteDocument {
            meta: NoteMeta {
                title: "Original entry".to_string(),
                source: "https://example.com/blog/1".to_string(),
                created_at: fixed_created.clone(),
                tags: vec!["rss".to_string()],
                ..Default::default()
            },
            body: "Original content.".to_string(),
            search_snippet: None,
            search_score: None,
        };

        let saved1 = save_note_with_context(&ctx, note1).expect("first save");
        assert_eq!(
            saved1.meta.created_at, fixed_created,
            "first save should have the fixed created_at"
        );

        // Act: simulate re-poll — save with same source but ..Default::default()
        // (feed poller does not set meta.id or meta.created_at).
        let note2 = NoteDocument {
            meta: NoteMeta {
                title: "Updated entry".to_string(),
                source: "https://example.com/blog/1".to_string(),
                tags: vec!["rss".to_string()],
                ..Default::default()
            },
            body: "Updated content.".to_string(),
            search_snippet: None,
            search_score: None,
        };

        let saved2 = save_note_with_context(&ctx, note2).expect("second save (re-poll)");

        // Assert: the created_at should be preserved from the original note,
        // NOT reset to the current time.
        assert_eq!(
            saved2.meta.created_at, fixed_created,
            "re-poll should preserve original created_at, got '{}'",
            saved2.meta.created_at
        );

        // The ID should also be preserved (source-based dedup from #3077).
        assert_eq!(
            saved1.meta.id, saved2.meta.id,
            "re-poll should reuse the same ID"
        );
    }

    /// Verify that notes with different sources get different created_at
    /// values (no false preservation across unrelated notes).
    #[test]
    fn regression_3084_different_sources_independent_created_at() {
        let (_temp, ctx) = setup();

        let note_a = NoteDocument {
            meta: NoteMeta {
                title: "Source A".to_string(),
                source: "https://example.com/a".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
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
                created_at: "2026-06-15T12:00:00Z".to_string(),
                ..Default::default()
            },
            body: "B".to_string(),
            search_snippet: None,
            search_score: None,
        };

        let sa = save_note_with_context(&ctx, note_a).expect("a");
        let sb = save_note_with_context(&ctx, note_b).expect("b");

        // Each should keep its own created_at.
        assert_eq!(sa.meta.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(sb.meta.created_at, "2026-06-15T12:00:00Z");
        assert_ne!(sa.meta.id, sb.meta.id);
    }
}
