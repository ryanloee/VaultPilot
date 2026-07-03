use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use crate::ai;
use crate::models::NoteDocument;
use crate::storage::{
    has_notes_async, initialize_storage_async, load_context_notes_async,
    load_recent_notes_for_overview_async, save_note_with_images_async, StorageContext,
};
use tracing::instrument;

/// Per-AI-call timeout to prevent indefinite hangs in the orchestration layer.
const AI_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout for storage-layer I/O calls (NFS, slow disk, etc.).
const STORAGE_IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum number of backups to retain per note.
const MAX_BACKUPS_PER_NOTE: usize = 5;

// ── Write Backup / Revert (#1986) ──────────────────────────────────────────

/// A backup of a note's state before an AI write was applied.
/// Stores all `NoteMeta` fields so that a revert fully restores metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WriteBackup {
    /// The note ID that was modified.
    pub note_id: String,
    /// The note path (from NoteMeta.path).
    pub note_path: String,
    /// The note title before modification.
    pub title: String,
    /// Tags associated with the note.
    pub tags: Vec<String>,
    /// Keywords associated with the note.
    pub keywords: Vec<String>,
    /// Platform metadata.
    pub platform: String,
    /// Board metadata.
    pub board: String,
    /// Kernel metadata.
    pub kernel: String,
    /// Status metadata.
    pub status: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Last-updated timestamp.
    pub updated_at: String,
    /// Source metadata.
    pub source: String,
    /// Summary / excerpt of the note.
    pub summary: String,
    /// Collections the note belongs to.
    pub collections: Vec<String>,
    /// The note body (markdown content) before modification.
    pub body: String,
    /// When the write was approved (Unix timestamp).
    pub timestamp: i64,
}

/// Thread-safe store for write backups, keyed by note_id.
/// Used to allow the user to revert AI-written notes.
pub struct WriteTracker {
    backups: Mutex<HashMap<String, Vec<WriteBackup>>>,
}

impl WriteTracker {
    /// Create a new empty WriteTracker.
    pub fn new() -> Self {
        Self {
            backups: Mutex::new(HashMap::new()),
        }
    }

    /// Record a backup of a note before it gets modified.
    pub fn record_backup(&self, note: &NoteDocument) {
        let entry = WriteBackup {
            note_id: note.meta.id.clone(),
            note_path: note.meta.path.clone(),
            title: note.meta.title.clone(),
            tags: note.meta.tags.clone(),
            keywords: note.meta.keywords.clone(),
            platform: note.meta.platform.clone(),
            board: note.meta.board.clone(),
            kernel: note.meta.kernel.clone(),
            status: note.meta.status.clone(),
            created_at: note.meta.created_at.clone(),
            updated_at: note.meta.updated_at.clone(),
            source: note.meta.source.clone(),
            summary: note.meta.summary.clone(),
            collections: note.meta.collections.clone(),
            body: note.body.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        };
        let mut map = self.backups.lock().unwrap_or_else(|e| e.into_inner());
        let backups = map.entry(entry.note_id.clone()).or_default();
        backups.push(entry);
        // Keep only the most recent N backups
        if backups.len() > MAX_BACKUPS_PER_NOTE {
            backups.remove(0);
        }
    }

    /// Retrieve the most recent backup for a given note_id, if any.
    pub fn get_latest_backup(&self, note_id: &str) -> Option<WriteBackup> {
        let map = self.backups.lock().unwrap_or_else(|e| e.into_inner());
        map.get(note_id).and_then(|v| v.last().cloned())
    }

    /// Remove and return the most recent backup for a note_id.
    pub fn pop_backup(&self, note_id: &str) -> Option<WriteBackup> {
        let mut map = self.backups.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(backups) = map.get_mut(note_id) {
            let entry = backups.pop();
            if backups.is_empty() {
                map.remove(note_id);
            }
            entry
        } else {
            None
        }
    }
}

impl Default for WriteTracker {
    fn default() -> Self {
        Self::new()
    }
}

// Lazy global WriteTracker shared across the agent session.
use std::sync::LazyLock;
pub static WRITE_TRACKER: LazyLock<WriteTracker> = LazyLock::new(WriteTracker::new);

