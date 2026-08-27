//! Regression tests for issue #2953: ensure mirror backflow has a single entry
//! point and no duplicate/divergent code paths.
//!
//! After #2931 and #2938 both implemented the same backflow feature, main was
//! verified to contain only ONE copy of the backflow logic (from the squash
//! merge `6e3ac73`). The other branch's commits never reached main.
//!
//! What this test covers:
//!  - End-to-end backflow with external edit (vault unchanged): verifies that
//!    the backflow phase triggers exactly once and produces the correct result.
//!  - A subsequent sync produces zero backflow (no duplicate/cycling backflow).
//!  - The mirror file is re-exported after backflow and is internally consistent.
//!  - Verifies `build_conflict_merge_body` produces exactly one copy of each
//!    version (addressing the concern that merging two divergent implementations
//!    could lead to duplicate content in the conflict marker).

#[cfg(test)]
mod tests {
    use crate::mirror::*;
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{initialize_storage_with_context, save_note_with_context, StorageContext};
    use std::fs;

    #[test]
    fn regression_2953_backflow_single_entry_point() {
        // Basic backflow: vault unchanged, mirror externally edited.
        // The backflow must trigger exactly once and the result must be stable
        // (no duplicate backflow on the next sync cycle).
        let vault = std::env::temp_dir().join(format!("vp_2953_vault_{}", std::process::id()));
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::create_dir_all(&vault);
        let ctx = StorageContext::for_test(&vault);
        initialize_storage_with_context(&ctx).expect("init storage");

        save_note_with_context(
            &ctx,
            NoteDocument {
                meta: NoteMeta {
                    id: "test_note".to_string(),
                    title: "Original".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                    ..Default::default()
                },
                body: "# Original\n\nHello from vault.\n".to_string(),
                ..Default::default()
            },
        )
        .expect("save note");

        let mirror = std::env::temp_dir().join(format!("vp_2953_mirror_{}", std::process::id()));
        let _ = fs::remove_dir_all(&mirror);

        // First sync: creates mirror file + state with content_hash
        let r1 = mirror_sync_with_context(&ctx, &mirror).expect("first sync");
        assert_eq!(r1.created, 1);
        assert_eq!(r1.backflow, 0, "no backflow on first sync");

        // Simulate external edit: modify the mirror file content
        // (keep the anchor so the file remains valid)
        let mirror_file = mirror.join("test_note.md");
        let mirror_content = fs::read_to_string(&mirror_file).expect("read mirror");
        let edited = mirror_content.replace("Hello from vault.", "Hello from external editor!");
        fs::write(&mirror_file, &edited).expect("write external edit");

        // Second sync: should detect hash mismatch and flow back
        let r2 = mirror_sync_with_context(&ctx, &mirror).expect("second sync");
        assert_eq!(
            r2.backflow, 1,
            "external edit must produce exactly ONE backflow (no duplicate entry)"
        );

        // Verify the vault now contains the external edit
        let updated = crate::storage::load_note_with_context(&ctx, "test_note")
            .expect("reload note after backflow");
        assert!(
            updated.body.contains("external editor"),
            "external edit must be flowed back into the vault; got body:\n{}",
            updated.body
        );

        // Verify the mirror file was re-exported after backflow and matches vault
        let reexported = fs::read_to_string(&mirror_file).expect("read re-exported mirror");
        assert!(
            reexported.contains("external editor"),
            "re-exported mirror must contain the backflow result"
        );
        assert!(
            reexported.contains("vaultpilot-note-id"),
            "re-exported mirror must have the anchor comment"
        );

        // Third sync: state should now be stable → zero backflow
        let r3 = mirror_sync_with_context(&ctx, &mirror).expect("third sync");
        assert_eq!(
            r3.backflow, 0,
            "third sync must have zero backflow (stable state after merge)"
        );
    }

    #[test]
    fn regression_2953_build_conflict_merge_body_exactly_once() {
        // The conflict merge builder must produce each section exactly once.
        // Regression guard against the concern that merging two divergent
        // backflow implementations could cause duplicate conflict markers.
        let vault = "# Vault v2\n\nvault-only content\n";
        let mirror = "# Mirror v2\n\nmirror-only content\n";
        let merged = build_conflict_merge_body(vault, mirror);

        // Banner pair appears exactly once
        let open_banner = "<!-- ===== CONFLICT: vault and mirror both changed ===== -->";
        let close_banner = "<!-- ===== END CONFLICT ===== -->";
        assert_eq!(merged.matches(open_banner).count(), 1, "open banner once");
        assert_eq!(merged.matches(close_banner).count(), 1, "close banner once");

        // Each labelled section appears exactly once
        assert_eq!(
            merged.matches("## Vault version (auto-saved)").count(),
            1,
            "vault section once"
        );
        assert_eq!(
            merged.matches("## Mirror version (external edit)").count(),
            1,
            "mirror section once"
        );

        // Each content appears exactly once
        assert_eq!(merged.matches("vault-only content").count(), 1);
        assert_eq!(merged.matches("mirror-only content").count(), 1);

        // Mirror content comes after vault content
        let vault_pos = merged.find("vault-only content").unwrap();
        let mirror_pos = merged.find("mirror-only content").unwrap();
        assert!(mirror_pos > vault_pos, "mirror section after vault section");
    }
}
