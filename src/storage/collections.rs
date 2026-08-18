//! Collection CRUD — many-to-many note grouping layer (#2042).
//!
//! Collections are user-defined groups that transcend the filesystem folder
//! hierarchy. A note can belong to zero or more collections, and deleting a
//! collection never deletes its member notes.
//!
//! Collections form a tree: `parent_id` is empty for root collections, and
//! non-empty for nested (sub-)collections. Deleting a collection cascades to
//! its children (they are removed too, notes are never deleted).

#![allow(dead_code)]

use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use tracing::{instrument, warn};
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
    create_collection_with_parent(context, name, description, "")
}

/// Create a new collection under `parent_id` (empty string = root).
#[instrument(skip(context))]
pub fn create_collection_with_parent(
    context: &StorageContext,
    name: &str,
    description: &str,
    parent_id: &str,
) -> Result<Collection> {
    let (connection, _) = open_connection(context)?;
    if !parent_id.is_empty() {
        let parent_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM collections WHERE id = ?1)",
                params![parent_id],
                |row| row.get(0),
            )
            .with_context(|| format!("failed to check parent collection '{parent_id}'"))?;
        if !parent_exists {
            anyhow::bail!("parent collection '{parent_id}' does not exist");
        }
    }
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();

    connection.execute(
        "INSERT INTO collections (id, name, description, created_at, updated_at, parent_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, name, description, now, now, parent_id],
    ).with_context(|| format!("failed to create collection '{name}'"))?;

    Ok(Collection {
        id,
        name: name.to_string(),
        description: description.to_string(),
        created_at: now.clone(),
        updated_at: now,
        parent_id: parent_id.to_string(),
        note_count: 0,
    })
}

/// Delete a collection and all its note associations (cascade).
/// Child collections are deleted recursively (cascade) — notes are never
/// deleted.
#[instrument(skip(context))]
pub fn delete_collection_with_context(
    context: &StorageContext,
    collection_id: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    delete_collection_tree(&connection, collection_id)?;
    let rows = connection
        .execute(
            "DELETE FROM collections WHERE id = ?1",
            params![collection_id],
        )
        .with_context(|| format!("failed to delete collection '{collection_id}'"))?;
    Ok(rows > 0)
}

/// Recursively delete child collections (and their note associations via
/// ON DELETE CASCADE), depth-first, so the root delete is the last statement.
fn delete_collection_tree(connection: &rusqlite::Connection, collection_id: &str) -> Result<()> {
    let mut stmt = connection.prepare("SELECT id FROM collections WHERE parent_id = ?1")?;
    let children: Vec<String> = stmt
        .query_map(params![collection_id], |row| row.get(0))
        .with_context(|| format!("failed to query children of collection '{collection_id}'"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .with_context(|| "failed to collect child collection ids")?;
    drop(stmt);

    for child in children {
        delete_collection_tree(connection, &child)?;
        connection
            .execute("DELETE FROM collections WHERE id = ?1", params![child])
            .with_context(|| format!("failed to delete child collection '{child}'"))?;
    }
    Ok(())
}

/// List all collections with their note counts.
#[instrument(skip(context))]
pub fn list_collections_with_context(context: &StorageContext) -> Result<Vec<Collection>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        r#"
        SELECT
            c.id, c.name, c.description, c.created_at, c.updated_at, c.parent_id,
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
                parent_id: row.get(5)?,
                note_count: row.get::<_, i64>(6)? as usize,
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
                c.id, c.name, c.description, c.created_at, c.updated_at, c.parent_id,
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
                    parent_id: row.get(5)?,
                    note_count: row.get::<_, i64>(6)? as usize,
                })
            },
        )
        .optional()
        .with_context(|| "failed to query collection")?;

    Ok(result)
}

/// Rename a collection. Returns `true` if renamed.
#[instrument(skip(context))]
pub fn rename_collection_with_context(
    context: &StorageContext,
    collection_id: &str,
    name: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let rows = connection
        .execute(
            "UPDATE collections SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now, collection_id],
        )
        .with_context(|| format!("failed to rename collection '{collection_id}'"))?;
    Ok(rows > 0)
}