/// Revert a note to its pre-AI-write state by restoring the backup.
/// Returns the restored `NoteDocument` on success, or an error message.
pub async fn revert_write(
    ctx: &StorageContext,
    note_id: &str,
) -> Result<NoteDocument, anyhow::Error> {
    let backup = WRITE_TRACKER
        .get_latest_backup(note_id)
        .ok_or_else(|| anyhow::anyhow!("no backup found for note '{}'", note_id))?;

    let restored = NoteDocument {
        meta: crate::models::NoteMeta {
            id: backup.note_id.clone(),
            title: backup.title.clone(),
            tags: backup.tags.clone(),
            keywords: backup.keywords.clone(),
            platform: backup.platform.clone(),
            board: backup.board.clone(),
            kernel: backup.kernel.clone(),
            status: backup.status.clone(),
            created_at: backup.created_at.clone(),
            updated_at: backup.updated_at.clone(),
            source: backup.source.clone(),
            path: backup.note_path.clone(),
            summary: backup.summary.clone(),
            collections: backup.collections.clone(),
        },
        body: backup.body.clone(),
        ..Default::default()
    };

    let saved = save_note_with_images_async(ctx, restored, &[]).await?;

    // Only pop the backup after the save succeeded — if the save fails,
    // the backup is preserved for a retry.
    WRITE_TRACKER.pop_backup(note_id);

    tracing::info!(
        "reverted note '{}' to backup from {}",
        saved.meta.id,
        backup.timestamp
    );
    Ok(saved)
}

// ── Original module code below ─────────────────────────────────────────────

/// Generate writing-oriented markdown content using vault notes as context.
///
/// This is a single-round function: it searches the vault for relevant notes,
/// then makes a single AI call with a writing assistant persona. No complex
/// tool-calling loops or multiple rounds.
///
/// `mode` is one of "write", "edit", "expand", or "summarize".
#[instrument(skip(context, prompt))]
pub async fn write_with_ai_with_context(
    context: &StorageContext,
    prompt: String,
    context_note_id: Option<String>,
    mode: String,
) -> Result<String, anyhow::Error> {
    let settings = initialize_storage_async(context).await?;

    let raw_prompt = prompt.trim().to_string();
    if raw_prompt.is_empty() {
        return Err(anyhow::anyhow!("prompt is empty"));
    }

    // Search for relevant vault notes (single round)
    let mut docs: Vec<NoteDocument> = Vec::new();

    // If a specific context note ID is given, try to load it directly
    if let Some(ref note_id) = context_note_id {
        let note_id = note_id.trim();
        if !note_id.is_empty() {
            match load_note_by_id_async(context, note_id).await {
                Ok(Some(doc)) => docs.push(doc),
                Ok(None) => {
                    tracing::warn!("context note '{}' not found", note_id);
                }
                Err(e) => {
                    tracing::warn!("failed to load context note '{}': {}", note_id, e);
                }
            }
        }
    }

    // If no context note was loaded (or not specified), search the vault
    if docs.is_empty() && has_notes_async(context).await.unwrap_or(false) {
        match tokio::time::timeout(
            STORAGE_IO_TIMEOUT,
            load_context_notes_async(context, &raw_prompt, &[], 8),
        )
        .await
        .map_err(|_| anyhow::anyhow!("storage I/O timed out (search_notes in write)"))
        .and_then(|r| r)
        {
            Ok(found_docs) => {
                if found_docs.is_empty() {
                    // Fallback: load recent notes for context
                    if let Ok(recent) = tokio::time::timeout(
                        STORAGE_IO_TIMEOUT,
                        load_recent_notes_for_overview_async(context, 6),
                    )
                    .await
                    .map_err(|_| anyhow::anyhow!("storage I/O timed out (recent notes fallback)"))
                    .and_then(|r| r)
                    {
                        docs = recent;
                    }
                } else {
                    docs = found_docs;
                }
            }
            Err(_) => {
                tracing::warn!("search_notes failed in write mode, proceeding without notes");
            }
        }
    }

    // Single-round AI call with writing persona
    let result = tokio::time::timeout(
        AI_CALL_TIMEOUT,
        ai::generate_with_context(&settings, &raw_prompt, &docs, &mode),
    )
    .await
    .map_err(|_| anyhow::anyhow!("AI call timed out (generate_with_context)"))??;

    Ok(result)
}

