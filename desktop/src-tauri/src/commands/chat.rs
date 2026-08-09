//! Chat-state and AI-action commands.

use crate::state::AppState;
use tauri::Emitter;
use vaultpilot_lib::ai::actions::{execute_ai_action, list_ai_actions, AiActionRequest};
use vaultpilot_lib::ask_with_ai_with_context;
use vaultpilot_lib::models::{ChatState, ConversationSummary, ConversationTurn};
use vaultpilot_lib::storage::{load_chat_state_async, save_chat_state_async};

#[tauri::command]
pub async fn load_chat_state(state: tauri::State<'_, AppState>) -> Result<ChatState, String> {
    let _guard = state.storage.chat_state_lock.lock().await;
    load_chat_state_async(&state.storage)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_chat_state(
    state: tauri::State<'_, AppState>,
    chat_state: ChatState,
) -> Result<ChatState, String> {
    let _guard = state.storage.chat_state_lock.lock().await;
    save_chat_state_async(&state.storage, &chat_state)
        .await
        .map_err(|e| e.to_string())
}

/// Lists the built-in AI quick actions (translate, rewrite, summarize, …).
/// Returns serde_json::Value (the action descriptors are heterogeneous).
#[tauri::command]
pub async fn list_actions() -> Result<Vec<serde_json::Value>, String> {
    Ok(list_ai_actions())
}

/// Runs a single AI quick action against the active provider.
#[tauri::command]
pub async fn execute_ai_action_cmd(
    state: tauri::State<'_, AppState>,
    request: AiActionRequest,
) -> Result<serde_json::Value, String> {
    let settings = vaultpilot_lib::storage::initialize_storage_async(&state.storage)
        .await
        .map_err(|e| e.to_string())?;
    let result = execute_ai_action(&settings, &request).await;
    serde_json::to_value(&result).map_err(|e| e.to_string())
}

/// Compresses long chat history into a summary + trimmed turns.
#[tauri::command]
pub async fn compress_chat_history(
    state: tauri::State<'_, AppState>,
    summary: Option<ConversationSummary>,
    history: Vec<ConversationTurn>,
) -> Result<ConversationSummary, String> {
    vaultpilot_lib::compress_chat_history_with_context(
        &state.storage,
        summary,
        history,
        |_stage, _detail| {
            // No event streaming for the compression path in stage 1.
        },
    )
    .await
    .map_err(|e| e.to_string())
}

/// Ask the AI a question with vault context. Emits `agentStatus` events to the
/// frontend (Tauri event `agent-status`) as the request progresses, then
/// returns the final grounded answer.
#[tauri::command]
pub async fn ask_with_ai(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    question: String,
    history: Option<Vec<ConversationTurn>>,
    image_paths: Option<Vec<String>>,
    model_override: Option<String>,
) -> Result<serde_json::Value, String> {
    let storage = state.storage.clone();
    let result = ask_with_ai_with_context(
        &storage,
        question,
        history,
        image_paths,
        model_override,
        |stage, detail| {
            // Fan out progress to the frontend via a Tauri event. Best-effort:
            // a closed window just drops the emission.
            let _ = app_handle.emit(
                "agent-status",
                serde_json::json!({
                    "stage": stage,
                    "detail": detail,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                }),
            );
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    serde_json::to_value(&result).map_err(|e| e.to_string())
}
