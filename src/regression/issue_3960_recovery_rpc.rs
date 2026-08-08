//! Regression tests for #3960 — WinUI File Recovery UI backend.
//!
//! Bug:      WinUI/mobile had no UI for crash-recovery snapshots; recovery was
//!           only reachable via `vp recovery list/show/restore` in the CLI.
//! Fix:      Added JSON-RPC methods to the agent backend
//!           (recoveryList / recoveryShow / recoveryRestore / recoveryDelete,
//!           src/bin/vaultpilot-agent.rs) plus a WinUI File Recovery dialog.
//!           This regression test exercises the underlying
//!           `vaultpilot_lib::recovery` functions with the exact semantics the
//!           RPC handlers use: list → show → restore (write content back into
//!           the vault at the original relative path) → delete.
//!
//! Run:      cargo test --lib regression_3960
//!           (or: cargo test --lib issue_3960)

use crate::recovery::{
    delete_recovery_snapshot, get_recovery_snapshot, list_recovery_snapshots,
    save_recovery_snapshot,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn test_vault() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-regression-3960-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("create temp vault");
        dir
    }

    #[test]
    fn regression_3960_recovery_rpc_semantics_roundtrip() {
        let vault = test_vault();
        let note_rel = "notes/recovered-note.md";

        // 1. Save a snapshot (simulates an unsaved edit buffer captured by
        //    the editor, mirroring what recovery_save is called with).
        let saved = save_recovery_snapshot(
            &vault,
            note_rel,
            "Recovered Note",
            "# Recovered content\n\nline two\n",
        )
        .expect("save recovery snapshot");
        assert!(!saved.id.is_empty());

        // 2. recoveryList: the snapshot appears, newest first, with the
        //    fields the WinUI list view needs.
        let list = list_recovery_snapshots(&vault, None).expect("list recovery snapshots");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].note_path, note_rel);
        assert_eq!(list[0].title, "Recovered Note");
        assert_eq!(list[0].content_size, 30);
        assert!(!list[0].created_at.is_empty());

        // 3. recoveryShow: full content is available for preview.
        let shown = get_recovery_snapshot(&vault, &saved.id)
            .expect("get recovery snapshot")
            .expect("snapshot exists");
        assert_eq!(shown.content, "# Recovered content\n\nline two\n");

        // 4. recoveryRestore: the RPC handler writes the content back into
        //    the vault at the snapshot's original relative path. Replicate
        //    exactly what recovery_restore_async does (agent.rs).
        let target = vault.join(&shown.note_path);
        fs::create_dir_all(target.parent().unwrap()).expect("create note parent dir");
        fs::write(&target, &shown.content).expect("write restored note");
        let restored = fs::read_to_string(&target).expect("read restored note");
        assert_eq!(restored, "# Recovered content\n\nline two\n");

        // 5. recoveryDelete: snapshot is removed.
        let removed = delete_recovery_snapshot(&vault, &saved.id).expect("delete snapshot");
        assert!(removed);
        assert!(
            get_recovery_snapshot(&vault, &saved.id)
                .expect("query after delete")
                .is_none(),
            "snapshot must be gone after delete"
        );
        assert!(list_recovery_snapshots(&vault, None)
            .expect("list after delete")
            .is_empty());

        fs::remove_dir_all(&vault).ok();
    }

    #[test]
    fn regression_3960_recovery_list_filters_by_note_path() {
        let vault = test_vault();
        save_recovery_snapshot(&vault, "a.md", "A", "aaa").expect("save a");
        save_recovery_snapshot(&vault, "b.md", "B", "bbb").expect("save b");

        let filtered = list_recovery_snapshots(&vault, Some("a.md")).expect("filtered list");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].note_path, "a.md");

        fs::remove_dir_all(&vault).ok();
    }
}
