//! Persistent note snapshots — Agent/modification history with cross-session rollback (#2855).
//!
//! Every time a note is saved via `save_note_with_images_with_context`, the *old*
//! version is captured into the `note_snapshots` table before the update. CLI
//! commands (`history`, `restore`, `diff`) allow users to browse, compare, and
//! roll back to any snapshot.
//!
//! Retention: max 20 snapshots per note (oldest pruned automatically).

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use crate::models::NoteDocument;

use super::notes::load_note_with_context;
use super::pool::open_connection;
use super::StorageContext;

/// Maximum number of snapshots retained per note.
const MAX_SNAPSHOTS_PER_NOTE: usize = 20;

/// A single snapshot of a note at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSnapshot {
    /// Snapshot UUID.
    pub id: String,
    /// Note ID this snapshot belongs to.
    pub note_id: String,
    /// The full Markdown body at snapshot time.
    pub body: String,
    /// Serialized frontmatter / note metadata as JSON.
    pub frontmatter: String,
    /// Origin of the modification: "agent", "user", or "sync".
    pub source: String,
    /// ISO-8601 timestamp when the snapshot was taken.
    pub created_at: String,
}

/// Record a snapshot of the note *before* it is modified.
///
/// Loads the current note from the database, serialises its state, and inserts
/// a new row into `note_snapshots`. If the note does not yet exist (first
/// create) no snapshot is taken — there is nothing to capture yet.
///
/// After inserting, old snapshots beyond `MAX_SNAPSHOTS_PER_NOTE` are pruned
/// so the table does not grow unboundedly.
#[instrument(skip(context))]
pub fn record_snapshot_before_save(
    context: &StorageContext,
    note_id: &str,
    source: &str,
) -> Result<()> {
    // Don't snapshot brand-new notes (no prior state to capture).
    if note_id.is_empty() {
        return Ok(());
    }

    // Try to load the *current* version of the note (before the save overwrites it).
    let current = match load_note_with_context(context, note_id) {
        Ok(note) => note,
        Err(e) => {
            // If the note doesn't exist yet (first save), that's fine — nothing to snapshot.
            if e.downcast_ref::<super::notes::NoteNotFound>().is_some() {
                return Ok(());
            }
            return Err(e);
        }
    };

    let snapshot_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let frontmatter_json = serialize_note_meta(&current.meta)?;

    let (connection, _) = open_connection(context)?;

    connection.execute(
        "INSERT INTO note_snapshots (id, note_id, body, frontmatter, source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            snapshot_id,
            note_id,
            current.body,
            frontmatter_json,
            source,
            now,
        ],
    )?;

    // Prune old snapshots beyond retention limit.
    prune_old_snapshots(&connection, note_id)?;

    Ok(())
}

