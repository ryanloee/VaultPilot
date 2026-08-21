//! Vault-external crash-recovery snapshots — the "File Recovery" safety net (#3451).
//!
//! This is a **separate layer** from [`crate::storage::snapshots`] (the
//! modification-history / rollback layer). The two are intentionally distinct:
//!
//! | | `snapshots` (#2855) | `recovery` (this module, #3451) |
//! |---|---|---|
//! | **When** | on every `save_note` (captures the *old* version) | on a timer while editing (captures the *current unsaved buffer*) |
//! | **Retention** | by count (max 20 / note) | by time (default 7 days) |
//! | **Location** | inside the vault (`<vault>/.vaultpilot/knowledge-index.sqlite`) | **outside** the vault (OS data dir) — survives vault corruption/deletion |
//! | **Purpose** | browse/diff/rollback known edits | recover unsaved work after a crash |
//!
//! Storing recovery snapshots *outside* the vault (in the OS data directory,
//! namespaced per-vault) is the key property: if the vault DB or the vault
//! folder itself is corrupted or deleted, the recovery snapshots still exist
//! and can be restored — exactly mirroring Obsidian's File Recovery plugin.
//!
//! # Storage layout
//!
//! ```text
//! <data-root>/vaultpilot/recovery/<vault-namespace>/recovery.sqlite
//! ```
//!
//! Where `<data-root>` is `XDG_DATA_HOME` (or `~/.local/share`) on Linux,
//! `LOCALAPPDATA` on Windows, and `HOME/Library/Application Support` on macOS.
//! `<vault-namespace>` is a stable SHA-256 hash of the canonical vault path, so
//! each vault keeps its own isolated recovery store without leaking across
//! devices (the store is device-local, never synced).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Default retention window for recovery snapshots (days). Mirrors Obsidian's
/// File Recovery default of 7 days.
pub const DEFAULT_RECOVERY_RETENTION_DAYS: i64 = 7;

/// A single crash-recovery snapshot of an unsaved edit buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverySnapshot {
    /// Snapshot UUID (primary key).
    pub id: String,
    /// Vault-relative path of the note being edited (stable key — the buffer
    /// may not have a note ID yet if it was never saved).
    pub note_path: String,
    /// Best-effort note title for display in recovery UIs.
    pub title: String,
    /// The full edit-buffer content at snapshot time.
    pub content: String,
    /// Size of `content` in bytes (denormalised for cheap stats listing).
    pub content_size: i64,
    /// ISO-8601 timestamp when the snapshot was taken.
    pub created_at: String,
}

/// Resolve the OS-level data root used to keep recovery snapshots *outside* the
/// (synced, potentially corruptible) vault.
///
/// Order of preference:
/// 1. `XDG_DATA_HOME` (Linux freedesktop standard)
/// 2. `LOCALAPPDATA` (Windows per-user app data)
/// 3. `$HOME/.local/share` (Linux fallback)
/// 4. `$HOME/Library/Application Support` (macOS)
/// 5. temp dir (last resort, logged)
fn recovery_data_root() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        // Linux fallback.
        let p = home.join(".local").join("share");
        if p.is_dir() || create_dir_quiet(&p) {
            return p;
        }
        // macOS fallback.
        let p = home.join("Library").join("Application Support");
        if p.is_dir() || create_dir_quiet(&p) {
            return p;
        }
        // If neither is creatable, fall back to home itself.
        return home;
    }
    tracing::warn!(
        "XDG_DATA_HOME/LOCALAPPDATA/HOME all unset; recovery snapshots fall back to temp dir"
    );
    std::env::temp_dir()
}

fn create_dir_quiet(p: &Path) -> bool {
    fs::create_dir_all(p).is_ok()
}

