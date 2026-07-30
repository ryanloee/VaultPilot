//! Regression tests for #3605: vp mirror import command
//!
//! Tests the `mirror_import_with_context` function which imports `.md` mirror
//! files back into the vault. Covers importing new notes (no anchor), updating
//! existing notes by vaultpilot-note-id anchor, and skipping unchanged files.

#[cfg(test)]
mod tests {
    use crate::mirror::{self, compose_mirror_markdown};
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        initialize_storage_with_context, list_all_notes_with_context, load_note_with_context,
        save_note_with_context, StorageContext,
    };

    fn setup_test_context(label: &str) -> (StorageContext, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("vp_3605_{}_{}", std::process::id(), label));
        let _ = std::fs::create_dir_all(&dir);
        let ctx = StorageContext::for_test(&dir);
        initialize_storage_with_context(&ctx).expect("init storage");
        (ctx, dir)
    }

    #[test]
    fn import_new_files_creates_notes() {
        let (ctx, _dir) = setup_test_context("import_new");

        // Create a workspace temp dir for mirror files
        let mirror_dir =
            std::env::temp_dir().join(format!("vp_3605_mirror_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);

        std::fs::write(
            mirror_dir.join("hello.md"),
            "---\ntitle: Hello\n---\n\n# Hello World\nThis is a test note.",
        )
        .unwrap();
        std::fs::write(
            mirror_dir.join("no-frontmatter.md"),
            "# No frontmatter\nJust plain markdown.",
        )
        .unwrap();

        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, false).unwrap();

        assert_eq!(
            result.imported, 2,
            "both files should be imported as new notes"
        );
        assert_eq!(result.updated, 0);
        assert_eq!(result.skipped, 0);

        // Verify the notes exist in the vault.
        let all_notes = list_all_notes_with_context(&ctx).unwrap();
        assert_eq!(all_notes.len(), 2);
        let hello = all_notes.iter().find(|m| m.title == "Hello");
        assert!(hello.is_some(), "note with frontmatter title should exist");
    }

    #[test]
    fn import_with_anchor_updates_existing_note() {
        let (ctx, _dir) = setup_test_context("update_anchor");

        // Create an initial vault note
        let note_id = "550e8400-e29b-41d4-a716-446655440001".to_string();
        let original = NoteDocument {
            meta: NoteMeta {
                id: note_id.clone(),
                title: "Original".to_string(),
                ..Default::default()
            },
            body: "Original body.".to_string(),
            ..Default::default()
        };
        save_note_with_context(&ctx, original).unwrap();

        let composed =
            compose_mirror_markdown("---\ntitle: Updated\n---\n\nUpdated body.", &note_id);

        let mirror_dir =
            std::env::temp_dir().join(format!("vp_3605_mirror_up_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);
        std::fs::write(mirror_dir.join(format!("{}.md", note_id)), &composed).unwrap();

        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, false).unwrap();

        assert_eq!(result.imported, 0);
        assert_eq!(
            result.updated, 1,
            "note should be updated from anchor match"
        );
        assert_eq!(result.skipped, 0);

        // Verify the vault note was updated
        let updated = load_note_with_context(&ctx, &note_id).unwrap();
        assert_eq!(updated.meta.title, "Updated");
        assert_ne!(
            updated.body.trim(),
            "Original body.",
            "body should be updated from mirror content"
        );
    }

    #[test]
    fn unchanged_content_gets_skipped() {
        let (ctx, _dir) = setup_test_context("skip_unchanged");
        let note_id = "670e8400-e29b-41d4-a716-446655440003".to_string();

        let body = "Unchanged body.";
        let original = NoteDocument {
            meta: NoteMeta {
                id: note_id.clone(),
                title: "Same".to_string(),
                ..Default::default()
            },
            body: body.to_string(),
            ..Default::default()
        };
        // Save and reload to get the actual vault body (which may include
        // auto-generated summary sections from ensure_summary_section).
        let saved = save_note_with_context(&ctx, original).unwrap();
        let vault_body = saved.body.as_str();

        let mirror_content = compose_mirror_markdown(vault_body, &note_id);

        let mirror_dir = std::env::temp_dir().join(format!("vp_3605_skip_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);
        std::fs::write(mirror_dir.join(format!("{}.md", note_id)), &mirror_content).unwrap();

        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, false).unwrap();

        // #3607: identical content should now be skipped instead of updated.
        assert_eq!(result.imported, 0);
        assert_eq!(result.updated, 0, "identical content should not update");
        assert_eq!(result.skipped, 1, "identical content should be skipped");
    }

    #[test]
    fn force_flag_overrides_content_skip() {
        let (ctx, _dir) = setup_test_context("force_override");
        let note_id = "770e8400-e29b-41d4-a716-446655440004".to_string();

        let body = "Force override body.";
        let original = NoteDocument {
            meta: NoteMeta {
                id: note_id.clone(),
                title: "ForceTest".to_string(),
                ..Default::default()
            },
            body: body.to_string(),
            ..Default::default()
        };
        let saved = save_note_with_context(&ctx, original).unwrap();
        let vault_body = saved.body.as_str();

        let mirror_content = compose_mirror_markdown(vault_body, &note_id);

        let mirror_dir = std::env::temp_dir().join(format!("vp_3605_force_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);
        std::fs::write(mirror_dir.join(format!("{}.md", note_id)), &mirror_content).unwrap();

        // With force=true, even identical content should be updated.
        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, true).unwrap();

        assert_eq!(result.imported, 0);
        assert_eq!(
            result.updated, 1,
            "force=true should update even identical content"
        );
    }

    #[test]
    fn different_content_always_updates() {
        let (ctx, _dir) = setup_test_context("diff_content");
        let note_id = "870e8400-e29b-41d4-a716-446655440005".to_string();

        // Create a vault note with 'Old body.'
        let original = NoteDocument {
            meta: NoteMeta {
                id: note_id.clone(),
                title: "UpdateTest".to_string(),
                ..Default::default()
            },
            body: "Old body.".to_string(),
            ..Default::default()
        };
        save_note_with_context(&ctx, original).unwrap();

        // Mirror file has different content
        let mirror_content = compose_mirror_markdown("Updated body.", &note_id);
        let mirror_dir = std::env::temp_dir().join(format!("vp_3605_diff_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);
        std::fs::write(mirror_dir.join(format!("{}.md", note_id)), &mirror_content).unwrap();

        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, false).unwrap();

        assert_eq!(result.imported, 0);
        assert_eq!(result.updated, 1, "different content should update");
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn empty_mirror_dir_yields_noop() {
        let (ctx, _dir) = setup_test_context("empty_dir");
        let mirror_dir = std::env::temp_dir().join(format!("vp_3605_empty_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);

        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, false).unwrap();

        assert_eq!(result.imported, 0);
        assert_eq!(result.updated, 0);
        assert_eq!(result.skipped, 0);
    }
}
