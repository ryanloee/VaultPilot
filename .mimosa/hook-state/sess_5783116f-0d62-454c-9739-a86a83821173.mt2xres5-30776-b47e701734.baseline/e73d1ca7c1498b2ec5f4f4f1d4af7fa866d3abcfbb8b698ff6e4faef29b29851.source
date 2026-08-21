//! Regression test for issue #3083: Changelog silently capped at 200 notes.
//!
//! Bug:      `handle_changelog` passed `limit: Some(2000)` to
//!           `search_notes_with_context`, but that function clamps the limit
//!           to `max(200)`. Vaults with >200 recently modified notes would
//!           get a silently truncated changelog.
//! Root cause: Central limit clamp `query.limit.unwrap_or(50).clamp(1, 200)`
//!             in `search_notes_with_context` prevents any caller from
//!             retrieving more than 200 notes at once.
//! Fix:      Use `list_all_notes_with_context` + in-memory `modified_after`
//!           filter in the non-collection changelog path, consistent with
//!           the collection path's pagination approach.

#[cfg(test)]
mod tests {
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        initialize_storage_with_context, list_all_notes_with_context,
        notes::save_note_with_context, StorageContext,
    };
    use std::fs;

    /// Helper: create a temp dir and initialised StorageContext.
    fn setup() -> (std::path::PathBuf, StorageContext) {
        let dir = std::env::temp_dir().join(format!(
            "vp-issue-3083-test-{}-{}",
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

    /// Verify that `list_all_notes_with_context` returns all notes without
    /// being limited to 200 — the fix for the truncated changelog.
    #[test]
    fn regression_3083_list_all_notes_returns_more_than_200() {
        let (_temp, ctx) = setup();

        // Arrange: save 250 notes (exceeds the old 200 clamp)
        for i in 0..250 {
            let note = NoteDocument {
                meta: NoteMeta {
                    title: format!("Note {i}"),
                    tags: vec!["regression-3083".to_string()],
                    ..Default::default()
                },
                body: format!("Body for note {i}"),
                search_snippet: None,
                search_score: None,
            };
            save_note_with_context(&ctx, note).expect("save note");
        }

        // Act: list all notes
        let all = list_all_notes_with_context(&ctx).expect("list all notes");

        // Assert: we should have all 250 notes, not capped at 200
        assert!(
            all.len() > 200,
            "Expected more than 200 notes, got {}",
            all.len()
        );
        assert_eq!(
            all.len(),
            250,
            "Expected exactly 250 notes, got {}",
            all.len()
        );
    }
}
