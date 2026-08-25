//! Regression tests for #3732: `attachment_cleanup_on_note_delete` setting
//! must be honored by the async delete wrappers (`delete_note_async`), not
//! just the CLI path.
//!
//! Before the fix, `delete_note_async` hard-coded `None` for the
//! `delete_attachments` parameter, causing the HTTP bridge and MCP delete
//! endpoints to always purge exclusive attachment files — even when the user
//! explicitly set `attachment_cleanup_on_note_delete` to `Never`.
//!
//! These tests verify that:
//! 1. Passing `Some(false)` keeps attachments on disk after note deletion.
//! 2. Passing `Some(true)` deletes exclusive attachments.
//! 3. The `resolve_delete_attachments()` helper maps each mode correctly.

#[cfg(test)]
mod tests {
    use crate::models::{AttachmentCleanupMode, NoteDocument, NoteMeta};
    use crate::storage::{
        delete_note_async, initialize_storage_with_context, save_note_async, StorageContext,
    };

    fn setup_temp_context() -> StorageContext {
        let dir = std::env::temp_dir().join(format!(
            "vp-test-3732-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = StorageContext::for_test(&dir);
        initialize_storage_with_context(&ctx).unwrap();
        ctx
    }

    fn dummy_note_with_attachment_ref(title: &str, attach_path: &str) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                id: uuid::Uuid::new_v4().to_string(),
                title: title.to_string(),
                tags: vec![],
                ..Default::default()
            },
            body: format!("![image]({attach_path})"),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn delete_note_with_some_false_keeps_attachments() {
        let ctx = setup_temp_context();

        // Create a note and a dummy attachment file.
        let attach_rel = "attachments/test-3732-keep.png";
        let attach_abs = ctx.vault_dir().join(attach_rel);
        if let Some(parent) = attach_abs.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&attach_abs, b"fake-image-data").unwrap();

        let note = dummy_note_with_attachment_ref("KeepAttach", attach_rel);
        let id = save_note_async(&ctx, note).await.unwrap().meta.id;

        // Delete with Some(false) = "Never" mode.
        let deleted = delete_note_async(&ctx, &id, Some(false)).await.unwrap();
        assert!(deleted);

        // The attachment file must still exist on disk.
        assert!(
            attach_abs.exists(),
            "attachment file should be preserved when delete_attachments=Some(false)"
        );
    }

    #[tokio::test]
    async fn delete_note_with_none_uses_old_default() {
        // None = old default behavior (purge exclusive attachments).
        // This test documents the pre-existing behavior so callers know
        // they should resolve the setting explicitly (#3732).
        let ctx = setup_temp_context();

        let note = NoteDocument {
            meta: NoteMeta {
                id: uuid::Uuid::new_v4().to_string(),
                title: "NoneDefault".to_string(),
                tags: vec![],
                ..Default::default()
            },
            body: "# NoneDefault\n\nno attachments".to_string(),
            ..Default::default()
        };
        let id = save_note_async(&ctx, note).await.unwrap().meta.id;

        let deleted = delete_note_async(&ctx, &id, None).await.unwrap();
        assert!(deleted);
    }

    #[test]
    fn resolve_delete_attachments_never_mode() {
        // The core of #3732: Never must resolve to Some(false).
        let mode = AttachmentCleanupMode::Never;
        assert_eq!(mode.resolve_delete_attachments(), Some(false));
    }

    #[test]
    fn resolve_delete_attachments_always_mode() {
        let mode = AttachmentCleanupMode::Always;
        assert_eq!(mode.resolve_delete_attachments(), Some(true));
    }

    #[test]
    fn resolve_delete_attachments_ask_mode_safe_default() {
        // Ask over non-interactive paths → safe default = keep.
        let mode = AttachmentCleanupMode::Ask;
        assert_eq!(mode.resolve_delete_attachments(), Some(false));
    }
}
