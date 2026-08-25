//! Regression tests for issue #2887: `MirrorStateEntry.path` must store the
//! *relative* path inside the mirror directory, not the absolute path.
//!
//! The `.vp-mirror-state.json` state file is meant to be portable: moving or
//! copying the mirror directory should not break the contract. Persisting an
//! absolute path (e.g. `/home/user/.cache/vaultpilot-mirror/<id>.md`) made the
//! state file machine-specific and non-portable. These tests pin the contract
//! that the recorded `path` is relative to the mirror directory.

#[cfg(test)]
mod tests {
    use crate::mirror::{mirror_sync_with_context, read_mirror_state, MIRROR_STATE_FILE};
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{save_note_with_context, StorageContext};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn setup_temp_context() -> (std::path::PathBuf, StorageContext) {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-mirror-2887-{}-{}",
            std::process::id(),
            seq
        ));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    fn make_note(id: &str) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                id: id.to_string(),
                title: format!("Note {id}"),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                ..Default::default()
            },
            body: format!("body of {id}"),
            search_snippet: None,
            search_score: None,
        }
    }

    #[test]
    fn regression_2887_state_records_relative_not_absolute_path() {
        let (vault_dir, ctx) = setup_temp_context();

        // Use an absolute mirror directory so that an absolute-path bug would
        // clearly manifest (e.g. "/tmp/.../<id>.md" instead of "<id>.md").
        let mirror_dir =
            std::env::temp_dir().join(format!("vaultpilot-mirror-out-2887-{}", std::process::id()));
        let _ = fs::remove_dir_all(&mirror_dir);

        save_note_with_context(&ctx, make_note("alpha")).expect("save alpha");

        mirror_sync_with_context(&ctx, &mirror_dir).expect("mirror sync");

        let state_path = mirror_dir.join(MIRROR_STATE_FILE);
        let state = read_mirror_state(&state_path).expect("state must be readable");

        let entry = state.entries.get("alpha").expect("alpha must be recorded");
        assert_eq!(
            entry.path, "alpha.md",
            "recorded path must be relative to the mirror dir, not absolute (#2887)"
        );
        // On POSIX, an absolute path starts with '/'; on Windows with a drive
        // letter it starts with e.g. "C:". Reject both.
        let is_absolute =
            entry.path.starts_with('/') || entry.path.chars().nth(1).is_some_and(|c| c == ':');
        assert!(
            !is_absolute,
            "recorded path must not be absolute: {}",
            entry.path
        );
        // And the relative path must resolve to the real mirror file.
        let resolved = Path::new(&mirror_dir).join(&entry.path);
        assert!(
            resolved.exists(),
            "relative path must resolve to a mirror file"
        );

        let _ = fs::remove_dir_all(&vault_dir);
        let _ = fs::remove_dir_all(&mirror_dir);
    }

    #[test]
    fn regression_2887_relative_path_survives_moved_mirror() {
        // The whole point of relative paths: re-reading a state file that was
        // written under one absolute location must still describe a valid
        // layout when the mirror directory is relocated.
        let (vault_dir, ctx) = setup_temp_context();

        let mirror_dir = std::env::temp_dir().join(format!(
            "vaultpilot-mirror-move-a-2887-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&mirror_dir);

        save_note_with_context(&ctx, make_note("beta")).expect("save beta");
        mirror_sync_with_context(&ctx, &mirror_dir).expect("mirror sync");

        // Relocate the entire mirror dir to a new absolute location.
        let moved = std::env::temp_dir().join(format!(
            "vaultpilot-mirror-move-b-2887-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&moved);
        fs::rename(&mirror_dir, &moved).expect("relocate mirror dir");

        // Re-read the (now-relocated) state and confirm the relative path
        // still resolves correctly under the new location.
        let state =
            read_mirror_state(&moved.join(MIRROR_STATE_FILE)).expect("read relocated state");
        let entry = state.entries.get("beta").expect("beta recorded");
        assert_eq!(entry.path, "beta.md");
        assert!(moved.join(&entry.path).exists());

        let _ = fs::remove_dir_all(&vault_dir);
        let _ = fs::remove_dir_all(&moved);
    }
}
