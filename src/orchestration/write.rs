use std::time::Duration;

use crate::ai;
use crate::models::NoteDocument;
use crate::storage::{
    has_notes_async, initialize_storage_async, load_context_notes_async,
    load_recent_notes_for_overview_async, StorageContext,
};
use tracing::instrument;

/// Per-AI-call timeout to prevent indefinite hangs in the orchestration layer.
const AI_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout for storage-layer I/O calls (NFS, slow disk, etc.).
const STORAGE_IO_TIMEOUT: Duration = Duration::from_secs(30);

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
