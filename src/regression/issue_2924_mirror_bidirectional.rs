/// Regression tests for mirror bidirectional sync (#2924).
///
/// Tests the new pure functions: compute_file_hash, strip_anchor,
/// detect_external_edits, detect_conflicts.

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::io::Write;

    use crate::mirror::{
        compose_mirror_markdown, compute_file_hash, detect_conflicts, detect_external_edits,
        strip_anchor, MirrorDiff, MirrorState, MirrorStateEntry,
    };
    use crate::models::NoteMeta;

    fn meta(id: &str, updated_at: &str) -> NoteMeta {
        NoteMeta {
            id: id.to_string(),
            title: "Title".to_string(),
            updated_at: updated_at.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            source: "local".to_string(),
            path: String::new(),
            summary: String::new(),
            tags: vec![],
            keywords: vec![],
            platform: String::new(),
            board: String::new(),
            kernel: String::new(),
            status: String::new(),
            collections: vec![],
        }
    }

    // ── compute_file_hash ────────────────────────────────

    #[test]
    fn hash_returns_some_for_existing_file() {
        let dir = std::env::temp_dir().join(format!("vp_mirror_hash_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_file.txt");
        std::fs::write(&path, b"hello world").unwrap();
        let hash = compute_file_hash(&path);
        assert!(hash.is_some());
        assert_eq!(hash.unwrap().len(), 64); // SHA-256 hex is 64 chars
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_differs_for_different_content() {
        let dir = std::env::temp_dir().join(format!("vp_mirror_hdiff_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p1 = dir.join("f1.txt");
        let p2 = dir.join("f2.txt");
        std::fs::write(&p1, b"hello").unwrap();
        std::fs::write(&p2, b"world").unwrap();
        let h1 = compute_file_hash(&p1).unwrap();
        let h2 = compute_file_hash(&p2).unwrap();
        assert_ne!(h1, h2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_same_for_identical_content() {
        let dir = std::env::temp_dir().join(format!("vp_mirror_hsame_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p1 = dir.join("a.txt");
        let p2 = dir.join("b.txt");
        std::fs::write(&p1, b"same content").unwrap();
        std::fs::write(&p2, b"same content").unwrap();
        let h1 = compute_file_hash(&p1).unwrap();
        let h2 = compute_file_hash(&p2).unwrap();
        assert_eq!(h1, h2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_returns_none_for_nonexistent_file() {
        let hash = compute_file_hash(std::path::Path::new("/nonexistent/xyz_2924_test"));
        assert!(hash.is_none());
    }

    // ── strip_anchor ─────────────────────────────────────

    #[test]
    fn strip_anchor_removes_anchor_comment() {
        let content =
            "---\ntitle: Note\n---\n\nBody text here.\n\n<!-- vaultpilot-note-id: abc123 -->";
        let stripped = strip_anchor(content);
        assert_eq!(stripped, "---\ntitle: Note\n---\n\nBody text here.");
    }

    #[test]
    fn strip_anchor_no_anchor_returns_content() {
        let content = "Just plain text, no anchor comment.";
        let stripped = strip_anchor(content);
        assert_eq!(stripped, content.trim_end());
    }

    #[test]
    fn strip_anchor_empty_string() {
        let stripped = strip_anchor("");
        assert_eq!(stripped, "");
    }

    // ── detect_external_edits ────────────────────────────

    #[test]
    fn external_edit_detected_when_hash_differs_and_vault_unchanged() {
        let current = vec![meta("a", "t1"), meta("b", "t2")];
        let current_ids: HashSet<&String> = current.iter().map(|m| &m.id).collect();

        let mut state = MirrorState::new();
        state.entries.insert(
            "a".to_string(),
            MirrorStateEntry {
                updated_at: "t1".to_string(),
                title: "A".to_string(),
                path: "a.md".to_string(),
                content_hash: Some("abc123".to_string()),
            },
        );
        state.entries.insert(
            "b".to_string(),
            MirrorStateEntry {
                updated_at: "t2".to_string(),
                title: "B".to_string(),
                path: "b.md".to_string(),
                content_hash: Some("def456".to_string()),
            },
        );

        // Vault untouched → diff has no creates or updates
        let diff = MirrorDiff::default();

        // Note 'a' was NOT externally edited (hash matches), note 'b' WAS (hash changed)
        let mut current_hashes = HashMap::new();
        current_hashes.insert("a".to_string(), "abc123".to_string()); // matches
        current_hashes.insert("b".to_string(), "zzz999".to_string()); // differs!

        let edits = detect_external_edits(&current_ids, &diff, &state, &current_hashes);
        assert_eq!(edits, vec!["b".to_string()]);
    }

    #[test]
    fn external_edit_skipped_when_vault_also_modified() {
        let current = vec![meta("a", "t1_NEW"), meta("b", "t2")]; // 'a' updated_at changed
        let current_ids: HashSet<&String> = current.iter().map(|m| &m.id).collect();

        let mut state = MirrorState::new();
        state.entries.insert(
            "a".to_string(),
            MirrorStateEntry {
                updated_at: "t1_OLD".to_string(),
                title: "A".to_string(),
                path: "a.md".to_string(),
                content_hash: Some("abc123".to_string()),
            },
        );
        state.entries.insert(
            "b".to_string(),
            MirrorStateEntry {
                updated_at: "t2".to_string(),
                title: "B".to_string(),
                path: "b.md".to_string(),
                content_hash: Some("def456".to_string()),
            },
        );

        let diff = crate::mirror::compute_mirror_diff(&current, &state);
        // 'a' is in to_update (vault changed)

        // Both have hash changes but 'a' is in to_update → excluded from external edits
        let mut current_hashes = HashMap::new();
        current_hashes.insert("a".to_string(), "zzz999".to_string()); // differs!
        current_hashes.insert("b".to_string(), "zzz999".to_string()); // also differs

        let edits = detect_external_edits(&current_ids, &diff, &state, &current_hashes);
        // Only 'b' is detected — 'a' is skipped because vault already wants to update it
        assert_eq!(edits, vec!["b".to_string()]);
    }

    #[test]
    fn external_edit_no_hash_in_state_means_not_detected() {
        let current = vec![meta("a", "t1")];
        let current_ids: HashSet<&String> = current.iter().map(|m| &m.id).collect();

        let mut state = MirrorState::new();
        state.entries.insert(
            "a".to_string(),
            MirrorStateEntry {
                updated_at: "t1".to_string(),
                title: "A".to_string(),
                path: "a.md".to_string(),
                content_hash: None, // legacy entry, no hash
            },
        );

        let diff = MirrorDiff::default();
        let mut current_hashes = HashMap::new();
        current_hashes.insert("a".to_string(), "anything".to_string());

        let edits = detect_external_edits(&current_ids, &diff, &state, &current_hashes);
        assert!(edits.is_empty()); // No stored hash → can't detect
    }

    #[test]
    fn external_edit_no_hashes_at_all() {
        let current = vec![meta("a", "t1"), meta("b", "t2")];
        let current_ids: HashSet<&String> = current.iter().map(|m| &m.id).collect();
        let state = MirrorState::new();
        let diff = MirrorDiff::default();
        let current_hashes = HashMap::new();

        let edits = detect_external_edits(&current_ids, &diff, &state, &current_hashes);
        assert!(edits.is_empty());
    }

    // ── detect_conflicts ─────────────────────────────────

    #[test]
    fn conflict_when_both_vault_and_mirror_modified() {
        let current = vec![meta("a", "t2"), meta("b", "t2")]; // both changed from t1
        let mut state = MirrorState::new();
        state.entries.insert(
            "a".to_string(),
            MirrorStateEntry {
                updated_at: "t1".to_string(),
                title: "A".to_string(),
                path: "a.md".to_string(),
                content_hash: Some("abc".to_string()),
            },
        );
        state.entries.insert(
            "b".to_string(),
            MirrorStateEntry {
                updated_at: "t1".to_string(),
                title: "B".to_string(),
                path: "b.md".to_string(),
                content_hash: Some("def".to_string()),
            },
        );

        let diff = crate::mirror::compute_mirror_diff(&current, &state);
        // Both 'a' and 'b' are in to_update

        // Only 'a' was also externally edited
        let key_a = "a".to_string();
        let mut external_edits_set: HashSet<&String> = HashSet::new();
        external_edits_set.insert(&key_a);

        let conflicts = detect_conflicts(&diff, &external_edits_set);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(*conflicts[0], "a");
    }

    #[test]
    fn no_conflict_when_only_vault_modified() {
        let current = vec![meta("a", "t2")];
        let mut state = MirrorState::new();
        state.entries.insert(
            "a".to_string(),
            MirrorStateEntry {
                updated_at: "t1".to_string(),
                title: "A".to_string(),
                path: "a.md".to_string(),
                content_hash: Some("abc".to_string()),
            },
        );

        let diff = crate::mirror::compute_mirror_diff(&current, &state);
        let external_edits_set: HashSet<&String> = HashSet::new(); // empty

        let conflicts = detect_conflicts(&diff, &external_edits_set);
        assert!(conflicts.is_empty());
    }

    // ── MirrorStateEntry backward compatibility ──────────

    #[test]
    fn deserialize_legacy_state_without_content_hash() {
        let legacy_json = r#"{
            "version": 0,
            "entries": {
                "note_a": {
                    "updated_at": "2026-01-01T00:00:00Z",
                    "title": "Note A",
                    "path": "note_a.md"
                }
            }
        }"#;
        let state: MirrorState = serde_json::from_str(legacy_json).unwrap();
        let entry = state.entries.get("note_a").unwrap();
        assert_eq!(entry.updated_at, "2026-01-01T00:00:00Z");
        assert_eq!(entry.content_hash, None);
    }

    #[test]
    fn deserialize_state_with_content_hash() {
        let json = r#"{
            "version": 0,
            "entries": {
                "note_b": {
                    "updated_at": "2026-07-15T00:00:00Z",
                    "title": "Note B",
                    "path": "note_b.md",
                    "content_hash": "a1b2c3d4e5f6..."
                }
            }
        }"#;
        let state: MirrorState = serde_json::from_str(json).unwrap();
        let entry = state.entries.get("note_b").unwrap();
        assert_eq!(entry.content_hash, Some("a1b2c3d4e5f6...".to_string()));
    }

    #[test]
    fn serialize_state_skips_none_content_hash() {
        let mut state = MirrorState::new();
        state.entries.insert(
            "note_x".to_string(),
            MirrorStateEntry {
                updated_at: "t".to_string(),
                title: "X".to_string(),
                path: "x.md".to_string(),
                content_hash: None,
            },
        );
        let json = serde_json::to_string(&state).unwrap();
        // The JSON should NOT contain "content_hash" because it was None + skip_serializing_if
        assert!(!json.contains("content_hash"));
    }

    #[test]
    fn compose_mirror_markdown_handles_unicode_without_panic() {
        // Regression: ensure compose + strip don't panic on non-ASCII.
        let content = "你\n好\n世\n界";
        let note_id = "note_unicode";
        let composed = compose_mirror_markdown(content, note_id);
        // The composed file must contain the anchor
        assert!(composed.contains(note_id));
        // Strip the anchor and verify the original content is preserved
        let stripped = strip_anchor(&composed);
        assert_eq!(stripped, content);
    }
}