/// Stable per-vault namespace (SHA-256 of the canonical vault path) so each
/// vault gets its own recovery store, isolated from other vaults on the same
/// machine. Uses SHA-256 (not `DefaultHasher`) for cross-release stability.
fn vault_namespace(vault_dir: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(vault_dir.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// Resolve the recovery directory for a given vault.
///
/// Returns `<data-root>/vaultpilot/recovery/<vault-namespace>`. The directory
/// is created if it does not yet exist.
pub fn recovery_dir_for_vault(vault_dir: &Path) -> Result<PathBuf> {
    // Canonicalise so that different path spellings of the same vault map to
    // the same namespace. Fall back to the lexical path if the dir doesn't
    // exist yet (the vault may be brand-new).
    let canonical = vault_dir
        .canonicalize()
        .unwrap_or_else(|_| vault_dir.to_path_buf());
    let dir = recovery_data_root()
        .join("vaultpilot")
        .join("recovery")
        .join(vault_namespace(&canonical));
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create recovery dir {}", dir.display()))?;
    Ok(dir)
}

/// Open (and lazily migrate) the per-vault recovery SQLite database.
///
/// The DB lives at `<recovery-dir>/recovery.sqlite`, deliberately separate from
/// the vault's `knowledge-index.sqlite` so that vault DB corruption cannot take
/// the recovery store down with it.
fn open_recovery_db(vault_dir: &Path) -> Result<Connection> {
    let dir = recovery_dir_for_vault(vault_dir)?;
    let db_path = dir.join("recovery.sqlite");
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open recovery db {}", db_path.display()))?;
    // Lightweight pragmas for robustness under concurrent editor timers.
    conn.execute_batch(
        "PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
    )?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS recovery_snapshots (
            id           TEXT PRIMARY KEY,
            note_path    TEXT NOT NULL,
            title        TEXT NOT NULL DEFAULT '',
            content      TEXT NOT NULL,
            content_size INTEGER NOT NULL DEFAULT 0,
            created_at   TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_recovery_path    ON recovery_snapshots(note_path);
        CREATE INDEX IF NOT EXISTS idx_recovery_created ON recovery_snapshots(created_at);
        "#,
    )?;
    Ok(conn)
}

/// Validate that a recovery snapshot's `note_path` is a safe vault-relative
/// path.
///
/// Recovery snapshots are stored in a SQLite DB *outside* the vault, so
/// `note_path` must never be trusted when it crosses back into the vault: an
/// absolute path or `..` traversal would otherwise let a tampered DB (or a
/// buggy capture) write arbitrary files anywhere on disk on restore (#3984).
///
/// This is the API-boundary check enforced by [`save_recovery_snapshot`] and
/// re-checked (alongside symlink resolution) by [`recovery_target_path`]
/// before any restore write. Rejects:
/// - empty paths;
/// - absolute paths (`/etc/...`, `\etc\...` when rooted on Windows,
///   `C:\...` / `C:...` drive forms, UNC `\\...`);
/// - any `..` component (`../../etc/pwned`, `a/../b.md`).
pub fn validate_recovery_note_path(note_path: &str) -> Result<()> {
    // Normalize Windows separators first so `\..\etc` is caught even on a
    // Unix host, and a root-relative `\etc` is caught as absolute.
    let normalized = note_path.replace('\\', "/");
    let bytes = normalized.as_bytes();
    if normalized.is_empty() {
        bail!("recovery note_path must not be empty");
    }
    if normalized.starts_with('/') || Path::new(note_path).is_absolute() {
        bail!("recovery note_path must be vault-relative, got absolute path: '{note_path}'");
    }
    // Windows drive-letter prefixes (`C:\...`, `C:...`) are absolute or
    // drive-relative on Windows; never valid vault-relative note paths.
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        bail!("recovery note_path must be vault-relative, got Windows-style path: '{note_path}'");
    }
    for component in normalized.split('/') {
        if component == ".." {
            bail!("recovery note_path must not contain '..' components: '{note_path}'");
        }
    }
    Ok(())
}

/// Resolve the on-disk destination for restoring a recovery snapshot,
/// refusing anything that lands outside the vault (#3984).
///
/// The recovery DB is stored outside the vault, so a snapshot's `note_path`
/// is treated as untrusted even though [`save_recovery_snapshot`] now
/// validates it at save time: a tampered DB, or snapshots written by older
/// builds, can still contain arbitrary paths. This performs:
///   1. the lexical validation from [`validate_recovery_note_path`];
///   2. a lexical containment re-check of the joined target;
///   3. a symlink-resolving containment check — the deepest existing
///      ancestor of the target is canonicalized and must still live under
///      the canonical vault dir, so a subdirectory that is a symlink
///      pointing outside the vault is refused too.
///
/// The caller may then `create_dir_all(target.parent())` and write; the path
/// returned is always inside the vault.
pub fn recovery_target_path(vault_dir: &Path, note_path: &str) -> Result<PathBuf> {
    validate_recovery_note_path(note_path)?;

    // Drop interior `//` and `.` components lexically (`a//b`, `a/./b` →
    // `a/b`) so the path we join is exactly the path we check. `..` is
    // already rejected above.
    let normalized = note_path
        .replace('\\', "/")
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect::<Vec<_>>()
        .join("/");
    if normalized.is_empty() {
        bail!("recovery note_path '{note_path}' does not name a file inside the vault");
    }
    let target = vault_dir.join(&normalized);
    if !target.starts_with(vault_dir) {
        bail!(
            "recovery note_path '{note_path}' escapes the vault (target {})",
            target.display()
        );
    }

    // Walk every component from the canonical vault root down to the target,
    // resolving symlinks — including dangling ones — at each step. The old
    // approach only canonicalized the deepest *existing* ancestor, so a
    // dangling symlink at the tail of the path (e.g. `sub -> /outside/x`
    // where `/outside/x` does not exist yet) was skipped: `canonicalize`
    // returned NotFound, the check fell back to the vault root, and the
    // caller's `fs::write` then followed the link and wrote outside the
    // vault (#4002).
    let vault_canon = vault_dir
        .canonicalize()
        .unwrap_or_else(|_| lexical_normalize(vault_dir));
    let mut current = vault_canon.clone();
    for component in normalized.split('/') {
        current.push(component);
        let meta = match std::fs::symlink_metadata(&current) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Nothing exists at (or below) this component, so there is no
                // symlink left to resolve; the lexical check above stands.
                break;
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!(
                        "failed to inspect recovery target component {}",
                        current.display()
                    )
                });
            }
        };
        if meta.file_type().is_symlink() {
            let resolved = resolve_symlink_chain(&current)?;
            let resolved_norm = lexical_normalize(&resolved);
            if !resolved_norm.starts_with(&vault_canon) {
                bail!(
                    "recovery note_path '{note_path}' escapes the vault via symlink \
                     ({} resolves to {}, outside {})",
                    current.display(),
                    resolved_norm.display(),
                    vault_canon.display()
                );
            }
            current = resolved_norm;
        } else if let Ok(canon) = current.canonicalize() {
            current = canon;
        }
    }

    Ok(target)
}

