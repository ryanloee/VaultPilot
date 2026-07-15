//! Regression tests for issue #2889: `vp mirror` must clean up orphan `.md`
//! files left behind by deleted notes when the persisted state is missing or
//! corrupt, and `extract_note_id_anchor` must actually participate in the
//! reconcile path (disk-scan fallback) rather than being dead test-only code.
//!
//! Reproduction from the issue:
//!  1. `vp mirror` of vault {A, B} -> A.md, B.md, state
//!  2. delete note A from the vault (state still records A)
//!  3. delete/rename `.vp-mirror-state.json` (corrupt/migrate)
//!  4. `vp mirror` again -> A.md must NOT linger as an orphan

#[cfg(test)]
mod tests {
    use crate::mirror::*;
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        delete_note_with_context, initialize_storage_with_context, save_note_with_context,
        StorageContext,
    };
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    fn note_doc(id: &str, title: &str, updated_at: &str) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                id: id.to_string(),
                title: title.to_string(),
                updated_at: updated_at.to_string(),
                ..Default::default()
            },
            body: format!("# {title}\n\nbody of {id}"),
            ..Default::default()
        }
    }

    #[test]
    fn regression_2889_disk_scan_finds_anchored_files_only() {
        let dir = std::env::temp_dir().join(format!("vp_2889_scan_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        // A valid mirror file carrying the note-id anchor.
        std::fs::write(
            dir.join("note_x.md"),
            "title: X\n\nbody\n\n<!-- vaultpilot-note-id: note_x -->\n",
        )
        .unwrap();
        // A `.md` file without an anchor must be ignored.
        std::fs::write(dir.join("note_y.md"), "no anchor here").unwrap();
        // The state file must be ignored.
        std::fs::write(dir.join(MIRROR_STATE_FILE), "{}").unwrap();
        // A non-`.md` file must be ignored.
        std::fs::write(dir.join("readme.txt"), "x").unwrap();

        let files = disk_scan_mirror_files(&dir).expect("scan must succeed");
        assert_eq!(files.len(), 1, "only the anchored .md must be indexed");
        assert!(files.contains_key("note_x"));
        assert!(!files.contains_key("note_y"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_2889_orphan_identification_is_pure() {
        let mut disk: HashMap<String, PathBuf> = HashMap::new();
        disk.insert("gone".to_string(), PathBuf::from("/m/gone.md"));
        disk.insert(
            "kept_current".to_string(),
            PathBuf::from("/m/kept_current.md"),
        );
        disk.insert("kept_state".to_string(), PathBuf::from("/m/kept_state.md"));

        let current: Vec<String> = vec!["kept_current".to_string()];
        let current_ids: HashSet<&String> = current.iter().collect();

        let mut state = MirrorState::new();
        state.entries.insert(
            "kept_state".to_string(),
            MirrorStateEntry {
                updated_at: "t".to_string(),
                title: "s".to_string(),
                path: "kept_state.md".to_string(),
                content_hash: None,
            },
        );

        let orphans = orphan_mirror_files(&disk, &current_ids, &state);
        assert_eq!(orphans.len(), 1, "only the truly-gone file is an orphan");
        assert_eq!(orphans[0], &PathBuf::from("/m/gone.md"));
    }

    #[test]
    fn regression_2889_orphan_md_removed_after_state_loss() {
        let vault = std::env::temp_dir().join(format!("vp_2889_vault_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        let _ = std::fs::create_dir_all(&vault);
        let ctx = StorageContext::for_test(&vault);
        initialize_storage_with_context(&ctx).expect("init storage");

        // Vault contains notes A and B.
        save_note_with_context(&ctx, note_doc("A", "Note A", "t1")).expect("save A");
        save_note_with_context(&ctx, note_doc("B", "Note B", "t2")).expect("save B");

        let mirror = std::env::temp_dir().join(format!("vp_2889_mirror_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&mirror);
        let _ = std::fs::create_dir_all(&mirror);

        // First full mirror: writes A.md, B.md, and the state file.
        let r1 = mirror_sync_with_context(&ctx, &mirror).expect("first sync");
        assert_eq!(r1.created, 2);
        assert!(mirror.join("A.md").exists(), "A mirrored");
        assert!(mirror.join("B.md").exists(), "B mirrored");
        assert!(mirror.join(MIRROR_STATE_FILE).exists(), "state written");

        // Delete note A from the vault; its mirror `.md` lingers on disk.
        assert!(delete_note_with_context(&ctx, "A").expect("delete A"));

        // Simulate state loss / corruption / migration.
        let _ = std::fs::remove_file(mirror.join(MIRROR_STATE_FILE));

        // Second sync with missing state must NOT leave A.md as an orphan.
        let r2 = mirror_sync_with_context(&ctx, &mirror).expect("second sync");
        assert!(
            !mirror.join("A.md").exists(),
            "orphan A.md must be cleaned up via disk-scan"
        );
        assert!(
            mirror.join("B.md").exists(),
            "current note B.md must remain"
        );
        assert_eq!(r2.deleted, 1, "exactly one orphan file removed");
        assert_eq!(r2.created, 1, "note B re-exported to fill the state gap");

        let _ = std::fs::remove_dir_all(&vault);
        let _ = std::fs::remove_dir_all(&mirror);
    }
}
