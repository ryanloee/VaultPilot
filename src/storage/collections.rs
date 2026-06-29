//! Collection CRUD — many-to-many note grouping layer (#2042).
//!
//! Collections are flat, user-defined groups that transcend the filesystem
//! folder hierarchy. A note can belong to zero or more collections, and
//! deleting a collection never deletes its member notes.

#![allow(dead_code)]

use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use tracing::instrument;
use uuid::Uuid;

use crate::models::{Collection, NoteMeta};

use super::pool::open_connection;
use super::StorageContext;

// ────────────────────────────────────────────────────────
// Collection CRUD
// ────────────────────────────────────────────────────────

/// Create a new collection. Returns the created collection.
#[instrument(skip(context))]
pub fn create_collection_with_context(
    context: &StorageContext,
    name: &str,
    description: &str,
) -> Result<Collection> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();

    connection.execute(
        "INSERT INTO collections (id, name, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, name, description, now, now],
    ).with_context(|| format!("failed to create collection '{name}'"))?;

    Ok(Collection {
        id,
        name: name.to_string(),
        description: description.to_string(),
        created_at: now.clone(),
        updated_at: now,
        note_count: 0,
    })
}

/// Delete a collection and all its note associations (cascade).
#[instrument(skip(context))]
pub fn delete_collection_with_context(
    context: &StorageContext,
    collection_id: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let rows = connection
        .execute(
            "DELETE FROM collections WHERE id = ?1",
            params![collection_id],
        )
        .with_context(|| format!("failed to delete collection '{collection_id}'"))?;
    Ok(rows > 0)
}

/// List all collections with their note counts.
#[instrument(skip(context))]
pub fn list_collections_with_context(context: &StorageContext) -> Result<Vec<Collection>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        r#"
        SELECT
            c.id, c.name, c.description, c.created_at, c.updated_at,
            COALESCE(nc.cnt, 0) AS note_count
        FROM collections c
        LEFT JOIN (
            SELECT collection_id, COUNT(*) AS cnt
            FROM note_collections
            GROUP BY collection_id
        ) nc ON nc.collection_id = c.id
        ORDER BY c.name ASC
        "#,
    )?;

    let collections = stmt
        .query_map([], |row| {
            Ok(Collection {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                note_count: row.get::<_, i64>(5)? as usize,
            })
        })
        .with_context(|| "failed to query collections")?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| "failed to collect collections")?;

    Ok(collections)
}

/// Get a single collection by ID.
#[instrument(skip(context))]
pub fn get_collection_with_context(
    context: &StorageContext,
    collection_id: &str,
) -> Result<Option<Collection>> {
    let (connection, _) = open_connection(context)?;

    let result = connection
        .query_row(
            r#"
            SELECT
                c.id, c.name, c.description, c.created_at, c.updated_at,
                COALESCE(nc.cnt, 0) AS note_count
            FROM collections c
            LEFT JOIN (
                SELECT collection_id, COUNT(*) AS cnt
                FROM note_collections
                GROUP BY collection_id
            ) nc ON nc.collection_id = c.id
            WHERE c.id = ?1
            "#,
            params![collection_id],
            |row| {
                Ok(Collection {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    note_count: row.get::<_, i64>(5)? as usize,
                })
            },
        )
        .optional()
        .with_context(|| "failed to query collection")?;

    Ok(result)
}

// ────────────────────────────────────────────────────────
// Note-Collection associations
// ────────────────────────────────────────────────────────

/// Add a note to a collection. Returns `true` if the association was created,
/// `false` if it already existed.
#[instrument(skip(context))]
pub fn add_note_to_collection_with_context(
    context: &StorageContext,
    note_id: &str,
    collection_id: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();

    // Check if the association already exists
    let exists: bool = connection
        .query_row(
            "SELECT 1 FROM note_collections WHERE note_id = ?1 AND collection_id = ?2",
            params![note_id, collection_id],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);

    if exists {
        return Ok(false);
    }

    connection
        .execute(
            "INSERT INTO note_collections (note_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
            params![note_id, collection_id, now],
        )
        .with_context(|| {
            format!("failed to add note '{note_id}' to collection '{collection_id}'")
        })?;

    // Update the collection's updated_at timestamp
    connection.execute(
        "UPDATE collections SET updated_at = ?1 WHERE id = ?2",
        params![now, collection_id],
    )?;

    Ok(true)
}

/// Remove a note from a collection. Returns `true` if the association was removed.
#[instrument(skip(context))]
pub fn remove_note_from_collection_with_context(
    context: &StorageContext,
    note_id: &str,
    collection_id: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let rows = connection
        .execute(
            "DELETE FROM note_collections WHERE note_id = ?1 AND collection_id = ?2",
            params![note_id, collection_id],
        )
        .with_context(|| {
            format!("failed to remove note '{note_id}' from collection '{collection_id}'")
        })?;

    if rows > 0 {
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "UPDATE collections SET updated_at = ?1 WHERE id = ?2",
            params![now, collection_id],
        )?;
    }

    Ok(rows > 0)
}

