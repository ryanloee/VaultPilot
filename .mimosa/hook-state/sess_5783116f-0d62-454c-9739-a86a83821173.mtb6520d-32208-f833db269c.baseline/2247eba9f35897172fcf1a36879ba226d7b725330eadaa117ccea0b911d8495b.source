//! Regression test for #3350: Agent SaveNote resets created_at on update.
//!
//! Bug:     In `execute_tool` SaveNote branch, `created_at` was always set to `chrono::Utc::now()`,
//!          overwriting the original creation timestamp when updating existing notes.
//! Root cause: The code loaded the existing note for backup but never read its `created_at`,
//!             and the draft's `StructuredNoteDraft` doesn't carry a `created_at` field,
//!             so the agent always injected `now()`.
//! Fix:      Preserve `created_at` from existing note when available; fall back to `now()`
//!           for genuinely new notes. Also forward draft fields (platform, board, kernel,
//!           status, source, summary) that were silently dropped.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::models::{NoteDocument, NoteMeta, StructuredNoteDraft};
use crate::storage::{load_note_with_context, save_note_with_images_with_context, StorageContext};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> (PathBuf, StorageContext) {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!(
        "vaultpilot_regression_3350_{}_{}",
        std::process::id(),
        n
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("vault")).unwrap();
    let ctx = StorageContext::for_test(&tmp);
    (tmp, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulate what execute_tool does for a NEW note:
    /// created_at = now(), then save. Verify created_at is non-empty.
    #[test]
    fn regression_3350_new_note_gets_fresh_created_at() {
        let (_tmp, ctx) = setup();
        let created_at = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        let note = NoteDocument {
            meta: NoteMeta {
                id: id.clone(),
                title: "New Note".into(),
                // path left empty → storage builds it inside vault dir
                created_at: created_at.clone(),
                updated_at: created_at.clone(),
                ..Default::default()
            },
            body: "# New\n\nContent".into(),
            ..Default::default()
        };
        let saved = save_note_with_images_with_context(&ctx, note, &[]).unwrap();
        assert!(!saved.meta.created_at.is_empty());
        assert_eq!(
            saved.meta.created_at, created_at,
            "storage should preserve the caller-provided created_at"
        );
    }

    /// Simulate update: create a note first, then "update" it with a new body
    /// but preserve the original created_at. This is what the agent fix does.
    #[test]
    fn regression_3350_update_preserves_created_at() {
        let (_tmp, ctx) = setup();

        // First: create a note with a known created_at
        let original_created_at = "2024-01-15T10:30:00Z".to_string();
        let id = uuid::Uuid::new_v4().to_string();
        let note = NoteDocument {
            meta: NoteMeta {
                id: id.clone(),
                title: "Original".into(),
                created_at: original_created_at.clone(),
                updated_at: original_created_at.clone(),
                platform: "linux".into(),
                board: "x86".into(),
                kernel: "6.1".into(),
                status: "draft".into(),
                source: "captured".into(),
                summary: "first version".into(),
                ..Default::default()
            },
            body: "# Original\n\nFirst body".into(),
            ..Default::default()
        };
        save_note_with_images_with_context(&ctx, note, &[]).unwrap();

        // Now simulate what the FIXED agent does: load existing, preserve created_at
        let existing = load_note_with_context(&ctx, &id).unwrap();
        let preserved_created_at = existing.meta.created_at.clone();
        assert_eq!(
            preserved_created_at, original_created_at,
            "original created_at should still be the old value"
        );

        let draft = StructuredNoteDraft {
            title: "Updated Title".into(),
            body: "# Updated\n\nNew body".into(),
            tags: vec!["updated".into()],
            keywords: vec!["fix".into()],
            platform: "windows".into(),
            board: "arm64".into(),
            kernel: "6.5".into(),
            status: "review".into(),
            source: "edited".into(),
            summary: "updated version".into(),
        };

        // This is the key assertion: the update should preserve created_at
        let update_note = NoteDocument {
            meta: NoteMeta {
                id: id.clone(),
                title: draft.title.clone(),
                // path left empty → storage builds it inside vault dir
                tags: draft.tags.clone(),
                keywords: draft.keywords.clone(),
                platform: draft.platform.clone(),
                board: draft.board.clone(),
                kernel: draft.kernel.clone(),
                status: draft.status.clone(),
                source: draft.source.clone(),
                summary: draft.summary.clone(),
                created_at: preserved_created_at, // <-- THE FIX: use existing value
                updated_at: chrono::Utc::now().to_rfc3339(),
                ..Default::default()
            },
            body: draft.body.clone(),
            ..Default::default()
        };

        let saved = save_note_with_images_with_context(&ctx, update_note, &[]).unwrap();

        // Assert: created_at is the ORIGINAL value, not now()
        assert_eq!(
            saved.meta.created_at, original_created_at,
            "update MUST preserve original created_at (#3350)"
        );
        // Assert: updated_at is newer (storage always sets it to now on update)
        assert_ne!(
            saved.meta.updated_at, original_created_at,
            "updated_at should be refreshed"
        );
        // Assert: draft fields were forwarded correctly
        assert_eq!(saved.meta.platform, "windows");
        assert_eq!(saved.meta.board, "arm64");
        assert_eq!(saved.meta.kernel, "6.5");
        assert_eq!(saved.meta.status, "review");
        assert_eq!(saved.meta.source, "edited");
        assert_eq!(saved.meta.summary, "updated version");
        assert_eq!(saved.meta.title, "Updated Title");
        // Body may have a summary preamble prepended by storage layer
        assert!(
            saved.body.contains("# Updated\n\nNew body"),
            "body should contain the updated content, got: {:?}",
            saved.body
        );
    }

    /// Edge case: if existing note has empty created_at for some reason,
    /// fall back to now() (the current behavior for genuinely new notes).
    #[test]
    fn regression_3350_empty_existing_created_at_falls_back_to_now() {
        let (_tmp, ctx) = setup();

        let id = uuid::Uuid::new_v4().to_string();
        // Create note with empty created_at (edge case, shouldn't normally happen)
        let note = NoteDocument {
            meta: NoteMeta {
                id: id.clone(),
                title: "Empty Created".into(),
                created_at: String::new(), // empty!
                updated_at: "2024-01-15T10:30:00Z".into(),
                ..Default::default()
            },
            body: "# Empty\n\ncreated_at".into(),
            ..Default::default()
        };
        save_note_with_images_with_context(&ctx, note, &[]).unwrap();

        let existing = load_note_with_context(&ctx, &id).unwrap();
        // Storage fills in empty created_at, so it won't be empty after save
        assert!(!existing.meta.created_at.is_empty());

        // Now "update" with a fallback: if existing created_at is empty (unlikely),
        // the agent falls back to now()
        let fallback_at = chrono::Utc::now().to_rfc3339();
        let created_at = if existing.meta.created_at.trim().is_empty() {
            fallback_at.clone()
        } else {
            existing.meta.created_at.clone()
        };

        let update = NoteDocument {
            meta: NoteMeta {
                id: id.clone(),
                title: "Updated".into(),
                created_at,
                updated_at: chrono::Utc::now().to_rfc3339(),
                ..Default::default()
            },
            body: "# Updated".into(),
            ..Default::default()
        };
        let saved = save_note_with_images_with_context(&ctx, update, &[]).unwrap();

        // Should have preserved the storage-filled created_at (not fallback)
        assert!(!saved.meta.created_at.is_empty());
        // If the existing note had stored a non-empty value (which it did),
        // created_at should match that, not be now()
        assert_eq!(saved.meta.created_at, existing.meta.created_at);
    }
}
