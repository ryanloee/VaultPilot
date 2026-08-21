//! Regression tests for enhancement #2936: deleting a note should let the
//! caller choose whether associated attachment files are also removed from disk.
//!
//! `delete_note_with_context` gained a `delete_attachments: Option<bool>` arg:
//!   * `None` / `Some(true)`  -> keep prior behavior (non-shared attachment
//!     files are physically removed)
//!   * `Some(false)`          -> delete **only** the note's `.md` file; every
//!     attachment file on disk is preserved (Obsidian "Never" cleanup mode)
//!
//! Reproduction intent:
//!   A note has an image attachment. Deleting with `Some(false)` must leave the
//!   attachment file on disk (so the user can keep the screenshot while dropping
//!   the note), whereas `Some(true)`/`None` must clean it up.

#[cfg(test)]
mod tests {
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        delete_note_with_context, initialize_storage_with_context, load_note_with_context,
        save_note_with_images_with_context, StorageContext,
    };
    use std::path::PathBuf;

    // A minimal 1x1 transparent PNG (valid image, decodable for perceptual hash).
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn note_doc(id: &str, title: &str) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                id: id.to_string(),
                title: title.to_string(),
                ..Default::default()
            },
            body: format!("# {title}\n\nbody of {id}"),
            ..Default::default()
        }
    }

    /// Save a note that references one image attachment, return the note id and
    /// the absolute path of the attachment file that was written into the vault.
    fn save_note_with_attachment(
        ctx: &StorageContext,
        id: &str,
        vault: &std::path::Path,
    ) -> (String, PathBuf) {
        let src_png = vault.join(format!("{id}_src.png"));
        std::fs::write(&src_png, PNG).unwrap();

        let saved = save_note_with_images_with_context(
            ctx,
            note_doc(id, &format!("Note {id}")),
            &[src_png.to_string_lossy().to_string()],
        )
        .expect("save note with image");

        // The attachment is copied into `<note_stem>-assets/` next to the note
        // file (which lives under `vault/YYYY/MM/`).
        let note_path = PathBuf::from(&saved.meta.path);
        let stem = note_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let assets_dir = note_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .join(format!("{stem}-assets"));
        let mut found = None;
        if let Ok(entries) = std::fs::read_dir(&assets_dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "png").unwrap_or(false) {
                    found = Some(p);
                    break;
                }
            }
        }
        (
            saved.meta.id,
            found.expect("attachment file written into vault"),
        )
    }

    #[test]
    fn regression_2936_delete_keeps_attachments_when_opt_out() {
        let vault = std::env::temp_dir().join(format!("vp_2936_keep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        std::fs::create_dir_all(&vault).unwrap();
        let ctx = StorageContext::for_test(&vault);
        initialize_storage_with_context(&ctx).expect("init storage");

        let (id, attachment_path) = save_note_with_attachment(&ctx, "keep", &vault);
        assert!(
            attachment_path.exists(),
            "attachment must exist before delete"
        );

        // Opt out of attachment deletion.
        let deleted = delete_note_with_context(&ctx, &id, Some(false)).expect("delete");
        assert!(deleted, "note should be reported deleted");

        // Note itself is gone...
        assert!(
            load_note_with_context(&ctx, &id).is_err(),
            "note .md should be removed"
        );
        // ...but the attachment file on disk is preserved.
        assert!(
            attachment_path.exists(),
            "attachment file must survive a delete with Some(false)"
        );

        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn regression_2936_delete_removes_attachments_by_default() {
        let vault = std::env::temp_dir().join(format!("vp_2936_clean_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        std::fs::create_dir_all(&vault).unwrap();
        let ctx = StorageContext::for_test(&vault);
        initialize_storage_with_context(&ctx).expect("init storage");

        let (id, attachment_path) = save_note_with_attachment(&ctx, "clean", &vault);
        assert!(
            attachment_path.exists(),
            "attachment must exist before delete"
        );

        // Default behavior (None) still cleans up attachment files.
        let deleted = delete_note_with_context(&ctx, &id, None).expect("delete");
        assert!(deleted, "note should be reported deleted");

        assert!(load_note_with_context(&ctx, &id).is_err(), "note gone");
        assert!(
            !attachment_path.exists(),
            "attachment file must be removed on default delete"
        );

        let _ = std::fs::remove_dir_all(&vault);
    }
}