/// Lexically normalize a path: drop `.` and empty components, resolve `..`
/// against the current prefix without touching the filesystem.
fn lexical_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Refuse to pop above the root (e.g. `C:\..` stays `C:\`).
                if out.file_name().is_some() {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Follow a symlink chain (including dangling links) and return the final
/// lexical target. Absolute link targets are used as-is; relative targets
/// are joined against the link's parent directory first.
fn resolve_symlink_chain(path: &Path) -> Result<PathBuf> {
    const MAX_DEPTH: usize = 32;
    let mut current = path.to_path_buf();
    for _ in 0..MAX_DEPTH {
        let meta = match std::fs::symlink_metadata(&current) {
            Ok(meta) => meta,
            // The chain ends at a path that does not exist yet (dangling
            // link target); that is fine as long as every symlink we walked
            // resolved inside the vault.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(current),
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("failed to stat symlink candidate {}", current.display())
                });
            }
        };
        if !meta.file_type().is_symlink() {
            return Ok(current);
        }
        let link = std::fs::read_link(&current)
            .with_context(|| format!("failed to read symlink {}", current.display()))?;
        current = if link.is_absolute() {
            link
        } else {
            let parent = current.parent().unwrap_or_else(|| Path::new(""));
            parent.join(link)
        };
        current = lexical_normalize(&current);
    }
    bail!("symlink chain too deep at {}", path.display())
}

/// Save the current unsaved edit buffer as a recovery snapshot.
///
/// `note_path` should be vault-relative (e.g. `inbox/draft.md`). `title` is a
/// best-effort display title. Returns the created snapshot.
#[allow(clippy::missing_panics_doc)]
pub fn save_recovery_snapshot(
    vault_dir: &Path,
    note_path: &str,
    title: &str,
    content: &str,
) -> Result<RecoverySnapshot> {
    // `note_path` is stored in a DB outside the vault and later written back
    // into the vault on restore — never accept a path that could land outside
    // the vault (#3984).
    validate_recovery_note_path(note_path)?;
    let conn = open_recovery_db(vault_dir)?;
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let size = content.len() as i64;
    conn.execute(
        "INSERT INTO recovery_snapshots (id, note_path, title, content, content_size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, note_path, title, content, size, now],
    )?;
    Ok(RecoverySnapshot {
        id,
        note_path: note_path.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        content_size: size,
        created_at: now,
    })
}

/// List recovery snapshots, optionally filtered to a single note path.
///
/// Results are ordered newest-first so the most recently auto-saved buffer
/// appears at the top (matching the recovery-prompt UX where you want the
/// latest crash-time state).
pub fn list_recovery_snapshots(
    vault_dir: &Path,
    note_path: Option<&str>,
) -> Result<Vec<RecoverySnapshot>> {
    let conn = open_recovery_db(vault_dir)?;
    let mut stmt = conn.prepare(
        "SELECT id, note_path, title, content, content_size, created_at
         FROM recovery_snapshots
         WHERE (?1 IS NULL OR note_path = ?1)
         ORDER BY created_at DESC",
    )?;
    let rows = stmt
        .query_map(params![note_path], row_to_snapshot)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_to_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecoverySnapshot> {
    Ok(RecoverySnapshot {
        id: row.get(0)?,
        note_path: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        content_size: row.get(4)?,
        created_at: row.get(5)?,
    })
}

/// Fetch a single recovery snapshot by ID.
pub fn get_recovery_snapshot(vault_dir: &Path, id: &str) -> Result<Option<RecoverySnapshot>> {
    let conn = open_recovery_db(vault_dir)?;
    let mut stmt = conn.prepare(
        "SELECT id, note_path, title, content, content_size, created_at
         FROM recovery_snapshots WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], row_to_snapshot)?;
    match rows.next() {
        Some(r) => Ok(Some(r?)),
        None => Ok(None),
    }
}

/// Delete a single recovery snapshot by ID. Returns `true` if a row was removed.
pub fn delete_recovery_snapshot(vault_dir: &Path, id: &str) -> Result<bool> {
    let conn = open_recovery_db(vault_dir)?;
    let removed = conn.execute("DELETE FROM recovery_snapshots WHERE id = ?1", params![id])?;
    Ok(removed > 0)
}

/// Delete all recovery snapshots older than `retention_days`. Returns the
/// number of rows removed.
///
/// With the default [`DEFAULT_RECOVERY_RETENTION_DAYS`] this trims the store to
/// the last 7 days, implementing the time-based retention policy (as opposed to
/// the count-based retention used by the modification-history layer).
pub fn cleanup_expired(vault_dir: &Path, retention_days: i64) -> Result<usize> {
    let conn = open_recovery_db(vault_dir)?;
    let cutoff = (Utc::now() - Duration::days(retention_days)).to_rfc3339();
    let removed = conn.execute(
        "DELETE FROM recovery_snapshots WHERE created_at < ?1",
        params![cutoff],
    )?;
    Ok(removed)
}

/// Count the total number of recovery snapshots for a vault. Used by the
/// crash-recovery prompt on app startup ("Detected N recovery points").
pub fn count_recovery_snapshots(vault_dir: &Path) -> Result<usize> {
    let conn = open_recovery_db(vault_dir)?;
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM recovery_snapshots", [], |row| {
        row.get(0)
    })?;
    Ok(n as usize)
}

/// Count recovery snapshots created since a given instant — used to detect
/// crash-time buffers that may not have been flushed to the vault.
pub fn count_since(vault_dir: &Path, since: &DateTime<Utc>) -> Result<usize> {
    let conn = open_recovery_db(vault_dir)?;
    let cutoff = since.to_rfc3339();
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM recovery_snapshots WHERE created_at >= ?1",
        params![cutoff],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a unique temp-backed "vault dir" for an isolated test. Uses PID +
    /// UUID so parallel `cargo test` invocations never collide (lesson from the
    /// #2512 / mail flaky-test incident).
    fn test_vault() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-recovery-test-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_and_get_snapshot_round_trip() {
        let vault = test_vault();
        let snap = save_recovery_snapshot(
            &vault,
            "inbox/draft.md",
            "My Draft",
            "# Hello\nunsaved buffer content",
        )
        .expect("save");
        assert!(!snap.id.is_empty());
        assert_eq!(snap.note_path, "inbox/draft.md");
        assert_eq!(snap.title, "My Draft");
        assert_eq!(snap.content, "# Hello\nunsaved buffer content");
        assert!(snap.content_size > 0);

        let fetched = get_recovery_snapshot(&vault, &snap.id)
            .expect("get")
            .expect("snapshot should exist");
        assert_eq!(fetched.content, snap.content);
        assert_eq!(fetched.note_path, snap.note_path);
    }

    #[test]
    fn list_orders_newest_first_and_filters_by_path() {
        let vault = test_vault();
        let s1 = save_recovery_snapshot(&vault, "a.md", "A", "v1").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let s2 = save_recovery_snapshot(&vault, "a.md", "A", "v2").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let _s3 = save_recovery_snapshot(&vault, "b.md", "B", "other").unwrap();

        // All snapshots, newest first.
        let all = list_recovery_snapshots(&vault, None).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].note_path, "b.md"); // most recent

        // Filtered to a.md, newest first.
        let a_only = list_recovery_snapshots(&vault, Some("a.md")).unwrap();
        assert_eq!(a_only.len(), 2);
        assert_eq!(a_only[0].id, s2.id); // v2 newer than v1
        assert_eq!(a_only[1].id, s1.id);
    }

    #[test]
    fn delete_removes_only_target() {
        let vault = test_vault();
        let s1 = save_recovery_snapshot(&vault, "a.md", "A", "v1").unwrap();
        let s2 = save_recovery_snapshot(&vault, "a.md", "A", "v2").unwrap();

        let removed = delete_recovery_snapshot(&vault, &s1.id).unwrap();
        assert!(removed);

        assert!(get_recovery_snapshot(&vault, &s1.id).unwrap().is_none());
        assert!(get_recovery_snapshot(&vault, &s2.id).unwrap().is_some());

        // Deleting a non-existent id returns false.
        let again = delete_recovery_snapshot(&vault, &s1.id).unwrap();
        assert!(!again);
    }

    #[test]
    fn cleanup_expired_respects_retention_window() {
        let vault = test_vault();
        // A fresh snapshot is within the 7-day window → not purged.
        let fresh = save_recovery_snapshot(&vault, "a.md", "A", "fresh").unwrap();

        let removed = cleanup_expired(&vault, DEFAULT_RECOVERY_RETENTION_DAYS).unwrap();
        assert_eq!(removed, 0);
        assert!(get_recovery_snapshot(&vault, &fresh.id).unwrap().is_some());

        // A retention window of 0 days purges everything older than "now".
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let removed_zero = cleanup_expired(&vault, 0).unwrap();
        assert!(removed_zero >= 1);
        assert!(get_recovery_snapshot(&vault, &fresh.id).unwrap().is_none());
    }

    #[test]
    fn count_helpers_are_consistent() {
        let vault = test_vault();
        save_recovery_snapshot(&vault, "a.md", "A", "1").unwrap();
        save_recovery_snapshot(&vault, "b.md", "B", "2").unwrap();

        assert_eq!(count_recovery_snapshots(&vault).unwrap(), 2);

        let now = Utc::now();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        save_recovery_snapshot(&vault, "c.md", "C", "3").unwrap();
        assert_eq!(count_since(&vault, &now).unwrap(), 1);
    }

    #[test]
    fn recovery_store_is_outside_the_vault() {
        // The whole point of File Recovery: the store must NOT live inside the
        // vault directory (so vault corruption/deletion can't destroy it).
        let vault = test_vault();
        save_recovery_snapshot(&vault, "a.md", "A", "content").unwrap();

        let recovery_dir = recovery_dir_for_vault(&vault).unwrap();
        let db = recovery_dir.join("recovery.sqlite");
        assert!(db.exists(), "recovery db should exist");

        // The recovery dir must not be a subpath of the vault dir.
        assert!(
            !recovery_dir.starts_with(&vault),
            "recovery store leaked into the vault: {} is under {}",
            recovery_dir.display(),
            vault.display()
        );
    }

    #[test]
    fn separate_vaults_get_isolated_stores() {
        let vault_a = test_vault();
        let vault_b = test_vault();
        save_recovery_snapshot(&vault_a, "a.md", "A", "from A").unwrap();
        save_recovery_snapshot(&vault_b, "a.md", "A", "from B").unwrap();

        // Vault A sees only its own snapshot.
        let a_snaps = list_recovery_snapshots(&vault_a, None).unwrap();
        assert_eq!(a_snaps.len(), 1);
        assert_eq!(a_snaps[0].content, "from A");

        let b_snaps = list_recovery_snapshots(&vault_b, None).unwrap();
        assert_eq!(b_snaps.len(), 1);
        assert_eq!(b_snaps[0].content, "from B");
    }

    #[test]
    fn unicode_content_round_trips() {
        // Guard against the #2512 UTF-8 byte-index class of bug: multi-byte
        // content must survive the round trip intact.
        let vault = test_vault();
        let body = "# 标题 🚀\n\n多字节内容 — emoji ✓";
        let snap = save_recovery_snapshot(&vault, "笔记/草稿.md", "标题", body).unwrap();
        let fetched = get_recovery_snapshot(&vault, &snap.id).unwrap().unwrap();
        assert_eq!(fetched.content, body);
        assert_eq!(fetched.note_path, "笔记/草稿.md");
        assert_eq!(fetched.title, "标题");
    }

    #[test]
    fn save_rejects_absolute_note_paths() {
        // #3984: recovery snapshots live outside the vault; an absolute
        // note_path must never be accepted (restore would write outside).
        let vault = test_vault();
        for bad in [
            "/etc/pwned",
            "\\etc\\pwned",
            "C:\\pwned",
            "C:/pwned",
            "\\\\server\\share",
        ] {
            let err = save_recovery_snapshot(&vault, bad, "t", "x")
                .expect_err("absolute note_path must be rejected");
            assert!(
                err.to_string().contains("vault-relative"),
                "unexpected error for {bad:?}: {err}"
            );
        }
        // Nothing leaked into the store.
        assert!(list_recovery_snapshots(&vault, None).unwrap().is_empty());
    }

    #[test]
    fn save_rejects_parent_traversal_note_paths() {
        // #3984: `..` components could walk out of the vault on restore.
        let vault = test_vault();
        for bad in [
            "../../escape.md",
            "a/../../escape.md",
            "..\\escape.md",
            "a/..\\b.md",
            "a/../b.md",
        ] {
            let err = save_recovery_snapshot(&vault, bad, "t", "x")
                .expect_err("traversal note_path must be rejected");
            assert!(
                err.to_string().contains("'..'"),
                "unexpected error for {bad:?}: {err}"
            );
        }
        // Normal paths still work after the rejections.
        let snap = save_recovery_snapshot(&vault, "inbox/draft.md", "t", "x").unwrap();
        let fetched = get_recovery_snapshot(&vault, &snap.id).unwrap().unwrap();
        assert_eq!(fetched.note_path, "inbox/draft.md");
        assert_eq!(list_recovery_snapshots(&vault, None).unwrap().len(), 1);
    }

    #[test]
    fn restore_target_stays_inside_the_vault() {
        // #3984: recovery_target_path is the restore-time boundary (used by
        // `recovery_restore_async` in the agent binary).
        let vault = test_vault();

        // Valid relative paths resolve to a location inside the vault.
        let target = recovery_target_path(&vault, "notes/inbox/draft.md").unwrap();
        assert_eq!(target, vault.join("notes/inbox/draft.md"));
        assert!(target.starts_with(&vault));

        // Escapes are refused outright.
        for bad in [
            "../../../etc/pwned",
            "/etc/pwned",
            "C:\\Windows\\pwned",
            "x/../pwned",
            "x/./../pwned",
        ] {
            let err =
                recovery_target_path(&vault, bad).expect_err("escaping note_path must be refused");
            assert!(
                err.to_string().contains("vault") || err.to_string().contains("'..'"),
                "unexpected error for {bad:?}: {err}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn restore_target_refuses_symlinked_subdir_leaving_vault() {
        // A subdirectory inside the vault that is a symlink to an outside
        // directory must not be writeable through a snapshot restore.
        use std::os::unix::fs::symlink;
        let vault = test_vault();
        let outside =
            std::env::temp_dir().join(format!("vaultpilot-recovery-outside-{}", Uuid::new_v4()));
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, vault.join("leak")).unwrap();

        let err = recovery_target_path(&vault, "leak/pwned.md")
            .expect_err("symlink escape must be refused");
        assert!(
            err.to_string().contains("symlink"),
            "unexpected error: {err}"
        );

        fs::remove_file(vault.join("leak")).ok();
        fs::remove_dir_all(&outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn restore_target_refuses_dangling_symlink_escape() {
        // #4002: canonicalize() returns NotFound for a dangling symlink, so
        // the old deepest-existing-ancestor check skipped it and the restore
        // write followed the link outside the vault. Every component must be
        // resolved, including links whose target does not exist yet.
        use std::os::unix::fs::symlink;
        let vault = test_vault();
        let outside =
            std::env::temp_dir().join(format!("vaultpilot-recovery-dangling-{}", Uuid::new_v4()));
        fs::create_dir_all(&outside).unwrap();

        // Final component is a dangling link pointing outside the vault.
        symlink(outside.join("missing-dir"), vault.join("leak")).unwrap();
        let err = recovery_target_path(&vault, "leak")
            .expect_err("dangling symlink escape must be refused");
        assert!(
            err.to_string().contains("symlink"),
            "unexpected error: {err}"
        );

        // Intermediate component is a dangling link pointing outside.
        symlink(outside.join("also-missing"), vault.join("leak2")).unwrap();
        let err = recovery_target_path(&vault, "leak2/pwned.md")
            .expect_err("dangling intermediate symlink escape must be refused");
        assert!(
            err.to_string().contains("symlink"),
            "unexpected error: {err}"
        );

        // A dangling link whose target stays inside the vault is still valid.
        symlink(vault.join("future-dir"), vault.join("alias")).unwrap();
        let target = recovery_target_path(&vault, "alias/note.md").unwrap();
        assert!(target.starts_with(&vault));

        fs::remove_file(vault.join("leak")).ok();
        fs::remove_file(vault.join("leak2")).ok();
        fs::remove_file(vault.join("alias")).ok();
        fs::remove_dir_all(&outside).ok();
    }
}