/// Move a collection under a new parent (`""` = root). Rejects cycles
/// (moving a collection into itself or into its own descendant). Returns
/// `false` when the collection does not exist.
#[instrument(skip(context))]
pub fn move_collection_with_context(
    context: &StorageContext,
    collection_id: &str,
    new_parent_id: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    if collection_id == new_parent_id {
        anyhow::bail!("cannot move collection into itself");
    }
    if !new_parent_id.is_empty() {
        let parent_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM collections WHERE id = ?1)",
                params![new_parent_id],
                |row| row.get(0),
            )
            .with_context(|| format!("failed to check parent collection '{new_parent_id}'"))?;
        if !parent_exists {
            anyhow::bail!("parent collection '{new_parent_id}' does not exist");
        }
        // Cycle check: walk up from the target parent; if we ever reach
        // `collection_id`, moving would create a cycle.
        let mut current = new_parent_id.to_string();
        let mut hops = 0usize;
        loop {
            if current == collection_id {
                anyhow::bail!("cannot move collection into its own descendant");
            }
            let parent: Option<String> = connection
                .query_row(
                    "SELECT parent_id FROM collections WHERE id = ?1",
                    params![current],
                    |row| row.get(0),
                )
                .optional()
                .with_context(|| format!("failed to read parent of '{current}'"))?;
            match parent {
                Some(p) if !p.is_empty() => {
                    current = p;
                    hops += 1;
                    if hops > 10_000 {
                        anyhow::bail!("collection parent chain too deep — cycle detected");
                    }
                }
                _ => break,
            }
        }
    }

    let now = Utc::now().to_rfc3339();
    let rows = connection
        .execute(
            "UPDATE collections SET parent_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_parent_id, now, collection_id],
        )
        .with_context(|| format!("failed to move collection '{collection_id}'"))?;
    Ok(rows > 0)
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

    // Use INSERT OR IGNORE to avoid TOCTOU race conditions (#2189)
    let inserted = connection
        .execute(
            "INSERT OR IGNORE INTO note_collections (note_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
            params![note_id, collection_id, now],
        )
        .with_context(|| {
            format!("failed to add note '{note_id}' to collection '{collection_id}'")
        })?;

    if inserted == 0 {
        return Ok(false); // already existed
    }

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
        ORDER BY n.updated_at DESC
        LIMIT ?2 OFFSET ?3
        "#,
    )?;

    let notes = stmt
        .query_map(params![collection_id, limit as i64, offset as i64], |row| {
            Ok(NoteMeta {
                id: row.get(0)?,
                title: row.get(1)?,
                tags: match serde_json::from_str(&row.get::<_, String>(2)?) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(
                            field = "tags",
                            error = %e,
                            "failed to parse tags JSON: {}",
                            e,
                        );
                        Vec::new()
                    }
                },
                keywords: match serde_json::from_str(&row.get::<_, String>(3)?) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(
                            field = "keywords",
                            error = %e,
                            "failed to parse keywords JSON: {}",
                            e,
                        );
                        Vec::new()
                    }
                },
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
        SELECT c.id, c.name, c.description, c.created_at, c.updated_at, c.parent_id
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
                parent_id: row.get(5)?,
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
    parent_id: String,
) -> Result<Collection> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        create_collection_with_parent(&ctx, &name, &description, &parent_id)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn delete_collection_async(ctx: &StorageContext, collection_id: String) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || delete_collection_with_context(&ctx, &collection_id))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn rename_collection_async(
    ctx: &StorageContext,
    collection_id: String,
    name: String,
) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || rename_collection_with_context(&ctx, &collection_id, &name))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn move_collection_async(
    ctx: &StorageContext,
    collection_id: String,
    new_parent_id: String,
) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        move_collection_with_context(&ctx, &collection_id, &new_parent_id)
    })
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
        crate::storage::notes::delete_note_with_context(&ctx, &saved.meta.id, None).unwrap();

        // Collection should still exist but have zero notes
        let cols = list_collections_with_context(&ctx).unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].note_count, 0);
    }

    #[test]
    fn test_nested_collection_tree() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let root = create_collection_with_parent(&ctx, "Root", "", "").unwrap();
        let child = create_collection_with_parent(&ctx, "Child", "", &root.id).unwrap();
        let grandchild = create_collection_with_parent(&ctx, "Grandchild", "", &child.id).unwrap();

        assert_eq!(root.parent_id, "");
        assert_eq!(child.parent_id, root.id);
        assert_eq!(grandchild.parent_id, child.id);

        // Creating under a missing parent must fail.
        assert!(create_collection_with_parent(&ctx, "Orphan", "", "nope").is_err());

        let all = list_collections_with_context(&ctx).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_move_collection_and_cycle_rejection() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let a = create_collection_with_parent(&ctx, "A", "", "").unwrap();
        let b = create_collection_with_parent(&ctx, "B", "", "").unwrap();
        let c = create_collection_with_parent(&ctx, "C", "", "").unwrap();

        // Move B under A, C under B.
        assert!(move_collection_with_context(&ctx, &b.id, &a.id).unwrap());
        assert!(move_collection_with_context(&ctx, &c.id, &b.id).unwrap());

        let all = list_collections_with_context(&ctx).unwrap();
        let a_now = all.iter().find(|x| x.id == a.id).unwrap();
        let c_now = all.iter().find(|x| x.id == c.id).unwrap();
        assert_eq!(a_now.parent_id, "");
        assert_eq!(c_now.parent_id, b.id);

        // Moving A into its own descendant C must be rejected (cycle).
        assert!(move_collection_with_context(&ctx, &a.id, &c.id).is_err());
        // Moving A into itself must be rejected.
        assert!(move_collection_with_context(&ctx, &a.id, &a.id).is_err());
        // Moving C back to root is fine.
        assert!(move_collection_with_context(&ctx, &c.id, "").unwrap());
        // Moving under a missing parent must fail.
        assert!(move_collection_with_context(&ctx, &c.id, "missing").is_err());
    }

    #[test]
    fn test_delete_collection_cascades_to_children() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let root = create_collection_with_parent(&ctx, "Root", "", "").unwrap();
        let child = create_collection_with_parent(&ctx, "Child", "", &root.id).unwrap();
        let _grandchild = create_collection_with_parent(&ctx, "Grandchild", "", &child.id).unwrap();

        // Attach a note to the grandchild to prove note associations cascade.
        let note = crate::models::NoteDocument {
            meta: crate::models::NoteMeta {
                title: "Cascade Me".to_string(),
                ..Default::default()
            },
            body: "body".to_string(),
            ..Default::default()
        };
        let saved = save_note_with_context(&ctx, note).unwrap();
        add_note_to_collection_with_context(&ctx, &saved.meta.id, &child.id).unwrap();

        // Deleting the root removes the whole subtree.
        assert!(delete_collection_with_context(&ctx, &root.id).unwrap());
        assert!(list_collections_with_context(&ctx).unwrap().is_empty());

        // The note itself must survive.
        let loaded = crate::storage::load_note_with_context(&ctx, &saved.meta.id).unwrap();
        assert_eq!(loaded.meta.title, "Cascade Me");
    }

    #[test]
    fn test_rename_collection() {
        let (_temp, ctx) = setup_temp_context();
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        let col = create_collection_with_parent(&ctx, "Old", "", "").unwrap();
        assert!(rename_collection_with_context(&ctx, &col.id, "New").unwrap());
        assert!(!rename_collection_with_context(&ctx, "missing", "X").unwrap());

        let found = get_collection_with_context(&ctx, &col.id)
            .unwrap()
            .expect("exists");
        assert_eq!(found.name, "New");
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
