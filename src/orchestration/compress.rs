use crate::models::{ConversationSummary, ConversationTurn};
use crate::storage::{initialize_storage_async, StorageContext};
use chrono::Utc;
use tracing::instrument;

use crate::ai;

#[instrument(skip(context, summary, history, emit_status))]
pub async fn compress_chat_history_with_context(
    context: &StorageContext,
    summary: Option<ConversationSummary>,
    history: Vec<ConversationTurn>,
    mut emit_status: impl FnMut(&str, String),
) -> Result<ConversationSummary, anyhow::Error> {
    let settings = initialize_storage_async(context).await?;
    emit_status(
        "compressing",
        "Compressing earlier conversation context".to_string(),
    );
    let existing_summary = summary
        .as_ref()
        .map(|item| item.text.as_str())
        .unwrap_or_default();
    let text = ai::compress_conversation(&settings, existing_summary, &history).await?;

    Ok(ConversationSummary {
        text,
        generated_at: Utc::now().to_rfc3339(),
        covered_turn_count: history.len(),
        compression_count: summary.map(|item| item.compression_count + 1).unwrap_or(1),
    })
}
