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
    fn bulk_move_does_not_flatten_nested_subdir_3134() {
        let (temp, ctx) = setup_temp_context();
        // Save a note, then manually relocate it into a *nested* subdir of
        // the eventual target so that it is already "inside" the target tree.
        let ids = save_notes(&ctx, 1);
        let note = load_note_with_context(&ctx, &ids[0]).expect("load seed");
        let filename = std::path::Path::new(&note.meta.path)
            .file_name()
            .unwrap()
            .to_os_string();
        // Build the nested path under the vault root: <vault>/archive/2026/<file>
        let nested_path = ctx.vault_dir().join("archive").join("2026").join(&filename);
        std::fs::create_dir_all(nested_path.parent().unwrap()).expect("mkdir nested");
        std::fs::rename(&note.meta.path, &nested_path).expect("relocate into nested");
        // Update the indexed path directly (mirrors what the storage layer
        // would record after the file was moved by an external tool).
        let db_path = temp.join("knowledge-index.sqlite");
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        conn.execute(
            "UPDATE notes SET path = ?1 WHERE id = ?2",
            rusqlite::params![nested_path.to_string_lossy().replace('\\', "/"), &ids[0]],
        )
        .expect("update indexed path");
        drop(conn);

        // Target is the parent `archive/`. The note already lives in
        // `archive/2026/`, so it must be Skipped — NOT moved up to the
        // `archive/` top level (which would flatten the user's hierarchy).
        let result =
            bulk_move_notes_with_context(&ctx, &ids, "archive").expect("bulk move archive");

        assert_eq!(result.affected, 0, "nested note must not be relocated");
        assert_eq!(result.skipped, 1, "expected skip, got {:?}", result);
        assert!(result.failures.is_empty(), "{:?}", result.failures);

        // Confirm the path is still inside archive/2026, not archive/.
        let after = load_note_with_context(&ctx, &ids[0]).expect("load after move");
        let normalized = after.meta.path.replace('\\', "/");
        assert!(
            normalized.contains("archive/2026"),
            "note should remain nested, path was '{}'",
            normalized
        );
        assert!(
            !normalized.ends_with("/archive/note.md")
                && !normalized.ends_with("\\archive\\note.md"),
            "note must not be flattened to archive/ top level: '{}'",
            normalized
        );
    }

    #[test]
    fn bulk_delete_with_delete_attachments_flag_3135() {
        let (temp, ctx) = setup_temp_context();

        // Helper: create a note with one real physical attachment file registered
        // in the DB, and return (note_id, attachment_file_path).
        fn make_note_with_attachment(
            ctx: &StorageContext,
            temp: &std::path::Path,
            title: &str,
            att_id: &str,
        ) -> (String, std::path::PathBuf) {
            let note = save_note_with_context(ctx, note_doc(title, &["seed"])).expect("save note");
            let note_id = note.meta.id.clone();
            let attach_path = ctx
                .vault_dir()
                .join(format!("{}-assets", note.meta.id))
                .join("diagram.png");
            std::fs::create_dir_all(attach_path.parent().unwrap()).expect("mkdir assets");
            std::fs::write(&attach_path, b"fake-png-bytes").expect("write attachment file");
            let attach_str = attach_path.to_string_lossy().replace('\\', "/");
            let db_path = temp.join("knowledge-index.sqlite");
            let conn = rusqlite::Connection::open(&db_path).expect("open db");
            conn.execute(
                "INSERT INTO attachments (id, note_id, path, file_name, stem, ocr_text, semantic_vector, perceptual_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, '', '', '', 0)",
                rusqlite::params![att_id, note_id, attach_str, "diagram.png", "diagram"],
            )
            .expect("insert attachment row");
            drop(conn);
            (note_id, attach_path)
        }

        // CASE A — opt-OUT via Some(false): attachment file must be KEPT.
        // This is the control the new CLI `--delete-attachments` flag exposes
        // (and guards against a future regression where None silently deleted).
        let (id_a, path_a) = make_note_with_attachment(&ctx, &temp, "Keep Attach", "att-3135-a");
        assert!(path_a.exists(), "attachment A should exist pre-delete");
        let r_a = bulk_delete_notes_with_context(&ctx, std::slice::from_ref(&id_a), Some(false))
            .expect("bulk delete Some(false)");
        assert_eq!(r_a.affected, 1);
        assert!(
            path_a.exists(),
            "with Some(false), attachment file must be KEPT on disk"
        );

        // CASE B — force delete via Some(true): attachment file MUST be removed.
        // This is exactly what `vp notes batch --delete --delete-attachments`
        // now wires up (#3135: previously no CLI flag could trigger cleanup).
        let (id_b, path_b) = make_note_with_attachment(&ctx, &temp, "Delete Attach", "att-3135-b");
        assert!(path_b.exists(), "attachment B should exist pre-delete");
        let r_b = bulk_delete_notes_with_context(&ctx, std::slice::from_ref(&id_b), Some(true))
            .expect("bulk delete Some(true)");
        assert_eq!(r_b.affected, 1);
        assert!(
            !path_b.exists(),
            "with Some(true), attachment file must be deleted from disk"
        );
    }
}
