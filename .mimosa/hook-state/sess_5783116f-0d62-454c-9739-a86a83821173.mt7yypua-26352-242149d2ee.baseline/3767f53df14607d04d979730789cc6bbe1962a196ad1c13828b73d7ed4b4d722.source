//! Regression test for #3762 — Kanban meta field bulk-update API.
//!
//! When the UI drags a card from one Kanban column to another, it calls
//! `bulk_update_meta_field_with_context` to update the note's `status`
//! (or `board`, `platform`, etc.) field.
//!
//! This test verifies:
//! 1. updating a status field on a single note works
//! 2. the skipped counter works when the field already holds the target value
//! 3. invalid field names are rejected
//! 4. board field update works

#[cfg(test)]
mod tests {
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        bulk_delete_notes_with_context, bulk_update_meta_field_with_context,
        initialize_storage_with_context, load_note_with_context, save_note_with_context,
        StorageContext,
    };
    use chrono::Utc;
    use std::path::PathBuf;

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-3762-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        initialize_storage_with_context(&ctx).expect("init storage");
        (temp, ctx)
    }

    #[test]
    fn test_bulk_update_meta_field_status_single_note() {
        let (_tmp_dir, ctx) = setup_temp_context();
        let id = "test_note_3762_status".to_string();

        // Create a test note with status "todo"
        let note = NoteDocument {
            meta: NoteMeta {
                id: id.clone(),
                title: "Test Note".to_string(),
                status: "todo".to_string(),
                ..Default::default()
            },
            body: "# Test\n".to_string(),
            ..Default::default()
        };
        save_note_with_context(&ctx, note).expect("save");

        // 1. Update status from "todo" → "done".
        let result =
            bulk_update_meta_field_with_context(&ctx, std::slice::from_ref(&id), "status", "done")
                .expect("bulk_update_meta_field");
        assert_eq!(result.affected, 1, "should affect 1 note");
        assert_eq!(result.skipped, 0);
        assert!(result.failures.is_empty());

        // Verify the note's status is now "done".
        let reloaded = load_note_with_context(&ctx, &id).expect("load");
        assert_eq!(reloaded.meta.status, "done");

        // 2. Update again with the same value → should be skipped.
        let result2 =
            bulk_update_meta_field_with_context(&ctx, std::slice::from_ref(&id), "status", "done")
                .expect("bulk_update_meta_field same value");
        assert_eq!(result2.affected, 0, "no-op should not affect");
        assert_eq!(result2.skipped, 1, "no-op should be skipped");

        // 3. Invalid field should error.
        let err = bulk_update_meta_field_with_context(
            &ctx,
            std::slice::from_ref(&id),
            "created_at",
            "now",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not supported"),
            "should reject invalid field: {}",
            err
        );

        // 4. Board field update.
        let result3 =
            bulk_update_meta_field_with_context(&ctx, std::slice::from_ref(&id), "board", "sprint")
                .expect("bulk_update_meta_field board");
        assert_eq!(result3.affected, 1);
        let reloaded3 = load_note_with_context(&ctx, &id).expect("load");
        assert_eq!(reloaded3.meta.board, "sprint");

        // Cleanup.
        bulk_delete_notes_with_context(&ctx, &[id], Some(false)).ok();
    }
}
