//! Regression test for issue #3078: Vault Changelog CLI command.
//!
//! Verifies that notes can be queried by modification time and that the
//! date extraction logic works correctly.

#[cfg(test)]
mod tests {
    use crate::models::{NoteDocument, NoteMeta, SearchQuery};
    use crate::storage::{
        initialize_storage_with_context, notes::save_note_with_context, search_notes_with_context,
        StorageContext,
    };
    use std::fs;

    /// Helper: create a temp dir and initialised StorageContext.
    fn setup() -> (std::path::PathBuf, StorageContext) {
        let dir = std::env::temp_dir().join(format!(
            "vp-issue-3078-test-{}-{}",
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

    /// Verify that searching by modified_after returns notes changed within
    /// the time window.
    #[test]
    fn regression_3078_changelog_modified_after_filter() {
        let (_temp, ctx) = setup();

        // Create a note that was "modified" recently
        let note = NoteDocument {
            meta: NoteMeta {
                title: "Changelog test note".to_string(),
                tags: vec!["test".to_string()],
                summary: "A note for changelog testing".to_string(),
                ..Default::default()
            },
            body: "Test body".to_string(),
            search_snippet: None,
            search_score: None,
        };
        save_note_with_context(&ctx, note).expect("save note");

        // Query with modified_after = now minus 1 hour
        let since = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let query = SearchQuery {
            modified_after: Some(since),
            limit: Some(100),
            ..Default::default()
        };
        let result = search_notes_with_context(&ctx, query).expect("search");
        assert!(
            !result.notes.is_empty(),
            "Should find at least the note we just saved"
        );

        // Query with modified_after = now plus 1 hour (future) => should be empty
        let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let query_future = SearchQuery {
            modified_after: Some(future),
            limit: Some(100),
            ..Default::default()
        };
        let result_future = search_notes_with_context(&ctx, query_future).expect("search");
        assert!(
            result_future.notes.is_empty(),
            "Future timestamp should yield no results"
        );
    }

    /// Verify that the date portion extraction from RFC 3339 timestamps works.
    #[test]
    fn regression_3078_changelog_date_extraction() {
        // RFC 3339 examples
        let timestamps = vec![
            ("2026-07-18T12:34:56+00:00", "2026-07-18"),
            ("2026-01-05T00:00:00Z", "2026-01-05"),
            ("2025-12-31T23:59:59+08:00", "2025-12-31"),
        ];

        for (ts, expected_date) in timestamps {
            let date = &ts[..10];
            assert_eq!(date, expected_date, "Failed to extract date from {ts}");
        }
    }

    /// Verify that notes in collections can be filtered by the storage API.
    #[test]
    fn regression_3078_changelog_collection_filter() {
        let (_temp, ctx) = setup();

        // Create two notes
        let note1 = NoteDocument {
            meta: NoteMeta {
                title: "Note in Work collection".to_string(),
                ..Default::default()
            },
            body: "Work note".to_string(),
            search_snippet: None,
            search_score: None,
        };
        let note2 = NoteDocument {
            meta: NoteMeta {
                title: "Note in Personal collection".to_string(),
                ..Default::default()
            },
            body: "Personal note".to_string(),
            search_snippet: None,
            search_score: None,
        };
        let saved1 = save_note_with_context(&ctx, note1).expect("save note1");
        let saved2 = save_note_with_context(&ctx, note2).expect("save note2");

        // Create collections and add notes
        use crate::storage::{add_note_to_collection_with_context, create_collection_with_context};
        let coll_work = create_collection_with_context(&ctx, "Work", "").expect("create Work");
        let coll_personal =
            create_collection_with_context(&ctx, "Personal", "").expect("create Personal");
        add_note_to_collection_with_context(&ctx, &saved1.meta.id, &coll_work.id)
            .expect("add note1 to Work");
        add_note_to_collection_with_context(&ctx, &saved2.meta.id, &coll_personal.id)
            .expect("add note2 to Personal");

        // List notes in Work collection
        let work_notes =
            crate::storage::list_notes_in_collection_with_context(&ctx, &coll_work.id, 100, 0)
                .expect("list Work notes");
        assert_eq!(work_notes.len(), 1, "Should find 1 note in Work collection");
        assert!(
            work_notes[0].title.contains("Work"),
            "Title should contain 'Work'"
        );

        // Verify that notes from Work collection have updated_at populated (for date grouping)
        assert!(
            !work_notes[0].updated_at.is_empty(),
            "updated_at should be populated"
        );
    }
}