/// List all snapshots for a given note, newest first.
#[instrument(skip(context))]
pub fn list_snapshots_for_note(
    context: &StorageContext,
    note_id: &str,
) -> Result<Vec<NoteSnapshot>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        "SELECT id, note_id, body, frontmatter, source, created_at
         FROM note_snapshots
         WHERE note_id = ?1
         ORDER BY created_at DESC
         LIMIT 200",
    )?;

    let snapshots = stmt
        .query_map(params![note_id], |row| {
            Ok(NoteSnapshot {
                id: row.get(0)?,
                note_id: row.get(1)?,
                body: row.get(2)?,
                frontmatter: row.get(3)?,
                source: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .with_context(|| format!("failed to list snapshots for note '{note_id}'"))?;

    Ok(snapshots)
}

/// Retrieve a single snapshot by its ID.
#[instrument(skip(context))]
pub fn get_snapshot(context: &StorageContext, snapshot_id: &str) -> Result<Option<NoteSnapshot>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        "SELECT id, note_id, body, frontmatter, source, created_at
         FROM note_snapshots
         WHERE id = ?1",
    )?;

    let snapshot = stmt
        .query_row(params![snapshot_id], |row| {
            Ok(NoteSnapshot {
                id: row.get(0)?,
                note_id: row.get(1)?,
                body: row.get(2)?,
                frontmatter: row.get(3)?,
                source: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .optional()?;

    Ok(snapshot)
}

/// Restore a note to the state captured in a snapshot.
///
/// Loads the snapshot, overwrites the note file and database entry, and returns
/// the restored `NoteDocument`. A restore also creates a **new** snapshot of
/// the pre-restore state so the operation itself is reversible.
#[instrument(skip(context))]
pub fn restore_snapshot(
    context: &StorageContext,
    note_id: &str,
    snapshot_id: &str,
) -> Result<NoteDocument> {
    let snapshot = get_snapshot(context, snapshot_id)?
        .ok_or_else(|| anyhow::anyhow!("snapshot '{snapshot_id}' not found"))?;

    // Verify the snapshot belongs to the requested note.
    if snapshot.note_id != note_id {
        anyhow::bail!(
            "snapshot '{snapshot_id}' belongs to note '{}', not '{note_id}'",
            snapshot.note_id
        );
    }

    // Deserialise the frontmatter and reconstruct the NoteDocument.
    let mut restored: NoteDocument = serde_json::from_str(&snapshot.frontmatter)
        .with_context(|| "failed to deserialise snapshot frontmatter")?;
    restored.body = snapshot.body;

    // Record a snapshot of the current state *before* overwriting, so the
    // restore is reversible.
    record_snapshot_before_save(context, note_id, "user")?;

    // Save the restored note — this will index it into the DB and FTS.
    let saved = super::notes::save_note_with_images_with_context(context, restored, &[])?;

    Ok(saved)
}

/// Count of snapshots for a given note.
pub fn count_snapshots(context: &StorageContext, note_id: &str) -> Result<usize> {
    let (connection, _) = open_connection(context)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM note_snapshots WHERE note_id = ?1",
        params![note_id],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

// ── helpers ────────────────────────────────────────────────────────

fn serialize_note_meta(meta: &crate::models::NoteMeta) -> Result<String> {
    serde_json::to_string(meta).with_context(|| "failed to serialise NoteMeta for snapshot")
}

/// Keep at most `MAX_SNAPSHOTS_PER_NOTE` snapshots per note, deleting the
/// oldest when the limit is exceeded.
fn prune_old_snapshots(connection: &Connection, note_id: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM note_snapshots
         WHERE id IN (
             SELECT id FROM (
                 SELECT id, ROW_NUMBER() OVER (ORDER BY created_at DESC) AS rn
                 FROM note_snapshots
                 WHERE note_id = ?1
             ) WHERE rn > ?2
         )",
        params![note_id, MAX_SNAPSHOTS_PER_NOTE as i64],
    )?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{initialize_storage_with_context, notes::save_note_with_context};

    fn setup_test_context() -> (std::path::PathBuf, StorageContext) {
        let ns = Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("vp-snapshot-test-{ns}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let ctx = StorageContext::for_test(&dir);
        initialize_storage_with_context(&ctx).expect("init storage");
        (dir, ctx)
    }

    fn create_test_note(ctx: &StorageContext, title: &str, body: &str) -> NoteDocument {
        let note = NoteDocument {
            meta: NoteMeta {
                title: title.to_string(),
                ..Default::default()
            },
            body: body.to_string(),
            ..Default::default()
        };
        save_note_with_context(ctx, note).expect("save note")
    }

    #[test]
    fn test_snapshot_table_exists() {
        let (_dir, ctx) = setup_test_context();
        let (conn, _) = open_connection(&ctx).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='note_snapshots'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "note_snapshots table should exist");
    }

    #[test]
    fn test_snapshot_created_on_save() {
        let (_dir, ctx) = setup_test_context();

        // Create a note — first save, no snapshot (nothing to capture yet).
        let note = create_test_note(&ctx, "Snapshot Test", "v1 body");

        let snapshots = list_snapshots_for_note(&ctx, &note.meta.id).unwrap();
        assert_eq!(
            snapshots.len(),
            0,
            "first save should not create a snapshot"
        );

        // Update the note — this should create a snapshot of v1.
        let mut v2 = note.clone();
        v2.body = "v2 body".to_string();
        let saved = save_note_with_context(&ctx, v2).unwrap();

        let snapshots = list_snapshots_for_note(&ctx, &saved.meta.id).unwrap();
        assert_eq!(snapshots.len(), 1, "update should create one snapshot");
        assert!(
            snapshots[0].body.contains("v1 body"),
            "snapshot body should contain original text"
        );
        assert_eq!(snapshots[0].source, "agent", "default source is 'agent'");

        // Update again — second snapshot.
        let mut v3 = saved.clone();
        v3.body = "v3 body".to_string();
        let saved = save_note_with_context(&ctx, v3).unwrap();

        let snapshots = list_snapshots_for_note(&ctx, &saved.meta.id).unwrap();
        assert_eq!(
            snapshots.len(),
            2,
            "two updates should create two snapshots"
        );
        assert!(
            snapshots[0].body.contains("v2 body"),
            "latest snapshot = prior version"
        );
        assert!(
            snapshots[1].body.contains("v1 body"),
            "oldest snapshot = first version"
        );
    }

    #[test]
    fn test_restore_snapshot() {
        let (_dir, ctx) = setup_test_context();

        let mut note = create_test_note(&ctx, "Restore Test", "original body");

        // Update to v2.
        note.body = "v2 body".to_string();
        let v2 = save_note_with_context(&ctx, note).unwrap();

        // Get the snapshot (should be v1 = original).
        let snapshots = list_snapshots_for_note(&ctx, &v2.meta.id).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert!(
            snapshots[0].body.contains("original body"),
            "snapshot body should contain original text"
        );

        // Restore to the snapshot.
        let restored = restore_snapshot(&ctx, &v2.meta.id, &snapshots[0].id).unwrap();
        assert!(
            restored.body.contains("original body"),
            "restored body should contain snapshot text"
        );

        // A new snapshot of the pre-restore state should have been created.
        let snapshots_after = list_snapshots_for_note(&ctx, &v2.meta.id).unwrap();
        assert_eq!(
            snapshots_after.len(),
            2,
            "restore should create a new snapshot"
        );
    }

    #[test]
    fn test_retention_prunes_old_snapshots() {
        let (_dir, ctx) = setup_test_context();

        // Create a note.
        let mut note = create_test_note(&ctx, "Retention Test", "v0");

        // Save over MAX_SNAPSHOTS_PER_NOTE times.
        for i in 1..=MAX_SNAPSHOTS_PER_NOTE + 5 {
            note.body = format!("v{i}");
            note = save_note_with_context(&ctx, note).unwrap();
        }

        let snapshots = list_snapshots_for_note(&ctx, &note.meta.id).unwrap();
        assert!(
            snapshots.len() <= MAX_SNAPSHOTS_PER_NOTE,
            "should have at most {MAX_SNAPSHOTS_PER_NOTE} snapshots, got {}",
            snapshots.len()
        );
    }

    #[test]
    fn test_get_snapshot_by_id() {
        let (_dir, ctx) = setup_test_context();

        let mut note = create_test_note(&ctx, "Get Snapshot", "v1");

        // Update to create a snapshot.
        note.body = "v2".to_string();
        let saved = save_note_with_context(&ctx, note).unwrap();

        let snapshots = list_snapshots_for_note(&ctx, &saved.meta.id).unwrap();
        assert_eq!(snapshots.len(), 1);

        let fetched = get_snapshot(&ctx, &snapshots[0].id).unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().id, snapshots[0].id);

        let not_found = get_snapshot(&ctx, "nonexistent-uuid").unwrap();
        assert!(not_found.is_none());
    }
}
