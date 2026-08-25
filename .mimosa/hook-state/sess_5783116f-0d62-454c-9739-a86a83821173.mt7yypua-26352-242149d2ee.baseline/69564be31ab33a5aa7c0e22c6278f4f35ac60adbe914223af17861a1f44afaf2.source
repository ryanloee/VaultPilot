//! Regression tests for #3620: mirror import overwrites existing note title
//! with filename (UUID) when mirror file lacks frontmatter title.
//!
//! Bug: `mirror_import_with_context` computed `title` with a filename-stem
//! fallback (typically a UUID).  In the update-existing branch this `title`
//! was always `Some(<uuid>)`, so the existing note's human-readable title was
//! silently clobbered.
//!
//! Fix: only the *explicit* frontmatter title may override an existing note's
//! title.  The filename-stem fallback is reserved for new note creation only.

#[cfg(test)]
mod tests {
    use crate::mirror::{self, compose_mirror_markdown};
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        initialize_storage_with_context, load_note_with_context, save_note_with_context,
        StorageContext,
    };
    use std::path::PathBuf;

    fn setup_test_context(label: &str) -> (StorageContext, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "vp_3620_{}_{}_{}",
            std::process::id(),
            label,
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let ctx = StorageContext::for_test(&dir);
        initialize_storage_with_context(&ctx).expect("init storage");
        (ctx, dir)
    }

    /// #3620 core scenario: import an anchor-only mirror file (no frontmatter
    /// title, file named `{uuid}.md`) → existing title must be preserved.
    #[test]
    fn anchor_only_file_preserves_existing_title() {
        let (ctx, _dir) = setup_test_context("preserve_title");
        let note_id = "870e8400-e29b-41d4-a716-446655440001".to_string();

        // Existing vault note with a human-readable title.
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

        // Mirror file named by UUID, contains different body + anchor, but
        // **no frontmatter title** — exactly the bug scenario.
        let mirror_dir =
            std::env::temp_dir().join(format!("vp_3620_preserve_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);
        let mirror_content = compose_mirror_markdown("Updated body.", &note_id);
        std::fs::write(mirror_dir.join(format!("{}.md", note_id)), &mirror_content).unwrap();

        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, false).unwrap();

        assert_eq!(result.updated, 1, "note body should be updated");

        let updated = load_note_with_context(&ctx, &note_id).unwrap();
        assert_eq!(
            updated.meta.title, "UpdateTest",
            "existing title must be preserved when mirror file has no frontmatter title"
        );
        assert_ne!(
            updated.body.trim(),
            "Old body.",
            "body should be updated from mirror content"
        );
    }

    /// When the mirror file **does** have a frontmatter title, the existing
    /// note's title should be overridden (this is the existing intended
    /// behaviour — ensures the fix didn't break it).
    #[test]
    fn frontmatter_title_still_overrides_existing() {
        let (ctx, _dir) = setup_test_context("override_title");
        let note_id = "910e8400-e29b-41d4-a716-446655440002".to_string();

        let original = NoteDocument {
            meta: NoteMeta {
                id: note_id.clone(),
                title: "OldTitle".to_string(),
                ..Default::default()
            },
            body: "Old body.".to_string(),
            ..Default::default()
        };
        save_note_with_context(&ctx, original).unwrap();

        let mirror_dir =
            std::env::temp_dir().join(format!("vp_3620_override_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);
        let mirror_content =
            compose_mirror_markdown("---\ntitle: NewTitle\n---\n\nNew body.", &note_id);
        std::fs::write(mirror_dir.join(format!("{}.md", note_id)), &mirror_content).unwrap();

        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, false).unwrap();

        assert_eq!(result.updated, 1);
        let updated = load_note_with_context(&ctx, &note_id).unwrap();
        assert_eq!(
            updated.meta.title, "NewTitle",
            "explicit frontmatter title should override existing"
        );
    }

    /// New note creation (anchor present, but note not found in vault) should
    /// still fall back to the filename stem for the title.
    #[test]
    fn new_note_uses_filename_stem_fallback() {
        let (ctx, _dir) = setup_test_context("new_note_fallback");
        let note_id = "a20e8400-e29b-41d4-a716-446655440003".to_string();

        let mirror_dir = std::env::temp_dir().join(format!("vp_3620_new_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);

        // Mirror file with anchor but no frontmatter — note doesn't exist yet.
        let mirror_content = compose_mirror_markdown("Brand new body.", &note_id);
        std::fs::write(mirror_dir.join(format!("{}.md", note_id)), &mirror_content).unwrap();

        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, false).unwrap();

        assert_eq!(result.imported, 1, "note should be created as new");

        let created = load_note_with_context(&ctx, &note_id).unwrap();
        assert_eq!(
            created.meta.title, note_id,
            "new note without frontmatter title should fall back to filename stem"
        );
    }

    /// force=true on an anchor-only file (no frontmatter title) should update
    /// the body but still **preserve** the existing title.
    #[test]
    fn force_flag_preserves_title_without_frontmatter() {
        let (ctx, _dir) = setup_test_context("force_preserve");
        let note_id = "b30e8400-e29b-41d4-a716-446655440004".to_string();

        let original = NoteDocument {
            meta: NoteMeta {
                id: note_id.clone(),
                title: "ForceTest".to_string(),
                ..Default::default()
            },
            body: "Some body.".to_string(),
            ..Default::default()
        };
        save_note_with_context(&ctx, original).unwrap();

        let mirror_dir = std::env::temp_dir().join(format!("vp_3620_force_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mirror_dir);
        let mirror_content = compose_mirror_markdown("Some body.", &note_id);
        std::fs::write(mirror_dir.join(format!("{}.md", note_id)), &mirror_content).unwrap();

        // Same content + force=true → update should happen, title preserved.
        let result = mirror::mirror_import_with_context(&ctx, &mirror_dir, true).unwrap();

        assert_eq!(result.updated, 1, "force should update identical content");
        let updated = load_note_with_context(&ctx, &note_id).unwrap();
        assert_eq!(
            updated.meta.title, "ForceTest",
            "force update must still preserve title without frontmatter"
        );
    }
}
