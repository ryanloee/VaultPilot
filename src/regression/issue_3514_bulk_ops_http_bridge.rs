//! Regression tests for #3514: async wrappers for bulk note operations
//! (`bulk_delete_notes_async`, `bulk_move_notes_async`,
//! `bulk_update_tags_async`).
//!
//! The HTTP handlers in `http_bridge.rs` are thin wrappers around these
//! async functions.  The sync `_with_context` functions are covered by
//! `issue_3104_*` tests; these tests validate the async spawn_blocking
//! wrappers produce correct `BulkNoteOpResult` shapes.

#[cfg(test)]
mod tests {
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        bulk_delete_notes_async, bulk_move_notes_async, bulk_update_tags_async, delete_note_async,
        initialize_storage_with_context, save_note_async, StorageContext,
    };
    use chrono::Utc;

    fn setup_temp_context() -> StorageContext {
        let dir = std::env::temp_dir().join(format!(
            "vp-test-3514-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = StorageContext::for_test(&dir);
        initialize_storage_with_context(&ctx).unwrap();
        ctx
    }

    fn dummy_note(title: &str) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                id: uuid::Uuid::new_v4().to_string(),
                title: title.to_string(),
                tags: vec![],
                ..Default::default()
            },
            body: format!(
                "# {title}

body"
            ),
            ..Default::default()
        }
    }

    async fn create(ctx: &StorageContext, title: &str) -> String {
        let note = dummy_note(title);
        save_note_async(ctx, note).await.unwrap().meta.id
    }

    #[tokio::test]
    async fn delete_note_async_removes_single_note() {
        let ctx = setup_temp_context();
        let id = create(&ctx, "ToDelete").await;

        let deleted = delete_note_async(&ctx, &id).await.unwrap();
        assert!(deleted);

        let deleted2 = delete_note_async(&ctx, &id).await.unwrap();
        assert!(!deleted2, "second delete should return false");
    }

    #[tokio::test]
    async fn bulk_delete_async_deletes_multiple() {
        let ctx = setup_temp_context();
        let id1 = create(&ctx, "Bulk1").await;
        let id2 = create(&ctx, "Bulk2").await;
        let id3 = create(&ctx, "Bulk3").await;

        let result = bulk_delete_notes_async(&ctx, vec![id1.clone(), id2.clone()], None)
            .await
            .unwrap();

        assert_eq!(result.requested, 2);
        assert_eq!(result.affected, 2);
        assert!(result.failures.is_empty());

        // id3 still exists.
        let alive = delete_note_async(&ctx, &id3).await.unwrap();
        assert!(alive);
    }

    #[tokio::test]
    async fn bulk_delete_async_reports_missing() {
        let ctx = setup_temp_context();
        let id1 = create(&ctx, "Exists").await;

        let result =
            bulk_delete_notes_async(&ctx, vec![id1.clone(), "nonexistent".to_string()], None)
                .await
                .unwrap();

        assert_eq!(result.requested, 2);
        assert_eq!(result.affected, 1);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].id, "nonexistent");
    }

    #[tokio::test]
    async fn bulk_move_async_moves_to_subdir() {
        let ctx = setup_temp_context();
        let id1 = create(&ctx, "MoveA").await;
        let id2 = create(&ctx, "MoveB").await;

        let result = bulk_move_notes_async(&ctx, vec![id1, id2], "archive".to_string())
            .await
            .unwrap();

        assert_eq!(result.requested, 2);
        assert_eq!(result.affected, 2);
        assert!(result.failures.is_empty());
    }

    #[tokio::test]
    async fn bulk_tags_async_adds_tags() {
        let ctx = setup_temp_context();
        let id1 = create(&ctx, "Tag1").await;
        let id2 = create(&ctx, "Tag2").await;

        let result = bulk_update_tags_async(
            &ctx,
            vec![id1, id2],
            vec!["important".to_string(), "review".to_string()],
            vec![],
        )
        .await
        .unwrap();

        assert_eq!(result.requested, 2);
        assert_eq!(result.affected, 2);
        assert!(result.failures.is_empty());
    }

    #[tokio::test]
    async fn bulk_tags_async_removes_tags() {
        let ctx = setup_temp_context();
        let id1 = create(&ctx, "RemTag1").await;
        let id2 = create(&ctx, "RemTag2").await;

        // Add tags first.
        bulk_update_tags_async(
            &ctx,
            vec![id1.clone(), id2.clone()],
            vec!["temp".to_string(), "keep".to_string()],
            vec![],
        )
        .await
        .unwrap();

        // Remove "temp".
        let result = bulk_update_tags_async(&ctx, vec![id1, id2], vec![], vec!["temp".to_string()])
            .await
            .unwrap();

        assert_eq!(result.requested, 2);
        assert_eq!(result.affected, 2);
        assert!(result.failures.is_empty());
    }
}
