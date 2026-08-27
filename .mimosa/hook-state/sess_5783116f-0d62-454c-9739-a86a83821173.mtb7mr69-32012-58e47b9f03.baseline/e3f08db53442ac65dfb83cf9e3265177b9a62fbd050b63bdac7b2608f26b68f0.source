//! Regression tests for issue #2884: `vp mirror --watch` persisted state must
//! be *read back*, not dead write-only code.
//!
//! The original design wrote `.vp-mirror-state.json` on every cycle but never
//! consumed it, so every restart re-exported the entire vault. These tests pin
//! the contract that the state file is read back and used to skip unchanged
//! notes (incremental, fast-start sync).

#[cfg(test)]
mod tests {
    use crate::mirror::*;
    use crate::models::NoteMeta;
    use std::path::Path;

    fn meta(id: &str, updated_at: &str) -> NoteMeta {
        NoteMeta {
            id: id.to_string(),
            title: format!("Note {id}"),
            updated_at: updated_at.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn regression_2884_state_is_read_back_and_drives_incremental_sync() {
        let dir = std::env::temp_dir().join(format!("vp_mirror_2884_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let state_path = dir.join(MIRROR_STATE_FILE);

        // Simulate a previously persisted state: note 'a' mirrored at t1.
        let mut prior = MirrorState::new();
        prior.entries.insert(
            "a".to_string(),
            MirrorStateEntry {
                updated_at: "t1".to_string(),
                title: "Note a".to_string(),
                path: "a.md".to_string(),
                content_hash: None,
            },
        );
        write_mirror_state(&state_path, &prior).expect("write prior state");

        // The watch loop must READ this back (this is the #2884 fix).
        let read = read_mirror_state(&state_path).expect("persisted state must be readable");
        assert_eq!(read, prior, "read-back state equals what was written");

        // Vault unchanged since last run -> diff is empty (no full re-export).
        let current = vec![meta("a", "t1")];
        let diff = compute_mirror_diff(&current, &read);
        assert!(
            diff.to_create.is_empty() && diff.to_update.is_empty() && diff.to_delete.is_empty(),
            "reading state back must skip unchanged notes (incremental, not full re-export)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_2884_missing_state_falls_back_to_full_create() {
        // When no state file exists, every current note is treated as new.
        let dir =
            std::env::temp_dir().join(format!("vp_mirror_2884_missing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let state_path = dir.join(MIRROR_STATE_FILE);
        assert!(read_mirror_state(&state_path).is_none());

        let current = vec![meta("a", "t1"), meta("b", "t2")];
        let diff = compute_mirror_diff(&current, &MirrorState::new());
        assert_eq!(diff.to_create, vec!["a".to_string(), "b".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_2884_corrupt_state_is_treated_as_missing() {
        let dir =
            std::env::temp_dir().join(format!("vp_mirror_2884_corrupt_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let state_path = dir.join(MIRROR_STATE_FILE);
        std::fs::write(&state_path, "this is not valid json {{{").unwrap();

        // A corrupt state file must not panic and must be treated as absent,
        // so the caller falls back to a full sync instead of crashing.
        assert!(read_mirror_state(&state_path).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_2884_state_path_constant_is_hidden_file() {
        assert!(MIRROR_STATE_FILE.starts_with('.'));
        assert!(MIRROR_STATE_FILE.ends_with(".json"));
        let _ = Path::new(MIRROR_STATE_FILE);
    }
}
