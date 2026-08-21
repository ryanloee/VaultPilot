use std::time::Duration;

use crate::ai;
use crate::models::NoteDocument;
use crate::storage::{
    has_notes_async, initialize_storage_async, load_context_notes_async, load_note_async,
    load_recent_notes_for_overview_async, StorageContext,
};
use tracing::instrument;

/// Per-AI-call timeout to prevent indefinite hangs in the orchestration layer.
const AI_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout for storage-layer I/O calls (NFS, slow disk, etc.).
const STORAGE_IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Generate a Markdown comparison table from vault notes using AI (#1963).
///
/// This searches the vault for relevant notes based on the user's prompt,
/// then makes a single AI call with a data-table-analyst persona to produce
/// a structured comparison table. No complex tool-calling loops or multiple
/// rounds.
#[instrument(skip(context, prompt))]
pub async fn table_with_ai_with_context(
    context: &StorageContext,
    prompt: String,
    context_note_id: Option<String>,
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
        .map_err(|_| anyhow::anyhow!("storage I/O timed out (search_notes in table)"))
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
                tracing::warn!("search_notes failed in table mode, proceeding without notes");
            }
        }
    }

    // Single-round AI call with data-table-analyst persona
    let result = tokio::time::timeout(
        AI_CALL_TIMEOUT,
        ai::generate_table(&settings, &raw_prompt, &docs),
    )
    .await
    .map_err(|_| anyhow::anyhow!("AI call timed out (generate_table)"))??;

    Ok(result)
}

/// Load a single note by its ID directly from storage.
async fn load_note_by_id_async(
    ctx: &StorageContext,
    note_id: &str,
) -> Result<Option<NoteDocument>, anyhow::Error> {
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
                tags: vec!["test".to_string(), "comparison".to_string()],
                keywords: vec!["compare".to_string()],
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
    fn test_table_prompts_compile_and_produce_output() {
        // Verify that the table prompts are syntactically valid format! calls
        // and produce non-empty strings with expected content.
        let system = crate::prompting::table_system_prompt();
        assert!(!system.is_empty());
        assert!(system.contains("data table analyst"));
        assert!(system.contains("Dimension | Note 1 Title"));

        let user = crate::prompting::table_user_prompt("compare frameworks", &[]);
        assert!(!user.is_empty());
        assert!(user.contains("compare frameworks"));
    }

    #[test]
    fn test_table_prompt_mentions_no_prose_rule() {
        let system = crate::prompting::table_system_prompt();
        assert!(system.contains("Return ONLY the Markdown table"));
    }

    #[test]
    fn test_table_user_prompt_with_notes() {
        let docs = vec![
            make_note("a", "Framework A", "Has feature X"),
            make_note("b", "Framework B", "Has feature Y"),
        ];
        let user = crate::prompting::table_user_prompt("compare these", &docs);
        assert!(user.contains("Framework A"));
        assert!(user.contains("Framework B"));
        assert!(user.contains("compare these"));
    }

    #[test]
    fn test_table_prompt_has_injection_defense() {
        let system = crate::prompting::table_system_prompt();
        assert!(system.contains("PROMPT INJECTION DEFENSE"));
    }

    #[test]
    fn test_table_prompt_has_date() {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let system = crate::prompting::table_system_prompt();
        assert!(system.contains(&today));
    }
}