/// Load a single note by its ID directly from storage.
async fn load_note_by_id_async(
    ctx: &StorageContext,
    note_id: &str,
) -> Result<Option<NoteDocument>, anyhow::Error> {
    use crate::storage::load_note_async;
    match tokio::time::timeout(STORAGE_IO_TIMEOUT, load_note_async(ctx, note_id))
        .await
        .map_err(|_| anyhow::anyhow!("storage I/O timed out (load_note)"))
        .and_then(|r| r)
    {
        Ok(doc) => Ok(Some(doc)),
        Err(e) if e.to_string().contains("not found") => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_note(id: &str, title: &str, body: &str) -> NoteDocument {
        NoteDocument {
            meta: crate::models::NoteMeta {
                id: id.to_string(),
                title: title.to_string(),
                path: format!("{}.md", id),
                tags: vec!["test".to_string(), "rust".to_string()],
                keywords: vec!["keyword1".to_string()],
                platform: "test-platform".to_string(),
                board: "test-board".to_string(),
                kernel: "test-kernel".to_string(),
                status: "active".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-02T00:00:00Z".to_string(),
                source: "test".to_string(),
                summary: format!("Summary of {}", title),
                collections: vec!["collection-a".to_string()],
            },
            body: body.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_write_tracker_new_is_empty() {
        let tracker = WriteTracker::new();
        assert!(tracker.get_latest_backup("nonexistent").is_none());
        assert!(tracker.pop_backup("nonexistent").is_none());
    }

    #[test]
    fn test_write_tracker_record_and_pop() {
        let tracker = WriteTracker::new();
        let note = make_note("n1", "Original Title", "Original body content");
        tracker.record_backup(&note);

        let backup = tracker.get_latest_backup("n1");
        assert!(backup.is_some());
        let b = backup.unwrap();
        assert_eq!(b.note_id, "n1");
        assert_eq!(b.title, "Original Title");
        assert_eq!(b.body, "Original body content");
        assert_eq!(b.note_path, "n1.md");

        // pop should return the same backup
        let popped = tracker.pop_backup("n1");
        assert!(popped.is_some());
        let p = popped.unwrap();
        assert_eq!(p.title, "Original Title");
        assert_eq!(p.body, "Original body content");

        // after pop, no more backups for this note
        assert!(tracker.get_latest_backup("n1").is_none());
        assert!(tracker.pop_backup("n1").is_none());
    }

    #[test]
    fn test_write_tracker_max_backups() {
        let tracker = WriteTracker::new();
        // Record MAX_BACKUPS_PER_NOTE + 2 backups
        for i in 0..MAX_BACKUPS_PER_NOTE + 2 {
            let note = make_note("n1", &format!("Title {}", i), &format!("Body {}", i));
            tracker.record_backup(&note);
        }
        // Only the most recent MAX_BACKUPS_PER_NOTE should remain
        let map = tracker.backups.lock().unwrap();
        let backups = map.get("n1").unwrap();
        assert_eq!(backups.len(), MAX_BACKUPS_PER_NOTE);
        // The oldest backup (index 0) should have been dropped
        assert_eq!(backups[0].title, "Title 2"); // indices 0,1 are gone
        assert_eq!(
            backups[backups.len() - 1].title,
            format!("Title {}", MAX_BACKUPS_PER_NOTE + 1)
        );
    }

    #[test]
    fn test_write_tracker_multiple_notes() {
        let tracker = WriteTracker::new();
        tracker.record_backup(&make_note("a", "A", "body-a"));
        tracker.record_backup(&make_note("b", "B", "body-b"));

        let a = tracker.get_latest_backup("a");
        assert_eq!(a.unwrap().title, "A");

        let b = tracker.get_latest_backup("b");
        assert_eq!(b.unwrap().title, "B");

        // pop one note, the other should still be there
        tracker.pop_backup("a");
        assert!(tracker.get_latest_backup("a").is_none());
        assert!(tracker.get_latest_backup("b").is_some());
    }

    #[test]
    fn test_write_backup_serde_roundtrip() {
        let backup = WriteBackup {
            note_id: "n1".to_string(),
            note_path: "n1.md".to_string(),
            title: "Title".to_string(),
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            keywords: vec!["kw1".to_string()],
            platform: "plat".to_string(),
            board: "brd".to_string(),
            kernel: "krn".to_string(),
            status: "active".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            source: "src".to_string(),
            summary: "A summary".to_string(),
            collections: vec!["col1".to_string()],
            body: "Body content".to_string(),
            timestamp: 1700000000,
        };
        let json = serde_json::to_string(&backup).unwrap();
        let deserialized: WriteBackup = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.note_id, "n1");
        assert_eq!(deserialized.title, "Title");
        assert_eq!(deserialized.tags, vec!["tag1", "tag2"]);
        assert_eq!(deserialized.keywords, vec!["kw1"]);
        assert_eq!(deserialized.platform, "plat");
        assert_eq!(deserialized.board, "brd");
        assert_eq!(deserialized.kernel, "krn");
        assert_eq!(deserialized.status, "active");
        assert_eq!(deserialized.created_at, "2024-01-01T00:00:00Z");
        assert_eq!(deserialized.updated_at, "2024-01-02T00:00:00Z");
        assert_eq!(deserialized.source, "src");
        assert_eq!(deserialized.summary, "A summary");
        assert_eq!(deserialized.collections, vec!["col1"]);
        assert_eq!(deserialized.body, "Body content");
        assert_eq!(deserialized.timestamp, 1700000000);
    }

    #[test]
    fn test_write_tracker_poison_handling_default() {
        // Verify that the default implementation creates a valid tracker
        let tracker = WriteTracker::default();
        assert!(tracker.get_latest_backup("x").is_none());
    }
}
