//! Regression tests for issue #2859: Vault Markdown Mirror.
//!
//! Verifies the core mirror logic:
//!  - each mirror file carries a stable `<!-- vaultpilot-note-id: ... -->` anchor
//!  - note ids round-trip through the anchor
//!  - mirror file paths are keyed by stable note id (so renames never move files)
//!  - `compute_mirror_diff` correctly classifies create / update / delete / unchanged
//!  - the persisted mirror state survives a write+read round-trip

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
    fn regression_2859_mirror_markdown_has_anchor() {
        let body = "---\ntitle: Hello\n---\n\nSome body content here.";
        let md = compose_mirror_markdown(body, "note_1");
        assert!(md.contains("<!-- vaultpilot-note-id: note_1 -->"));
        assert!(md.contains("Some body content here."));
        // Anchor is the last meaningful line.
        assert!(md.trim_end().ends_with("-->"));
        // Original body is preserved verbatim.
        assert!(md.contains("Some body content here."));
    }

    #[test]
    fn regression_2859_extract_anchor_round_trip() {
        let content = "frontmatter\n\nbody text\n\n<!-- vaultpilot-note-id: abc123 -->\n";
        assert_eq!(extract_note_id_anchor(content), Some("abc123".to_string()));
        // No anchor present -> None.
        assert_eq!(extract_note_id_anchor("no anchor in this file"), None);
        // Empty id -> None.
        assert_eq!(
            extract_note_id_anchor("<!-- vaultpilot-note-id:   -->"),
            None
        );
    }

    #[test]
    fn regression_2859_mirror_file_path_is_stable_id() {
        let p = mirror_file_path(Path::new("/tmp/mirror"), "note_42");
        assert_eq!(p, Path::new("/tmp/mirror/note_42.md"));
    }

    #[test]
    fn regression_2859_diff_classifies_create_update_delete_unchanged() {
        let current = vec![meta("a", "t1"), meta("b", "t2")];

        let mut state = MirrorState::new();
        // 'a' already mirrored at t1 -> unchanged
        state.entries.insert(
            "a".to_string(),
            MirrorStateEntry {
                updated_at: "t1".to_string(),
                title: "Note a".to_string(),
                path: "a.md".to_string(),
                content_hash: None,
            },
        );
        // 'b' changed upstream -> update
        state.entries.insert(
            "b".to_string(),
            MirrorStateEntry {
                updated_at: "OLD".to_string(),
                title: "Note b".to_string(),
                path: "b.md".to_string(),
                content_hash: None,
            },
        );
        // 'c' deleted from vault -> delete
        state.entries.insert(
            "c".to_string(),
            MirrorStateEntry {
                updated_at: "t9".to_string(),
                title: "Note c".to_string(),
                path: "c.md".to_string(),
                content_hash: None,
            },
        );

        let diff = compute_mirror_diff(&current, &state);
        assert!(
            diff.to_create.is_empty(),
            "all current notes have state entries"
        );
        assert_eq!(diff.to_update, vec!["b".to_string()]);
        assert_eq!(diff.to_delete, vec!["c".to_string()]);
    }

    #[test]
    fn regression_2859_fresh_state_creates_everything() {
        let current = vec![meta("a", "t1"), meta("b", "t2"), meta("c", "t3")];
        let diff = compute_mirror_diff(&current, &MirrorState::new());
        assert_eq!(
            diff.to_create,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
        assert!(diff.to_update.is_empty());
        assert!(diff.to_delete.is_empty());
    }

    #[test]
    fn regression_2859_state_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("vp_mirror_rt_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let state_path = dir.join(MIRROR_STATE_FILE);

        let mut state = MirrorState::new();
        state.entries.insert(
            "note_x".to_string(),
            MirrorStateEntry {
                updated_at: "2026-07-15T00:00:00Z".to_string(),
                title: "X".to_string(),
                path: "note_x.md".to_string(),
                content_hash: None,
            },
        );
        write_mirror_state(&state_path, &state).expect("write must succeed");

        let read = read_mirror_state(&state_path).expect("state must be readable");
        assert_eq!(read, state);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
