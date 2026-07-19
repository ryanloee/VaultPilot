//! Regression tests for enhancement #3104: bulk operations on multiple notes
//! (tag / move / delete).
//!
//! `vaultpilot_lib::storage` gained three new bulk functions:
//!   * [`bulk_delete_notes_with_context`]      — delete a list of notes
//!   * [`bulk_update_tags_with_context`]       — add/remove tags on a list of notes
//!   * [`bulk_move_notes_with_context`]        — move notes into a vault subdir
//!
//! Each function returns a [`BulkNoteOpResult`] that distinguishes
//! `affected` / `skipped` / `failures`, so a batch that hits a couple of
//! missing ids still completes the rest (file-manager style).

#[cfg(test)]
mod tests {
    use crate::models::{NoteDocument, NoteMeta, SearchQuery};
    use crate::storage::{
        bulk_delete_notes_with_context, bulk_move_notes_with_context,
        bulk_update_tags_with_context, initialize_storage_with_context, load_note_with_context,
        save_note_with_context, search_notes_with_context, StorageContext,
    };
    use chrono::Utc;
    use std::path::PathBuf;

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        // PID + timestamp + uuid-equivalent entropy ensures process-level
        // isolation even under `cargo test --workspace --all-targets` which
        // may spin up multiple targets in parallel that share the OS temp
        // dir (see VaultPilot flaky-test postmortem on Windows CI).
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-3104-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        initialize_storage_with_context(&ctx).expect("init storage");
        (temp, ctx)
    }

    fn note_doc(title: &str, tags: &[&str]) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                title: title.to_string(),
                tags: tags.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            body: format!("# {title}\n\nbody"),
            ..Default::default()
        }
    }

    /// Save N notes and return their ids.
    fn save_notes(ctx: &StorageContext, count: usize) -> Vec<String> {
        let mut ids = Vec::new();
        for i in 0..count {
            let saved = save_note_with_context(ctx, note_doc(&format!("Note #{i}"), &["seed"]))
                .expect("save");
            ids.push(saved.meta.id);
        }
        ids
    }

    // ────────────────────────────────────────────────────────────────────
    // bulk_delete_notes_with_context
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn bulk_delete_removes_all_listed_notes() {
        let (_temp, ctx) = setup_temp_context();
        let ids = save_notes(&ctx, 3);

        let result = bulk_delete_notes_with_context(&ctx, &ids, None).expect("bulk delete");

        assert_eq!(result.requested, 3);
        assert_eq!(result.affected, 3);
        assert_eq!(result.skipped, 0);
        assert!(result.failures.is_empty(), "{:?}", result.failures);

        for id in &ids {
            assert!(
                load_note_with_context(&ctx, id).is_err(),
                "note {id} should be gone"
            );
        }
    }

    #[test]
    fn bulk_delete_reports_missing_ids_as_failures_without_aborting() {
        let (_temp, ctx) = setup_temp_context();
        let ids = save_notes(&ctx, 1);

        let batch = [
            ids[0].clone(),
            "does-not-exist-1".to_string(),
            "does-not-exist-2".to_string(),
        ];
        let result = bulk_delete_notes_with_context(&ctx, &batch, None).expect("bulk delete");

        assert_eq!(result.requested, 3);
        assert_eq!(result.affected, 1);
        assert_eq!(result.failures.len(), 2);
        for f in &result.failures {
            assert!(
                f.reason.contains("not found"),
                "unexpected reason: {}",
                f.reason
            );
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // bulk_update_tags_with_context
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn bulk_update_tags_adds_and_removes() {
        let (_temp, ctx) = setup_temp_context();
        // Two notes: one with [seed, a], one with [seed, b]
        let n1 = save_note_with_context(&ctx, note_doc("N1", &["seed", "a"])).expect("save");
        let n2 = save_note_with_context(&ctx, note_doc("N2", &["seed", "b"])).expect("save");
        let ids = vec![n1.meta.id.clone(), n2.meta.id.clone()];

        let result = bulk_update_tags_with_context(
            &ctx,
            &ids,
            &["triaged".to_string(), "review".to_string()],
            &["seed".to_string()],
        )
        .expect("bulk tag");

        assert_eq!(result.requested, 2);
        assert_eq!(result.affected, 2);
        assert_eq!(result.skipped, 0);
        assert!(result.failures.is_empty(), "{:?}", result.failures);

        let l1 = load_note_with_context(&ctx, &n1.meta.id).expect("load n1");
        assert!(
            !l1.meta.tags.contains(&"seed".to_string()),
            "seed should be removed"
        );
        assert!(l1.meta.tags.contains(&"a".to_string()), "a should remain");
        assert!(l1.meta.tags.contains(&"triaged".to_string()));
        assert!(l1.meta.tags.contains(&"review".to_string()));

        let l2 = load_note_with_context(&ctx, &n2.meta.id).expect("load n2");
        assert!(l2.meta.tags.contains(&"b".to_string()));
        assert!(l2.meta.tags.contains(&"triaged".to_string()));
    }

    #[test]
    fn bulk_update_tags_skips_notes_with_unchanged_tag_set() {
        let (_temp, ctx) = setup_temp_context();
        // n1 already has the tag → adding it is a no-op (skipped).
        let n1 = save_note_with_context(&ctx, note_doc("N1", &["foo"])).expect("save");
        // n2 lacks the tag → affected.
        let n2 = save_note_with_context(&ctx, note_doc("N2", &[])).expect("save");
        let ids = vec![n1.meta.id.clone(), n2.meta.id.clone()];

        let result =
            bulk_update_tags_with_context(&ctx, &ids, &["foo".to_string()], &[]).expect("bulk tag");

        assert_eq!(result.affected, 1);
        assert_eq!(result.skipped, 1);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn bulk_update_tags_removal_is_case_insensitive() {
        let (_temp, ctx) = setup_temp_context();
        let n = save_note_with_context(&ctx, note_doc("N1", &["FooBar"])).expect("save");

        let result = bulk_update_tags_with_context(
            &ctx,
            std::slice::from_ref(&n.meta.id),
            &[],
            &["foobar".to_string()],
        )
        .expect("bulk tag");

        assert_eq!(result.affected, 1);
        let l = load_note_with_context(&ctx, &n.meta.id).expect("load");
        assert!(
            !l.meta.tags.iter().any(|t| t.eq_ignore_ascii_case("foobar")),
            "tag should be gone (case-insensitive match)"
        );
    }

    #[test]
    fn bulk_update_tags_comma_separated_input_is_split() {
        let (_temp, ctx) = setup_temp_context();
        let n = save_note_with_context(&ctx, note_doc("N1", &[])).expect("save");

        let result = bulk_update_tags_with_context(
            &ctx,
            std::slice::from_ref(&n.meta.id),
            &["a, b ,c".to_string()],
            &[],
        )
        .expect("bulk tag");

        assert_eq!(result.affected, 1);
        let l = load_note_with_context(&ctx, &n.meta.id).expect("load");
        // Three distinct tags after split/trim
        assert!(l.meta.tags.contains(&"a".to_string()));
        assert!(l.meta.tags.contains(&"b".to_string()));
        assert!(l.meta.tags.contains(&"c".to_string()));
    }

    // ────────────────────────────────────────────────────────────────────
    // bulk_move_notes_with_context
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn bulk_move_notes_into_subdirectory() {
        let (_temp, ctx) = setup_temp_context();
        let ids = save_notes(&ctx, 2);

        let result = bulk_move_notes_with_context(&ctx, &ids, "archive/2026").expect("bulk move");

        assert_eq!(result.requested, 2);
        assert_eq!(result.affected, 2);
        assert!(result.failures.is_empty(), "{:?}", result.failures);

        // Each note's stored path should now live under archive/2026
        for id in &ids {
            let l = load_note_with_context(&ctx, id).expect("load after move");
            let normalized = l.meta.path.replace('\\', "/");
            assert!(
                normalized.contains("archive/2026"),
                "path '{}' should contain 'archive/2026'",
                normalized
            );
        }

        // And search should still find them at the new path.
        let sr = search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "Note".to_string(),
                limit: Some(10),
                ..Default::default()
            },
        )
        .expect("search");
        assert_eq!(sr.notes.len(), 2, "both notes should still be indexed");
    }

    #[test]
    fn bulk_move_reports_failure_for_missing_id() {
        let (_temp, ctx) = setup_temp_context();
        let valid_ids = save_notes(&ctx, 1);
        let batch = [valid_ids[0].clone(), "ghost-id-3104".to_string()];

        let result = bulk_move_notes_with_context(&ctx, &batch, "moved").expect("bulk move");

        assert_eq!(result.affected, 1);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].id, "ghost-id-3104");
    }

    #[test]
    fn bulk_move_rejects_path_escape_outside_vault() {
        let (_temp, ctx) = setup_temp_context();
        let ids = save_notes(&ctx, 1);

        // ../ should be rejected by normalize_tool_path confinement
        let err = bulk_move_notes_with_context(&ctx, &ids, "../../etc").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("invalid target directory") || msg.contains("vault"),
            "expected confinement error, got: {msg}"
        );
    }

    #[test]
    fn bulk_move_skips_note_already_in_target_dir() {
        let (_temp, ctx) = setup_temp_context();
        let ids = save_notes(&ctx, 1);

        // First move into `target/`: affects 1
        let r1 = bulk_move_notes_with_context(&ctx, &ids, "target").expect("first move");
        assert_eq!(r1.affected, 1);

        // Second move into the SAME dir: should skip (file is already there).
        let r2 = bulk_move_notes_with_context(&ctx, &ids, "target").expect("second move");
        assert_eq!(r2.affected, 0);
        assert_eq!(r2.skipped, 1, "expected skip, got {:?}", r2);
    }

    #[test]
    fn bulk_move_refuses_to_overwrite_existing_file() {
        let (_temp, ctx) = setup_temp_context();
        // Two notes with identical filenames in different dirs — we move
        // both into the same target, which should fail for the second one
        // rather than silently overwrite.
        let n1 = save_note_with_context(&ctx, note_doc("Same Title", &[])).expect("save");
        let n2 = save_note_with_context(&ctx, note_doc("Same Title", &[])).expect("save");
        // Both got paths derived from title — but the storage layer appends
        // the note id, so they shouldn't actually collide. To force a real
        // collision, we craft a duplicate by copying n1's file to n2's id.
        // Simpler: move n1 into target/ first, then craft a second file
        // manually at target/<n1_filename> and try to move n2.
        let _ = bulk_move_notes_with_context(&ctx, std::slice::from_ref(&n1.meta.id), "target")
            .expect("move n1");
        let n1_after = load_note_with_context(&ctx, &n1.meta.id).expect("load n1");
        let n1_path = std::path::PathBuf::from(&n1_after.meta.path);

        // Create a conflicting file at the same target path as n2 would
        // land. We need n2's filename to match n1's.
        // The simplest reliable way: write a dummy file with the same name
        // as n1 in target/, then move n2's filename to match by renaming
        // n2's file to match n1's filename before invoking bulk_move.
        let n2_after = load_note_with_context(&ctx, &n2.meta.id).expect("load n2");
        let n2_path = std::path::PathBuf::from(&n2_after.meta.path);
        let n2_new_path = n2_path.with_file_name(n1_path.file_name().unwrap());
        // Can't do this if it collides on the same dir, so skip if same.
        if n2_new_path != n2_path {
            let _ = std::fs::rename(&n2_path, &n2_new_path);
        }

        // Now try to move n2 into target/ — it would land on n1's file.
        let r = bulk_move_notes_with_context(&ctx, std::slice::from_ref(&n2.meta.id), "target")
            .expect("call");
        assert!(
            !r.failures.is_empty(),
            "expected at least one failure due to existing file, got {r:?}"
        );
    }
}
