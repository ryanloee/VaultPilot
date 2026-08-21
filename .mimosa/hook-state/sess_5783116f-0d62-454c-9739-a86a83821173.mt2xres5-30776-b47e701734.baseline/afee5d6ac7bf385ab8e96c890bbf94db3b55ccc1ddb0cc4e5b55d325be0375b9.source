//! Regression tests for enhancement #3541: image OCR full-text indexing.
//!
//! OCR text extracted from images is now indexed in a dedicated `image_text_fts`
//! FTS5 table, enabling efficient full-text search of text embedded in
//! screenshots and photos. The `set_attachment_ocr_text` function allows
//! external OCR tools to inject text for any attachment.
//!
//! Reproduction intent:
//!   1. A note has an image attachment.
//!   2. OCR text is set via `set_attachment_ocr_text`.
//!   3. Searching for a word that only exists in the OCR text (not in the
//!      note body or attachment filename) must return the note.
//!   4. A note saved without OCR text must not appear in image_text_fts.

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::models::{NoteDocument, NoteMeta, SearchQuery};
    use crate::storage::notes::{save_note_with_images_with_context, set_attachment_ocr_text};
    use crate::storage::{initialize_storage_with_context, search_notes_with_context};

    // A minimal 1x1 transparent PNG (valid image for attachment registration).
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    /// Unique temp directory under the system temp dir (no `tempfile` dep).
    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-ocr-{}-{}-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            uuid::Uuid::new_v4(),
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// Removes the temp directory on drop so tests don't leave artifacts behind.
    struct TempDirGuard(std::path::PathBuf);

    impl std::ops::Deref for TempDirGuard {
        type Target = std::path::Path;

        fn deref(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Create a temp directory + initialized StorageContext for testing.
    ///
    /// Returns `(guard, ctx)`. When destructured as `let (vault, ctx) = setup()`,
    /// locals drop in reverse declaration order: `ctx` (the connection pool
    /// holding open file handles into the dir) is released before the guard
    /// removes the dir — required on Windows, where deleting a locked file fails.
    fn setup() -> (TempDirGuard, crate::storage::StorageContext) {
        let dir = temp_dir();
        std::fs::create_dir_all(dir.join("notes")).expect("create notes dir");
        let ctx = crate::storage::StorageContext::for_test(&dir);
        initialize_storage_with_context(&ctx).expect("init storage");
        (TempDirGuard(dir), ctx)
    }

    fn note_doc(id: &str, title: &str, body: &str) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                id: id.to_string(),
                title: title.to_string(),
                ..Default::default()
            },
            body: body.to_string(),
            ..Default::default()
        }
    }

    /// Save a note with an image attachment and return (note_id, attachment_path).
    fn save_note_with_image(
        ctx: &crate::storage::StorageContext,
        id: &str,
        vault: &std::path::Path,
    ) -> (String, String) {
        let png_path = vault.join(format!("{id}.png"));
        std::fs::write(&png_path, PNG).unwrap();

        let saved = save_note_with_images_with_context(
            ctx,
            note_doc(
                id,
                &format!("Note {id}"),
                &format!("# Note {id}\n\nSee image"),
            ),
            &[png_path.to_string_lossy().to_string()],
        )
        .expect("save note with image");

        // Find the attachment path in the vault.
        let note_path = std::path::PathBuf::from(&saved.meta.path);
        let stem = note_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let assets_dir = note_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .join(format!("{stem}-assets"));
        let mut attachment_path = String::new();
        if let Ok(entries) = std::fs::read_dir(&assets_dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "png").unwrap_or(false) {
                    attachment_path = p.to_string_lossy().to_string();
                    break;
                }
            }
        }
        assert!(!attachment_path.is_empty(), "attachment not found in vault");
        (saved.meta.id, attachment_path)
    }

    #[test]
    fn ocr_text_search_finds_note_by_unique_word() {
        let (vault, ctx) = setup();
        let (note_id, attachment_path) = save_note_with_image(&ctx, "ocr-unique", &vault);

        // Inject OCR text with a very unique word not present anywhere else.
        let unique_word = "xylophone";
        set_attachment_ocr_text(&ctx, &attachment_path, unique_word).expect("set OCR text");

        // Search for the unique word — should find the note.
        let query = SearchQuery {
            text: unique_word.to_string(),
            limit: Some(10),
            ..Default::default()
        };
        let result = search_notes_with_context(&ctx, query).expect("search");
        assert!(
            result.notes.iter().any(|n| n.id == note_id),
            "search for OCR-only word '{}' should find the note; got {} results",
            unique_word,
            result.notes.len()
        );
    }

    #[test]
    fn ocr_text_search_finds_note_by_error_code() {
        let (vault, ctx) = setup();
        let (note_id, attachment_path) = save_note_with_image(&ctx, "ocr-error", &vault);

        // Simulate OCR of a screenshot containing an error message.
        let ocr_text = "Error 404 not found stack trace at line 42";
        set_attachment_ocr_text(&ctx, &attachment_path, ocr_text).expect("set OCR text");

        // Search for a term only in OCR text.
        let query = SearchQuery {
            text: "trace".to_string(),
            limit: Some(10),
            ..Default::default()
        };
        let result = search_notes_with_context(&ctx, query).expect("search");
        assert!(
            result.notes.iter().any(|n| n.id == note_id),
            "search for OCR text 'stacktrace' should find the note"
        );
    }

    #[test]
    fn note_without_ocr_text_not_found_by_ocr_only_word() {
        let (vault, ctx) = setup();
        let (note_id, _attachment_path) = save_note_with_image(&ctx, "ocr-none", &vault);

        // On non-Windows, extract_image_text returns empty → no OCR indexed.
        // Search for a word that doesn't appear in the note body.
        let query = SearchQuery {
            text: "xylophone".to_string(),
            limit: Some(10),
            ..Default::default()
        };
        let result = search_notes_with_context(&ctx, query).expect("search");
        assert!(
            !result.notes.iter().any(|n| n.id == note_id),
            "note without OCR text should not match OCR-only search term"
        );
    }

    #[test]
    fn update_ocr_text_changes_search_results() {
        let (vault, ctx) = setup();
        let (note_id, attachment_path) = save_note_with_image(&ctx, "ocr-update", &vault);

        // Initially set OCR text with word A.
        set_attachment_ocr_text(&ctx, &attachment_path, "alpha bravo charlie")
            .expect("set OCR text A");

        // Verify search for "alpha" finds it.
        let query = SearchQuery {
            text: "alpha".to_string(),
            limit: Some(10),
            ..Default::default()
        };
        let result = search_notes_with_context(&ctx, query).expect("search alpha");
        assert!(
            result.notes.iter().any(|n| n.id == note_id),
            "should find note with OCR text 'alpha'"
        );

        // Update OCR text to word B (replacing).
        set_attachment_ocr_text(&ctx, &attachment_path, "delta echo foxtrot")
            .expect("update OCR text B");

        // Search for old word should no longer find it.
        let query = SearchQuery {
            text: "alpha".to_string(),
            limit: Some(10),
            ..Default::default()
        };
        let result = search_notes_with_context(&ctx, query).expect("search alpha after update");
        assert!(
            !result.notes.iter().any(|n| n.id == note_id),
            "old OCR text should be replaced, not accumulated"
        );

        // Search for new word should find it.
        let query = SearchQuery {
            text: "delta".to_string(),
            limit: Some(10),
            ..Default::default()
        };
        let result = search_notes_with_context(&ctx, query).expect("search delta after update");
        assert!(
            result.notes.iter().any(|n| n.id == note_id),
            "updated OCR text 'delta' should be searchable"
        );
    }
}
