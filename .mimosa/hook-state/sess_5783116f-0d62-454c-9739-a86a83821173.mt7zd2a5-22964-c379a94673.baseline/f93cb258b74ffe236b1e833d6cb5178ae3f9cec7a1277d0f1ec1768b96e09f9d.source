//! Chat-state and AI-action commands.

use crate::state::AppState;
use tauri::Emitter;
use vaultpilot_lib::ai::actions::{
    execute_ai_action as lib_execute_ai_action, list_ai_actions, AiActionRequest,
};
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
pub async fn execute_ai_action(
    state: tauri::State<'_, AppState>,
    request: AiActionRequest,
) -> Result<serde_json::Value, String> {
    let settings = vaultpilot_lib::storage::initialize_storage_async(&state.storage)
        .await
        .map_err(|e| e.to_string())?;
    let result = lib_execute_ai_action(&settings, &request).await;
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
        None, // no provider override for chat
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

/// Persists a base64-encoded attachment (image picked via `<input
/// type="file">`, audio blob from MediaRecorder) and returns its absolute
/// path. The WebView only hands the frontend in-memory bytes, but the agent's
/// image/audio pipeline needs real disk paths (#4074).
///
/// When `persistent` is `true` (image sends) the bytes are written into the
/// vault's `attachments/chat/` directory so history images survive OS
/// temp-dir wipes; the frontend persists only the returned path — never the
/// base64 blob — keeping `chat_state.json` small (#4083). Otherwise (audio
/// blobs) the file goes to the OS temp dir, which is TTL-swept.
///
/// Note: the command name must stay `save_temp_attachment` (no `_cmd` suffix)
/// — Tauri v2 registers commands by their exact fn identifier, and the
/// frontend invokes `save_temp_attachment` (#4082).
#[tauri::command]
pub async fn save_temp_attachment(
    state: tauri::State<'_, AppState>,
    data_base64: String,
    filename: String,
    persistent: Option<bool>,
) -> Result<String, String> {
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|e| format!("failed to decode base64 attachment: {e}"))?;
    if persistent.unwrap_or(false) {
        let settings = vaultpilot_lib::storage::initialize_storage_async(&state.storage)
            .await
            .map_err(|e| e.to_string())?;
        let vault_dir = std::path::PathBuf::from(&settings.vault_dir);
        vaultpilot_lib::attachments::save_chat_attachment(&data, &filename, &vault_dir)
            .map_err(|e| e.to_string())
    } else {
        vaultpilot_lib::attachments::save_temp_attachment(&data, &filename)
            .map_err(|e| e.to_string())
    }
}

/// Transcribes an audio file (e.g. a voice message recorded in the UI) to text
/// via the active provider's Whisper-compatible endpoint (#4074).
///
/// Command name is the exact fn identifier (no `_cmd` suffix) so the frontend
/// `invoke("transcribe_audio", …)` resolves (#4082).
#[tauri::command]
pub async fn transcribe_audio(
    state: tauri::State<'_, AppState>,
    audio_path: String,
    language: Option<String>,
) -> Result<String, String> {
    let settings = vaultpilot_lib::storage::initialize_storage_async(&state.storage)
        .await
        .map_err(|e| e.to_string())?;
    let provider = settings.effective_provider().clone();
    vaultpilot_lib::ai::transcription::transcribe_audio(&audio_path, &provider, language.as_deref())
        .await
        .map_err(|e| e.to_string())
}
