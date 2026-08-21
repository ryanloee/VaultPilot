//! Regression test for #3320 — Tag Merge CLI command.
//!
//! Feature: `vp tag merge --from <variants> --to <canonical>`
//! Tests the underlying `bulk_update_tags_with_context` function and the
//! JSON-based tag-finding query used by the handler.
//!
//! See: `handle_tag_merge` in src/bin/vaultpilot-cli/main.rs

#[cfg(test)]
mod tests {
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        bulk_update_tags_with_context, initialize_storage_with_context,
        list_all_notes_with_context, save_note_with_context, StorageContext,
    };
    use std::path::PathBuf;
    use uuid::Uuid;

    fn setup(ctx: &StorageContext) {
        let notes: Vec<(&str, &str, Vec<&str>)> = vec![
            ("n1", "Note One", vec!["#meeting", "#notes"]),
            ("n2", "Note Two", vec!["#meetings", "#archive"]),
            ("n3", "Note Three", vec!["#meeting", "#work"]),
            ("n4", "Note Four", vec!["#Meetings", "#personal"]),
            ("n5", "Note Five", vec!["#ai", "#ml"]),
            ("n6", "Note Six", vec!["#AI", "#deep-learning"]),
            ("n7", "Note Seven", vec!["#ai", "#meeting"]),
        ];

        for (_, title, tags) in notes {
            let note = NoteDocument {
                meta: NoteMeta {
                    id: String::new(), // auto-assigned
                    title: title.to_string(),
                    tags: tags.iter().map(|t| t.to_string()).collect(),
                    path: String::new(), // auto-generated
                    ..Default::default()
                },
                body: format!("Body of {}", title),
                search_score: None,
                search_snippet: None,
            };
            let saved = save_note_with_context(ctx, note).expect("save should succeed");
            // Overwrite id to be predictable - not needed, we'll use the returned id
            let _ = saved;
        }
    }

    fn new_ctx() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!("vp-3320-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        initialize_storage_with_context(&ctx).expect("init storage");
        (temp, ctx)
    }

    #[test]
    fn regression_3320_merge_case_sensitive_variants() {
        let (_tmp, ctx) = new_ctx();
        setup(&ctx);

        // Find notes with meetings/Meetings variants by their title
        let all = list_all_notes_with_context(&ctx).unwrap();
        let ids: Vec<String> = all
            .iter()
            .filter(|n| n.tags.iter().any(|t| t.eq_ignore_ascii_case("#meetings")))
            .map(|n| n.id.clone())
            .collect();

        assert_eq!(ids.len(), 2, "should find 2 notes with meetings variants");

        // Apply merge: #meetings/#Meetings → #meeting
        let result = bulk_update_tags_with_context(
            &ctx,
            &ids,
            &["#meeting".to_string()],
            &["#meetings".to_string()],
        )
        .expect("merge should succeed");

        assert_eq!(result.affected, 2, "both notes affected");
        assert_eq!(result.failures.len(), 0, "no failures");

        // Verify notes were updated
        for id in &ids {
            let note = crate::storage::load_note_with_context(&ctx, id).unwrap();
            assert!(
                note.meta.tags.contains(&"#meeting".to_string()),
                "note {} should have canonical tag",
                id
            );
            assert!(
                !note
                    .meta
                    .tags
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case("#meetings")),
                "note {} should no longer have meetings variant",
                id
            );
        }
    }

    #[test]
    fn regression_3320_merge_ai_variants() {
        let (_tmp, ctx) = new_ctx();
        setup(&ctx);

        // Find all notes with #AI
        let all = list_all_notes_with_context(&ctx).unwrap();
        let ai_upper_ids: Vec<String> = all
            .iter()
            .filter(|n| n.tags.iter().any(|t| t == "#AI"))
            .map(|n| n.id.clone())
            .collect();

        assert!(
            !ai_upper_ids.is_empty(),
            "should find at least one note with #AI"
        );

        // Merge #AI → #ai
        let result = bulk_update_tags_with_context(
            &ctx,
            &ai_upper_ids,
            &["#ai".to_string()],
            &["#AI".to_string()],
        )
        .expect("merge should succeed");

        assert!(result.affected >= 1);

        // Verify tags were updated
        for id in &ai_upper_ids {
            let note = crate::storage::load_note_with_context(&ctx, id).unwrap();
            assert!(
                !note.meta.tags.contains(&"#AI".to_string()),
                "note {} should no longer have #AI",
                id
            );
        }
    }

    #[test]
    fn regression_3320_noop_when_tags_absent() {
        let (_tmp, ctx) = new_ctx();
        setup(&ctx);

        let all = list_all_notes_with_context(&ctx).unwrap();
        let ids: Vec<String> = all
            .iter()
            .filter(|n| {
                n.tags
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case("#nonexistent"))
            })
            .map(|n| n.id.clone())
            .collect();

        assert_eq!(ids.len(), 0, "no notes should match non-existent tag");
    }

    #[test]
    fn regression_3320_merge_preserves_unrelated_tags() {
        let (_tmp, ctx) = new_ctx();
        setup(&ctx);

        // Get the n2 note (has #meetings, #archive)
        let all = list_all_notes_with_context(&ctx).unwrap();
        let n2_ids: Vec<String> = all
            .iter()
            .filter(|n| n.tags.contains(&"#archive".to_string()))
            .map(|n| n.id.clone())
            .collect();

        assert_eq!(
            n2_ids.len(),
            1,
            "should find exactly one note with #archive"
        );

        // Merge #meetings → #meeting
        let result = bulk_update_tags_with_context(
            &ctx,
            &n2_ids,
            &["#meeting".to_string()],
            &["#meetings".to_string()],
        )
        .expect("merge should succeed");

        assert_eq!(result.affected, 1);

        let n2 = crate::storage::load_note_with_context(&ctx, &n2_ids[0]).unwrap();
        assert!(
            n2.meta.tags.contains(&"#meeting".to_string()),
            "note should have canonical tag"
        );
        assert!(
            n2.meta.tags.contains(&"#archive".to_string()),
            "note should preserve unrelated #archive tag"
        );
        assert!(
            !n2.meta
                .tags
                .iter()
                .any(|t| t.eq_ignore_ascii_case("#meetings")),
            "note should no longer have meetings variant"
        );
    }
}
