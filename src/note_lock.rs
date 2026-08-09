//! Per-note password locks (#3977).
//!
//! Locks are stored as small JSON files under `<vault>/.vaultpilot/locks/`
//! (one per note id). The password is never persisted: only a PHC-style
//! PBKDF2-HMAC-SHA256 hash (the same scheme as App Lock, #3304/#3323) is
//! stored. Locked notes still show metadata, but their body is masked by the
//! CLI until the user unlocks them with the correct password.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::AppSettings;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NoteLockEntry {
    /// PHC-style PBKDF2 hash of the note password (never plaintext).
    password_hash: String,
    /// When the lock was applied (RFC3339).
    locked_at: String,
}

fn locks_dir(vault_dir: &Path) -> PathBuf {
    vault_dir.join(".vaultpilot").join("locks")
}

fn lock_path(vault_dir: &Path, note_id: &str) -> PathBuf {
    // Note ids are already sanitized to alnum/'-'/'_' at save time, but keep
    // the path safe even for legacy ids that may contain other characters.
    let safe: String = note_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    locks_dir(vault_dir).join(format!("{safe}.json"))
}

/// Lock a note with a password. Overwrites any previous lock for that note.
pub fn lock_note(vault_dir: &Path, note_id: &str, password: &str) -> Result<()> {
    if password.trim().is_empty() {
        anyhow::bail!("note lock password must not be empty");
    }
    let path = lock_path(vault_dir, note_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("create note-lock directory {}", parent.display())
        })?;
    }
    let entry = NoteLockEntry {
        password_hash: AppSettings::hash_pin(password),
        locked_at: chrono::Utc::now().to_rfc3339(),
    };
    let bytes = serde_json::to_vec_pretty(&entry)?;
    fs::write(&path, bytes)
        .with_context(|| format!("write note lock {}", path.display()))?;
    Ok(())
}

/// Unlock a note with its password. Returns `true` when a lock was removed,
/// `false` when the note was not locked. Wrong passwords are an error.
pub fn unlock_note(vault_dir: &Path, note_id: &str, password: &str) -> Result<bool> {
    let path = lock_path(vault_dir, note_id);
    if !path.exists() {
        return Ok(false);
    }
    let bytes = fs::read(&path)
        .with_context(|| format!("read note lock {}", path.display()))?;
    let entry: NoteLockEntry = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse note lock {}", path.display()))?;
    if !AppSettings::verify_password_hash(&entry.password_hash, password) {
        anyhow::bail!("incorrect password for note '{note_id}'");
    }
    fs::remove_file(&path).with_context(|| format!("remove note lock {}", path.display()))?;
    Ok(true)
}

/// Whether a note is currently locked.
pub fn is_locked(vault_dir: &Path, note_id: &str) -> bool {
    lock_path(vault_dir, note_id).exists()
}

/// Body placeholder returned for locked notes (#3977).
pub const MASKED_BODY: &str =
    "🔒 This note is locked. Unlock it with `vp notes unlock <id> --password <password>`.";

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_vault() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vp_note_lock_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create vault");
        dir
    }

    #[test]
    fn lock_unlock_round_trip() {
        let vault = temp_vault();
        assert!(!is_locked(&vault, "n1"));

        lock_note(&vault, "n1", "s3cret").expect("lock");
        assert!(is_locked(&vault, "n1"));
        assert!(lock_path(&vault, "n1").exists());

        // Wrong password must not unlock.
        let err = unlock_note(&vault, "n1", "wrong").unwrap_err();
        assert!(err.to_string().contains("incorrect password"));
        assert!(is_locked(&vault, "n1"));

        assert!(unlock_note(&vault, "n1", "s3cret").expect("unlock"));
        assert!(!is_locked(&vault, "n1"));
        // Unlocking an unlocked note reports false.
        assert!(!unlock_note(&vault, "n1", "s3cret").expect("unlock again"));

        fs::remove_dir_all(&vault).ok();
    }

    #[test]
    fn empty_password_is_rejected() {
        let vault = temp_vault();
        assert!(lock_note(&vault, "n1", "  ").is_err());
        fs::remove_dir_all(&vault).ok();
    }
}