/// List all notes belonging to a collection.
#[instrument(skip(context))]
pub fn list_notes_in_collection_with_context(
    context: &StorageContext,
    collection_id: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<NoteMeta>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        r#"
        SELECT n.id, n.title, n.tags, n.keywords, n.platform, n.board,
               n.kernel, n.status, n.created_at, n.updated_at, n.source,
               n.path, n.summary
        FROM notes n
        INNER JOIN note_collections nc ON nc.note_id = n.id
        WHERE nc.collection_id = ?1
        ORDER BY n.updated_at DESC, n.id ASC
        LIMIT ?2 OFFSET ?3
        "#,
    )?;

    let notes = stmt
        .query_map(params![collection_id, limit as i64, offset as i64], |row| {
            Ok(NoteMeta {
                id: row.get(0)?,
                title: row.get(1)?,
                tags: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                keywords: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                platform: row.get(4)?,
                board: row.get(5)?,
                kernel: row.get(6)?,
                status: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                source: row.get(10)?,
                path: row.get(11)?,
                summary: row.get(12)?,
                collections: Vec::new(),
            })
        })
        .with_context(|| "failed to query notes in collection")?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| "failed to collect notes")?;

    Ok(notes)
}

/// Get all collection IDs that a note belongs to.
#[instrument(skip(context))]
pub fn get_collections_for_note_with_context(
    context: &StorageContext,
    note_id: &str,
) -> Result<Vec<Collection>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        r#"
        SELECT c.id, c.name, c.description, c.created_at, c.updated_at
        FROM collections c
        INNER JOIN note_collections nc ON nc.collection_id = c.id
        WHERE nc.note_id = ?1
        ORDER BY c.name ASC
        "#,
    )?;

    let collections = stmt
        .query_map(params![note_id], |row| {
            Ok(Collection {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                note_count: 0,
            })
        })
        .with_context(|| "failed to query collections for note")?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| "failed to collect collections")?;

    Ok(collections)
}

/// Get collection IDs (as a HashSet) for a note — used internally for
/// building search results without constructing full Collection structs.
pub fn get_collection_ids_for_note(
    connection: &rusqlite::Connection,
    note_id: &str,
) -> Result<HashSet<String>> {
    let mut stmt =
        connection.prepare("SELECT collection_id FROM note_collections WHERE note_id = ?1")?;

    let ids = stmt
        .query_map(params![note_id], |row| row.get::<_, String>(0))
        .with_context(|| "failed to query collection ids for note")?
        .collect::<Result<HashSet<_>, _>>()
        .with_context(|| "failed to collect collection ids")?;

    Ok(ids)
}

/// Count notes in a collection.
#[instrument(skip(context))]
pub fn count_notes_in_collection_with_context(
    context: &StorageContext,
    collection_id: &str,
) -> Result<usize> {
    let (connection, _) = open_connection(context)?;
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM note_collections WHERE collection_id = ?1",
            params![collection_id],
            |row| row.get(0),
        )
        .with_context(|| "failed to count notes in collection")?;
    Ok(count as usize)
}

// ────────────────────────────────────────────────────────
// Async wrappers
// ────────────────────────────────────────────────────────

