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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WriteBackup {
    /// The note ID that was modified.
    pub note_id: String,
    /// The note path (from NoteMeta.path).
    pub note_path: String,
    /// The note title before modification.
    pub title: String,
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
            body: note.body.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        };
        let mut map = self.backups.lock().unwrap();
        let backups = map.entry(entry.note_id.clone()).or_default();
        backups.push(entry);
        // Keep only the most recent N backups
        if backups.len() > MAX_BACKUPS_PER_NOTE {
            backups.remove(0);
        }
    }

    /// Retrieve the most recent backup for a given note_id, if any.
    pub fn get_latest_backup(&self, note_id: &str) -> Option<WriteBackup> {
        let map = self.backups.lock().unwrap();
        map.get(note_id).and_then(|v| v.last().cloned())
    }

    /// Remove and return the most recent backup for a note_id.
    pub fn pop_backup(&self, note_id: &str) -> Option<WriteBackup> {
        let mut map = self.backups.lock().unwrap();
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
        .pop_backup(note_id)
        .ok_or_else(|| anyhow::anyhow!("no backup found for note '{}'", note_id))?;

    let restored = NoteDocument {
        meta: crate::models::NoteMeta {
            id: backup.note_id.clone(),
            title: backup.title.clone(),
            path: backup.note_path.clone(),
            ..Default::default()
        },
        body: backup.body.clone(),
        ..Default::default()
    };

    let saved = save_note_with_images_async(ctx, restored, &[]).await?;
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
