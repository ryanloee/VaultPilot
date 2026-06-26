//! Database backup and rotation.
//!
//! Extracted from storage.rs in Phase 1 of the incremental module split (#1212).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::debug;

/// On Windows, `fs::rename` fails if the destination file already exists.
/// This helper removes the file first on Windows; on Unix it is a no-op
/// because `rename` atomically replaces the destination (#829).
fn windows_remove_if_exists(path: &Path) {
    #[cfg(windows)]
    {
        let _ = fs::remove_file(path);
    }
    #[cfg(not(windows))]
    {
        let _ = path; // suppress unused variable warning
    }
}

/// Auto-backup the SQLite database, keeping the last 3 historical backups.
/// Creates rotating backups: db.bak, db.bak.1, db.bak.2, db.bak.3
/// where .bak is the most recent copy and .bak.3 is the oldest kept.
pub(crate) fn auto_backup_database(db_path: &Path) -> Result<()> {
    if !db_path.exists() {
        debug!("no existing database to backup");
        return Ok(());
    }

    let backup_dir = db_path.parent().unwrap_or(Path::new("."));
    let file_name = db_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("db_path has no file name: {}", db_path.display()))?;
    let file_name_str = file_name.to_string_lossy();
    let max_backups = 3;

    // 1. Delete the oldest backup that would overflow the limit (if it exists).
    //    We keep `max_backups` historical backups (.bak.1 .. .bak.{max_backups}),
    //    so .bak.{max_backups} is the one to remove before rotating.
    let overflow = backup_dir.join(format!("{file_name_str}.bak.{max_backups}"));
    if overflow.exists() {
        if let Err(e) = fs::remove_file(&overflow) {
            tracing::warn!(path = %overflow.display(), error = %e, "Failed to remove old backup");
        }
    }

    // 2. Rotate: .bak.{max_backups-1} → .bak.{max_backups}, ..., .bak.1 → .bak.2
    for i in (1..max_backups).rev() {
        let older = backup_dir.join(format!("{file_name_str}.bak.{i}"));
        let newer = backup_dir.join(format!("{file_name_str}.bak.{}", i + 1));
        if older.exists() {
            // On Windows, rename fails if the destination already exists.
            // Remove the destination first to ensure the rename succeeds.
            windows_remove_if_exists(&newer);
            if let Err(e) = fs::rename(&older, &newer) {
                tracing::warn!(from = %older.display(), to = %newer.display(), error = %e, "Failed to rotate backup");
            }
        }
    }

    // Move current .bak to .bak.1
    let current_bak = backup_dir.join(format!("{file_name_str}.bak"));
    if current_bak.exists() {
        let bak1 = backup_dir.join(format!("{file_name_str}.bak.1"));
        // On Windows, rename fails if the destination already exists.
        windows_remove_if_exists(&bak1);
        if let Err(e) = fs::rename(&current_bak, &bak1) {
            tracing::warn!(from = %current_bak.display(), to = %bak1.display(), error = %e, "Failed to rotate current backup");
        }
    }

    // Checkpoint WAL before copying to ensure backup is consistent.
    // In WAL mode, committed transactions may reside in the -wal file
    // and won't be included in a plain file copy.
    // Hold the checkpoint connection alive through fs::copy to prevent
    // new WAL transactions from starting between checkpoint and copy.
    let _checkpoint_guard = Connection::open(db_path).ok();
    if let Some(ref conn) = _checkpoint_guard {
        // Set busy_timeout so the checkpoint retries on SQLITE_BUSY instead of
        // failing immediately when another connection has an active transaction.
        let _ = conn.execute_batch("PRAGMA busy_timeout = 5000;");
        // TRUNCATE mode: flush WAL into main DB and truncate WAL file
        if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
            tracing::warn!(error = %e, "WAL checkpoint before backup failed, proceeding with copy");
        }
    }

    // Copy current database to .bak
    let current_bak = backup_dir.join(format!("{file_name_str}.bak"));
    fs::copy(db_path, &current_bak).with_context(|| {
        format!(
            "failed to backup database from {} to {}",
            db_path.display(),
            current_bak.display()
        )
    })?;

    debug!(source = %db_path.display(), backup = %current_bak.display(), "database backed up");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let dir = env::temp_dir().join(format!(
            "vaultpilot-backup-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn regression_1212_backup_creates_first_backup() {
        let dir = unique_temp_dir("first");
        let db_path = dir.join("test.db");
        fs::write(&db_path, b"fake db content").unwrap();

        auto_backup_database(&db_path).unwrap();

        let bak = dir.join("test.db.bak");
        assert!(bak.exists());
        assert_eq!(fs::read(&bak).unwrap(), b"fake db content");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_1212_backup_rotates_backups() {
        let dir = unique_temp_dir("rotate");
        let db_path = dir.join("test.db");

        // Create initial db
        fs::write(&db_path, b"version 1").unwrap();
        auto_backup_database(&db_path).unwrap();

        // Update db and backup again
        fs::write(&db_path, b"version 2").unwrap();
        auto_backup_database(&db_path).unwrap();

        // Update db and backup third time
        fs::write(&db_path, b"version 3").unwrap();
        auto_backup_database(&db_path).unwrap();

        // Current backup should have latest content
        let bak = dir.join("test.db.bak");
        assert_eq!(fs::read(&bak).unwrap(), b"version 3");

        // .bak.1 should have previous content
        let bak1 = dir.join("test.db.bak.1");
        assert_eq!(fs::read(&bak1).unwrap(), b"version 2");

        // .bak.2 should have oldest content
        let bak2 = dir.join("test.db.bak.2");
        assert_eq!(fs::read(&bak2).unwrap(), b"version 1");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_1212_backup_noop_when_no_db() {
        let dir = unique_temp_dir("noop");
        let db_path = dir.join("nonexistent.db");

        // Should succeed (no-op) when db doesn't exist
        auto_backup_database(&db_path).unwrap();

        // No backup files should be created
        assert!(!dir.join("nonexistent.db.bak").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_1893_backup_fourth_run_keeps_three_historical() {
        let dir = unique_temp_dir("fourth");
        let db_path = dir.join("test.db");

        for i in 1..=4 {
            fs::write(&db_path, format!("version {i}")).unwrap();
            auto_backup_database(&db_path).unwrap();
        }

        // Should keep 3 historical backups (.bak.1, .bak.2, .bak.3) + current (.bak)
        let bak = dir.join("test.db.bak");
        let bak1 = dir.join("test.db.bak.1");
        let bak2 = dir.join("test.db.bak.2");
        let bak3 = dir.join("test.db.bak.3");

        assert_eq!(fs::read(&bak).unwrap(), b"version 4");
        assert_eq!(fs::read(&bak1).unwrap(), b"version 3");
        assert_eq!(fs::read(&bak2).unwrap(), b"version 2");
        assert_eq!(fs::read(&bak3).unwrap(), b"version 1");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn regression_1893_backup_fifth_run_rotates_oldest_out() {
        let dir = unique_temp_dir("fifth");
        let db_path = dir.join("test.db");

        for i in 1..=5 {
            fs::write(&db_path, format!("version {i}")).unwrap();
            auto_backup_database(&db_path).unwrap();
        }

        // After 5 runs, the oldest (version 1) should have been rotated out
        let bak = dir.join("test.db.bak");
        let bak1 = dir.join("test.db.bak.1");
        let bak2 = dir.join("test.db.bak.2");
        let bak3 = dir.join("test.db.bak.3");
        let bak4 = dir.join("test.db.bak.4");

        assert_eq!(fs::read(&bak).unwrap(), b"version 5");
        assert_eq!(fs::read(&bak1).unwrap(), b"version 4");
        assert_eq!(fs::read(&bak2).unwrap(), b"version 3");
        assert_eq!(fs::read(&bak3).unwrap(), b"version 2");
        assert!(!bak4.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