pub async fn create_collection_async(
    ctx: &StorageContext,
    name: String,
    description: String,
) -> Result<Collection> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || create_collection_with_context(&ctx, &name, &description))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn delete_collection_async(ctx: &StorageContext, collection_id: String) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || delete_collection_with_context(&ctx, &collection_id))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn list_collections_async(ctx: &StorageContext) -> Result<Vec<Collection>> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || list_collections_with_context(&ctx))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn add_note_to_collection_async(
    ctx: &StorageContext,
    note_id: String,
    collection_id: String,
) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        add_note_to_collection_with_context(&ctx, &note_id, &collection_id)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn remove_note_from_collection_async(
    ctx: &StorageContext,
    note_id: String,
    collection_id: String,
) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        remove_note_from_collection_with_context(&ctx, &note_id, &collection_id)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn list_notes_in_collection_async(
    ctx: &StorageContext,
    collection_id: String,
    limit: usize,
    offset: usize,
) -> Result<Vec<NoteMeta>> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        list_notes_in_collection_with_context(&ctx, &collection_id, limit, offset)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::notes::save_note_with_context;
    use crate::storage::StorageContext;
    use chrono::Utc;
    use std::path::PathBuf;

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-collections-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    #[test]
    fn test_create_and_list_collections() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let col = create_collection_with_context(&ctx, "Test Col", "A test collection").unwrap();
        assert_eq!(col.name, "Test Col");
        assert!(!col.id.is_empty());

        let cols = list_collections_with_context(&ctx).unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, "Test Col");
        assert_eq!(cols[0].note_count, 0);
    }

    #[test]
    fn test_delete_collection() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let col = create_collection_with_context(&ctx, "Delete Me", "").unwrap();
        assert!(delete_collection_with_context(&ctx, &col.id).unwrap());
        assert!(!delete_collection_with_context(&ctx, "nonexistent").unwrap());
        assert!(list_collections_with_context(&ctx).unwrap().is_empty());
    }

    #[test]
    fn test_add_remove_note_to_collection() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let col = create_collection_with_context(&ctx, "Project A", "").unwrap();

        // Create a note first
        let note = crate::models::NoteDocument {
            meta: crate::models::NoteMeta {
                title: "Test Note".to_string(),
                ..Default::default()
            },
            body: "Hello".to_string(),
            ..Default::default()
        };
        let saved = save_note_with_context(&ctx, note).unwrap();

        // Add note to collection
        assert!(add_note_to_collection_with_context(&ctx, &saved.meta.id, &col.id).unwrap());
        // Duplicate add should return false
        assert!(!add_note_to_collection_with_context(&ctx, &saved.meta.id, &col.id).unwrap());

        // Verify notes in collection
        let notes = list_notes_in_collection_with_context(&ctx, &col.id, 50, 0).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Test Note");

        // Verify collections for note
        let note_cols = get_collections_for_note_with_context(&ctx, &saved.meta.id).unwrap();
        assert_eq!(note_cols.len(), 1);
        assert_eq!(note_cols[0].name, "Project A");

        // Remove note from collection
        assert!(remove_note_from_collection_with_context(&ctx, &saved.meta.id, &col.id).unwrap());
        assert!(!remove_note_from_collection_with_context(&ctx, &saved.meta.id, &col.id).unwrap());

        // Verify empty
        let notes = list_notes_in_collection_with_context(&ctx, &col.id, 50, 0).unwrap();
        assert_eq!(notes.len(), 0);
    }

    #[test]
    fn test_note_count_updates() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let col = create_collection_with_context(&ctx, "Multi", "").unwrap();

        // Create and add two notes
        for i in 0..2 {
            let note = crate::models::NoteDocument {
                meta: crate::models::NoteMeta {
                    title: format!("Note {}", i),
                    ..Default::default()
                },
                body: "body".to_string(),
                ..Default::default()
            };
            let saved = save_note_with_context(&ctx, note).unwrap();
            add_note_to_collection_with_context(&ctx, &saved.meta.id, &col.id).unwrap();
        }

        let cols = list_collections_with_context(&ctx).unwrap();
        assert_eq!(cols[0].note_count, 2);
    }

    #[test]
    fn test_collection_delete_does_not_delete_notes() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let col = create_collection_with_context(&ctx, "Temp", "").unwrap();
        let note = crate::models::NoteDocument {
            meta: crate::models::NoteMeta {
                title: "Keep Me".to_string(),
                ..Default::default()
            },
            body: "body".to_string(),
            ..Default::default()
        };
        let saved = save_note_with_context(&ctx, note).unwrap();
        add_note_to_collection_with_context(&ctx, &saved.meta.id, &col.id).unwrap();

        // Delete collection
        delete_collection_with_context(&ctx, &col.id).unwrap();

        // Note should still exist
        let loaded = crate::storage::load_note_with_context(&ctx, &saved.meta.id).unwrap();
        assert_eq!(loaded.meta.title, "Keep Me");
    }

    #[test]
    fn test_note_delete_cascades_collection_association() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let col = create_collection_with_context(&ctx, "Test", "").unwrap();
        let note = crate::models::NoteDocument {
            meta: crate::models::NoteMeta {
                title: "Delete Me".to_string(),
                ..Default::default()
            },
            body: "body".to_string(),
            ..Default::default()
        };
        let saved = save_note_with_context(&ctx, note).unwrap();
        add_note_to_collection_with_context(&ctx, &saved.meta.id, &col.id).unwrap();

        // Delete note
        crate::storage::notes::delete_note_with_context(&ctx, &saved.meta.id).unwrap();

        // Collection should still exist but have zero notes
        let cols = list_collections_with_context(&ctx).unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].note_count, 0);
    }

    #[test]
    fn test_get_collection() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let col = create_collection_with_context(&ctx, "Find Me", "found").unwrap();
        let found = get_collection_with_context(&ctx, &col.id)
            .unwrap()
            .expect("should exist");
        assert_eq!(found.name, "Find Me");
        assert_eq!(found.description, "found");

        let not_found = get_collection_with_context(&ctx, "does-not-exist").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_count_notes_in_collection() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let col = create_collection_with_context(&ctx, "Count Test", "").unwrap();
        assert_eq!(
            count_notes_in_collection_with_context(&ctx, &col.id).unwrap(),
            0
        );

        let note = crate::models::NoteDocument {
            meta: crate::models::NoteMeta {
                title: "Counted".to_string(),
                ..Default::default()
            },
            body: "body".to_string(),
            ..Default::default()
        };
        let saved = save_note_with_context(&ctx, note).unwrap();
        add_note_to_collection_with_context(&ctx, &saved.meta.id, &col.id).unwrap();
        assert_eq!(
            count_notes_in_collection_with_context(&ctx, &col.id).unwrap(),
            1
        );
    }
}
