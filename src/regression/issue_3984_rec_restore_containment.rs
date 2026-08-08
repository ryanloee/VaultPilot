//! Regression tests for #3984 — recovery restore/save path containment.
//!
//! Bug:  `recovery_restore_async` (src/bin/vaultpilot-agent.rs) joined a
//!       snapshot's `note_path` straight onto the vault dir and wrote the
//!       content there with no containment check. The crash-recovery DB
//!       lives *outside* the vault (OS data dir), so a `note_path` of
//!       `../../../etc/pwned` (tampered DB, or a buggy crash-time capture)
//!       would happily write arbitrary files outside the vault.
//!       `save_recovery_snapshot` (src/recovery.rs) also stored any
//!       `note_path` without validation.
//! Fix:  `vaultpilot_lib::recovery::validate_recovery_note_path` rejects
//!       absolute, Windows-rooted/drive and `..`-traversal paths at the save
//!       API boundary; `recovery_target_path` re-validates and additionally
//!       refuses symlink escapes before any restore write. The agent's
//!       `recovery_restore_async` now resolves its write target through
//!       `recovery_target_path`, so a hostile snapshot can no longer land a
//!       byte outside the vault.
//!
//! Run: cargo test -p vaultpilot --lib regression_3984

use crate::recovery::{recovery_target_path, validate_recovery_note_path};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn test_vault() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-regression-3984-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("create temp vault");
        dir
    }

    #[test]
    fn regression_3984_restore_refuses_escaping_note_paths() {
        let vault = test_vault();
        // The exact attack class from #3984: paths that walk out of the vault,
        // in both slash directions and with Windows/C-style prefixes.
        for bad in [
            "../../../etc/pwned",
            "/etc/pwned",
            "sub/../../etc/pwned",
            "..\\..\\etc\\pwned",
            "C:\\Windows\\pwned",
            "C:/Windows/pwned",
        ] {
            let err = validate_recovery_note_path(bad)
                .expect_err("save-side validation must reject escaping path");
            assert!(
                err.to_string().contains("vault-relative") || err.to_string().contains("'..'"),
                "unexpected save-side error for {bad:?}: {err}"
            );

            let err = recovery_target_path(&vault, bad)
                .expect_err("restore-side target resolution must refuse escaping path");
            assert!(
                err.to_string().contains("vault") || err.to_string().contains("'..'"),
                "unexpected restore-side error for {bad:?}: {err}"
            );
        }

        fs::remove_dir_all(&vault).ok();
    }

    #[test]
    fn regression_3984_restore_target_for_valid_paths_stays_in_vault() {
        let vault = test_vault();
        let target = recovery_target_path(&vault, "deep/inbox/draft.md")
            .expect("normal vault-relative path must resolve");
        assert_eq!(target, vault.join("deep/inbox/draft.md"));
        assert!(target.starts_with(&vault), "target must stay inside vault");
        assert!(target.strip_prefix(&vault).is_ok());
        fs::remove_dir_all(&vault).ok();
    }

    #[cfg(unix)]
    #[test]
    fn regression_3984_restore_refuses_symlink_escape() {
        use std::os::unix::fs::symlink;

        let vault = test_vault();
        let outside = std::env::temp_dir().join(format!(
            "vaultpilot-regression-3984-outside-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&outside).expect("create outside dir");
        symlink(&outside, vault.join("leak")).expect("create symlink");

        let err = recovery_target_path(&vault, "leak/pwned.md")
            .expect_err("symlinked subdir pointing outside the vault must be refused");
        assert!(
            err.to_string().contains("symlink"),
            "unexpected error: {err}"
        );

        fs::remove_file(vault.join("leak")).ok();
        fs::remove_dir_all(&outside).ok();
        fs::remove_dir_all(&vault).ok();
    }
}
