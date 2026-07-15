//! Regression tests for issue #2924: Markdown Mirror 双向同步
//!
//! Verifies that external edits to mirror files flow back into the vault:
//!  - `hash_content` produces stable hex digests
//!  - `strip_anchor_from_content` removes the anchor line from mirror files
//!  - `detect_external_changes` identifies files with mismatched hashes
//!  - `MirrorStateEntry` content_hash survives JSON round-trip (including None)
//!  - A full mirror_sync_with_context cycle writes hashes, then detects
//!    an external edit and flows it back

#[cfg(test)]
mod tests {
    use crate::mirror::*;
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        delete_note_with_context, initialize_storage_with_context, save_note_with_context,
        StorageContext,
    };
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn regression_2924_hash_content_is_stable() {
        let hash1 = hash_content("hello world");
        let hash2 = hash_content("hello world");
        assert_eq!(hash1, hash2, "same input yields same hash");
        assert_ne!(hash1, hash_content("hello world!"));
        // SHA-256 hex digest is 64 chars
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn regression_2924_strip_anchor_removes_anchor_line() {
        let full = "---\ntitle: Test\n---\n\nbody text\n\n<!-- vaultpilot-note-id: note_42 -->\n";
        let stripped = strip_anchor_from_content(full);
        assert!(!stripped.contains("vaultpilot-note-id"));
        assert!(stripped.contains("body text"));
        assert!(stripped.contains("title: Test"));
    }

    #[test]
    fn regression_2924_strip_anchor_with_extra_trailing_text() {
        // External editor may add content after the anchor
        let full = "---\ntitle: Test\n---\n\nbody\n\n<!-- vaultpilot-note-id: note_1 -->\nextra trailing text\n";
        let stripped = strip_anchor_from_content(full);
        assert!(!stripped.contains("vaultpilot-note-id"));
        assert!(stripped.contains("extra trailing text"));
    }

    #[test]
    fn regression_2924_strip_anchor_no_anchor_is_noop() {
        let content = "just plain markdown\nno anchor here\n";
        let stripped = strip_anchor_from_content(content);
        assert_eq!(stripped, content.trim_end());
    }

    #[test]
    fn regression_2924_detect_external_changes() {
        let mut state = MirrorState::new();
        state.entries.insert(
            "n1".to_string(),
            MirrorStateEntry {
                updated_at: "t1".to_string(),
                title: "N1".to_string(),
                path: "n1.md".to_string(),
                content_hash: Some("abc123".to_string()),
            },
        );
        state.entries.insert(
            "n2".to_string(),
            MirrorStateEntry {
                updated_at: "t2".to_string(),
                title: "N2".to_string(),
                path: "n2.md".to_string(),
                content_hash: None, // legacy entry, no hash stored
            },
        );

        // n1 hash matches -> unchanged; n2 has no stored hash -> treated as changed
        let mut disk_hashes: HashMap<String, String> = HashMap::new();
        disk_hashes.insert("n1".to_string(), "abc123".to_string());
        disk_hashes.insert("n2".to_string(), "def456".to_string());

        let changes = detect_external_changes(&disk_hashes, &state);
        assert_eq!(
            changes,
            vec!["n2".to_string()],
            "n2 has no stored hash = external change"
        );
    }

    #[test]
    fn regression_2924_mirror_state_entry_json_round_trips_content_hash() {
        // Verify that None content_hash is serialized/skipped properly
        let entry = MirrorStateEntry {
            updated_at: "t0".to_string(),
            title: "T".to_string(),
            path: "p.md".to_string(),
            content_hash: None,
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        assert!(!json.contains("content_hash"), "None should be skipped");

        let entry2 = MirrorStateEntry {
            updated_at: "t0".to_string(),
            title: "T".to_string(),
            path: "p.md".to_string(),
            content_hash: Some("abc".to_string()),
        };
        let json2 = serde_json::to_string(&entry2).expect("serialize");
        assert!(json2.contains("content_hash"), "Some should be included");
        assert!(json2.contains("abc"));
    }

    #[test]
    fn regression_2924_full_sync_writes_and_detects_hashes() {
        let vault = std::env::temp_dir().join(format!("vp_2924_vault_{}", std::process::id()));
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::create_dir_all(&vault);
        let ctx = StorageContext::for_test(&vault);
        initialize_storage_with_context(&ctx).expect("init storage");

        save_note_with_context(
            &ctx,
            NoteDocument {
                meta: NoteMeta {
                    id: "alpha".to_string(),
                    title: "Note Alpha".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                    ..Default::default()
                },
                body: "# Alpha\n\nhello from alpha\n".to_string(),
                ..Default::default()
            },
        )
        .expect("save alpha");

        let mirror = std::env::temp_dir().join(format!("vp_2924_mirror_{}", std::process::id()));
        let _ = fs::remove_dir_all(&mirror);

        // First sync: creates mirror file + state with content_hash
        let r1 = mirror_sync_with_context(&ctx, &mirror).expect("first sync");
        assert_eq!(r1.created, 1);
        assert_eq!(r1.backflow, 0);

        // Read state and verify content_hash is present
        let state_path = mirror.join(MIRROR_STATE_FILE);
        let state = read_mirror_state(&state_path).expect("read state");
        let entry = state.entries.get("alpha").expect("alpha recorded");
        assert!(
            entry.content_hash.is_some(),
            "first sync writes content_hash"
        );

        // Simulate external edit: modify the mirror file
        let mirror_file = mirror.join("alpha.md");
        let original = fs::read_to_string(&mirror_file).expect("read mirror");
        let edited = original.replace("hello from alpha", "hello from VS Code (externally edited)");
        fs::write(&mirror_file, &edited).expect("write external edit");

        // Second sync: should detect hash mismatch and flow back
        let r2 = mirror_sync_with_context(&ctx, &mirror).expect("second sync");
        assert_eq!(r2.backflow, 1, "external edit must flow back to vault");

        // After backflow, the mirror file should be re-exported (reflecting merged vault)
        let final_mirror = fs::read_to_string(&mirror_file).expect("read final mirror");
        assert!(
            final_mirror.contains("externally edited"),
            "mirror file must contain external edit after backflow+re-export"
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_dir_all(&mirror);
    }

    #[test]
    fn regression_2924_orphan_test_backflow_compatible() {
        // The orphan test (#2889) creates 2 notes, mirrors, deletes one,
        // removes state, then syncs again — backflow phase must not crash
        // when encountering a state with entries pointing to now-deleted notes.
        let vault = std::env::temp_dir().join(format!("vp_2924_v2_{}", std::process::id()));
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::create_dir_all(&vault);
        let ctx = StorageContext::for_test(&vault);
        initialize_storage_with_context(&ctx).expect("init storage");

        let make_note = |id: &str| NoteDocument {
            meta: NoteMeta {
                id: id.to_string(),
                title: format!("Note {id}"),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                ..Default::default()
            },
            body: format!("body of {id}"),
            ..Default::default()
        };

        save_note_with_context(&ctx, make_note("A")).expect("save A");
        save_note_with_context(&ctx, make_note("B")).expect("save B");

        let mirror = std::env::temp_dir().join(format!("vp_2924_mir2_{}", std::process::id()));
        let _ = fs::remove_dir_all(&mirror);

        let r1 = mirror_sync_with_context(&ctx, &mirror).expect("first sync");
        assert_eq!(r1.created, 2);

        // Delete A from vault
        assert!(delete_note_with_context(&ctx, "A").expect("delete A"));

        // Remove state file to simulate state loss
        let _ = fs::remove_file(mirror.join(MIRROR_STATE_FILE));

        // Second sync — must not panic despite backflow phase
        let r2 = mirror_sync_with_context(&ctx, &mirror).expect("second sync");
        assert!(!mirror.join("A.md").exists(), "orphan cleaned up");
        assert!(mirror.join("B.md").exists(), "B still present");

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_dir_all(&mirror);
    }

    #[test]
    fn regression_2924_compose_mirror_with_hash_determinism() {
        // Ensure compose_mirror_markdown produces deterministic output
        let body = "---\ntitle: T\n---\n\ncontent\n";
        let md1 = compose_mirror_markdown(body, "id_1");
        let md2 = compose_mirror_markdown(body, "id_1");
        assert_eq!(md1, md2);
        assert_eq!(hash_content(&md1), hash_content(&md2));
    }
}
