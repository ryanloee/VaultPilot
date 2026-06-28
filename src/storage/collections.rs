//! Collection operations — associating notes with collections.
//!
//! A "collection" is a logical grouping of notes (e.g. tags, folders,
//! playlists).  The `note_collections` table uses a composite primary key
//! `(note_id, collection_id)` to model a many-to-many relationship.
//!
//! # Concurrency safety
//!
//! All insert operations use `INSERT OR IGNORE` to avoid TOCTOU race
//! conditions instead of a separate `SELECT EXISTS` + `INSERT` pattern.
//! The caller checks `connection.changes()` to determine whether a new
//! row was actually inserted.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::params;

use super::pool::{open_connection, StorageContext};

/// Add a note to a collection.
///
/// Returns `Ok(true)` if the association was newly created, or `Ok(false)`
/// if it already existed (no-op).
///
/// Uses `INSERT OR IGNORE` to eliminate the TOCTOU race between checking
/// existence and inserting — two concurrent calls for the same
/// `(note_id, collection_id)` will both succeed silently, with one being
/// a no-op.
pub fn add_note_to_collection_with_context(
    context: &StorageContext,
    note_id: &str,
    collection_id: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();

    let changes = connection.execute(
        "INSERT OR IGNORE INTO note_collections (note_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
        params![note_id, collection_id, now],
    ).with_context(|| format!(
        "failed to insert note {note_id} into collection {collection_id}"
    ))?;

    Ok(changes > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::pool::ensure_schema;
    use rusqlite::Connection as RusqliteConnection;

    /// Helper: create an in-memory database with the full schema.
    fn setup_db() -> RusqliteConnection {
        let conn = RusqliteConnection::open_in_memory()
            .expect("failed to open in-memory SQLite");
        ensure_schema(&conn).expect("schema creation");
        conn
    }

    #[test]
    fn add_note_to_collection_inserts_new_row() {
        let conn = setup_db();

        // Insert a note so the FK constraint is satisfied
        conn.execute(
            "INSERT INTO notes (id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary, body_hash)
             VALUES ('note-1', 'Test', '', '', '', '', '', '', '2025-01-01T00:00:00Z', '2025-01-01T00:00:00Z', 'manual', '/tmp/test.md', '', 'hash')",
            [],
        ).expect("insert note");

        // We can't easily test via the public function because it needs a
        // StorageContext with a real pool, so test the SQL logic directly.
        let changes = conn.execute(
            "INSERT OR IGNORE INTO note_collections (note_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
            params!["note-1", "collection-a", "2025-01-01T00:00:00Z"],
        ).expect("first insert");
        assert_eq!(changes, 1, "first insert should affect 1 row");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM note_collections WHERE note_id = ?1 AND collection_id = ?2",
                params!["note-1", "collection-a"],
                |row| row.get(0),
            )
            .expect("count rows");
        assert_eq!(count, 1, "exactly one row should exist");
    }

    #[test]
    fn add_note_to_collection_idempotent() {
        let conn = setup_db();

        conn.execute(
            "INSERT INTO notes (id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary, body_hash)
             VALUES ('note-1', 'Test', '', '', '', '', '', '', '2025-01-01T00:00:00Z', '2025-01-01T00:00:00Z', 'manual', '/tmp/test.md', '', 'hash')",
            [],
        ).expect("insert note");

        // First insert
        let changes1 = conn.execute(
            "INSERT OR IGNORE INTO note_collections (note_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
            params!["note-1", "collection-a", "2025-01-01T00:00:00Z"],
        ).expect("first insert");
        assert_eq!(changes1, 1, "first insert should affect 1 row");

        // Second insert — should be ignored (no error, 0 changes)
        let changes2 = conn.execute(
            "INSERT OR IGNORE INTO note_collections (note_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
            params!["note-1", "collection-a", "2025-01-01T00:00:00Z"],
        ).expect("second insert");
        assert_eq!(changes2, 0, "duplicate insert should affect 0 rows");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM note_collections WHERE note_id = ?1 AND collection_id = ?2",
                params!["note-1", "collection-a"],
                |row| row.get(0),
            )
            .expect("count rows");
        assert_eq!(count, 1, "still exactly one row after duplicate insert");
    }

    #[test]
    fn concurrent_inserts_do_not_conflict() {
        let conn = setup_db();

        conn.execute(
            "INSERT INTO notes (id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary, body_hash)
             VALUES ('note-1', 'Test', '', '', '', '', '', '', '2025-01-01T00:00:00Z', '2025-01-01T00:00:00Z', 'manual', '/tmp/test.md', '', 'hash')",
            [],
        ).expect("insert note");

        // Simulate two concurrent inserts — the second one should not fail
        // with a UNIQUE constraint violation.
        let changes1 = conn.execute(
            "INSERT OR IGNORE INTO note_collections (note_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
            params!["note-1", "collection-b", "2025-01-01T00:00:00Z"],
        ).expect("insert A");
        assert_eq!(changes1, 1, "insert A should succeed");

        let changes2 = conn.execute(
            "INSERT OR IGNORE INTO note_collections (note_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
            params!["note-1", "collection-b", "2025-01-01T00:00:00Z"],
        ).expect("insert B (should not fail)");
        assert_eq!(changes2, 0, "insert B should be a no-op");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM note_collections WHERE note_id = ?1 AND collection_id = ?2",
                params!["note-1", "collection-b"],
                |row| row.get(0),
            )
            .expect("count rows");
        assert_eq!(count, 1, "only one row after 'concurrent' inserts");
    }
}
