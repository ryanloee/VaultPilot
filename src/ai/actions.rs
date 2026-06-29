use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::client::{send_request_with_temperature, RequestUsage};
use crate::models::AppSettings;

// ─── Action type enum ─────────────────────────────────────────────────

/// Supported AI quick action types for the global command palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AiActionType {
    /// Summarize selected text or note into key points.
    Summarize,
    /// Rewrite text with a specified tone (formal, concise, vivid).
    Rewrite,
    /// Translate text to a target language.
    Translate,
    /// Explain a selected concept or term.
    Explain,
    /// Continue writing from the given text.
    ContinueWriting,
    /// Extract action items / to-dos from text.
    ExtractTodos,
    /// Find notes related to the given content.
    FindRelatedNotes,
}

impl AiActionType {
    /// Human-readable Chinese label for display in the command palette.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Summarize => "总结",
            Self::Rewrite => "改写",
            Self::Translate => "翻译",
            Self::Explain => "解释",
            Self::ContinueWriting => "续写",
            Self::ExtractTodos => "提取待办",
            Self::FindRelatedNotes => "关联笔记",
        }
    }

    /// English identifier for IPC/HTTP transport.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Summarize => "summarize",
            Self::Rewrite => "rewrite",
            Self::Translate => "translate",
            Self::Explain => "explain",
            Self::ContinueWriting => "continueWriting",
            Self::ExtractTodos => "extractTodos",
            Self::FindRelatedNotes => "findRelatedNotes",
        }
    }

    /// Parse from an English identifier string.
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "summarize" => Some(Self::Summarize),
            "rewrite" => Some(Self::Rewrite),
            "translate" => Some(Self::Translate),
            "explain" => Some(Self::Explain),
            "continueWriting" | "continue_writing" => Some(Self::ContinueWriting),
            "extractTodos" | "extract_todos" => Some(Self::ExtractTodos),
            "findRelatedNotes" | "find_related_notes" => Some(Self::FindRelatedNotes),
            _ => None,
        }
    }

    /// Returns all available action types (for the command palette list).
    pub fn all() -> Vec<AiActionType> {
        vec![
            Self::Summarize,
            Self::Rewrite,
            Self::Translate,
            Self::Explain,
            Self::ContinueWriting,
            Self::ExtractTodos,
            Self::FindRelatedNotes,
        ]
    }
}

// ─── Request / Response types ───────────────────────────────────────────

/// Parameters for executing an AI quick action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiActionRequest {
    /// The action type to perform.
    pub action: AiActionType,
    /// The text content to operate on (selected text, note body, etc.).
    #[serde(default)]
    pub text: String,
    /// Optional parameter: target language for translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_language: Option<String>,
    /// Optional parameter: target tone for rewrite (formal, concise, vivid).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    /// Optional parameter: note ID for context (e.g., find_related_notes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    /// Optional model override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Result of an executed AI quick action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiActionResult {
    /// The resulting text after the action was applied.
    pub result: String,
    /// Token usage statistics.
    #[serde(default)]
    pub usage: RequestUsage,
    /// Error message if the action failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ─── Prompt builders ───────────────────────────────────────────────────

/// Build the system prompt for a given AI action type.
fn system_prompt(action: AiActionType) -> String {
    match action {
        AiActionType::Summarize => {
            "You are a text summarization assistant. Your task is to distill the \
             given text into concise, well-structured key points. \
             Output only the summary, no extra commentary.\n\
             Respond in the same language as the input text."
                .to_string()
        }
        AiActionType::Rewrite => {
            "You are a writing assistant. Rewrite the given text according to the \
             specified tone. If no tone is specified, rewrite it in a clear, \
             professional style. Preserve all factual information.\n\
             Respond in the same language as the input text."
                .to_string()
        }
        AiActionType::Translate => {
            "You are a professional translator. Translate the given text to the \
             specified target language. If no target language is specified, \
             detect the source language and translate to English (or Chinese if \
             the source is English). Output only the translation."
                .to_string()
        }
        AiActionType::Explain => {
            "You are a knowledgeable explainer. Explain the given concept, term, \
             or passage in clear, accessible language. Provide context and \
             examples where helpful. Respond in the same language as the input."
                .to_string()
        }
        AiActionType::ContinueWriting => {
            "You are a creative writing assistant. Continue writing from the \
             given text naturally, maintaining the same style, tone, and context. \
             Output only the continuation without prefix phrases."
                .to_string()
        }
        AiActionType::ExtractTodos => {
            "You are a task extraction assistant. Analyze the given text and \
             extract all action items, tasks, to-dos, and follow-ups. \
             Format the output as a bullet-point list with clear descriptions. \
             If no tasks are found, state that explicitly.\n\
             Respond in the same language as the input text."
                .to_string()
        }
        AiActionType::FindRelatedNotes => {
            "You are a knowledge base assistant. Analyze the given text and \
             describe what topics, keywords, and concepts it covers. \
             This description will be used for a search query to find related \
             notes in the vault. Output a concise search description."
                .to_string()
        }
    }
}

/// Build the user prompt for a given AI action type.
fn user_prompt(action: AiActionType, request: &AiActionRequest) -> String {
    match action {
        AiActionType::Summarize => {
            format!("Please summarize the following text into key points:\n\n{}", request.text)
        }
        AiActionType::Rewrite => {
            let tone = request.tone.as_deref().unwrap_or("professional");
            format!(
                "Please rewrite the following text with a {} tone:\n\n{}",
                tone, request.text
            )
        }
        AiActionType::Translate => {
            let language = request
                .target_language
                .as_deref()
                .unwrap_or("English (or Chinese if the source is English)");
            format!(
                "Translate the following text to {}:\n\n{}",
                language, request.text
            )
        }
        AiActionType::Explain => {
            format!("Please explain the following:\n\n{}", request.text)
        }
        AiActionType::ContinueWriting => {
            format!("Continue writing from the following text:\n\n{}", request.text)
        }
        AiActionType::ExtractTodos => {
            format!(
                "Extract all action items, tasks, and to-dos from the following text:\n\n{}",
                request.text
            )
        }
        AiActionType::FindRelatedNotes => {
            format!(
                "Based on the following text, generate a search query to find related notes:\n\n{}",
                request.text
            )
        }
    }
}

// ─── Execution ─────────────────────────────────────────────────────────

/// Validate the action request synchronously. Returns an error result if
/// validation fails, or `None` if the request is valid.
fn validate_request(request: &AiActionRequest) -> Option<AiActionResult> {
    if request.text.trim().is_empty() && request.action != AiActionType::FindRelatedNotes {
        return Some(AiActionResult {
            result: String::new(),
            usage: RequestUsage::default(),
            error: Some("输入文本不能为空。".to_string()),
        });
    }
    None
}

/// Execute an AI quick action (non-streaming).
///
/// Returns the AI-generated result, or an error result if the AI call fails.
#[instrument(skip(settings, request))]
pub async fn execute_ai_action(
    settings: &AppSettings,
    request: &AiActionRequest,
) -> AiActionResult {
    // Validate synchronously before making the AI call
    if let Some(error_result) = validate_request(request) {
        return error_result;
    }

    let system = system_prompt(request.action);
    let prompt = user_prompt(request.action, request);

    let mut action_settings = settings.clone();
    if let Some(ref model) = request.model {
        if !model.trim().is_empty() {
            action_settings.effective_provider_mut().model = model.clone();
        }
    }

    match send_request_with_temperature(&action_settings, &system, &prompt, &[], 0.3).await {
        Ok(response) => {
            let result = response.text.trim().to_string();
            AiActionResult {
                result,
                usage: response.usage,
                error: None,
            }
        }
        Err(e) => {
            let error_msg = format!("AI 操作执行失败：{}", crate::sanitize_error(&e.to_string()));
            AiActionResult {
                result: String::new(),
                usage: RequestUsage::default(),
                error: Some(error_msg),
            }
        }
    }
}

/// List all available AI actions with their metadata.
pub fn list_ai_actions() -> Vec<serde_json::Value> {
    AiActionType::all()
        .into_iter()
        .map(|action| {
            serde_json::json!({
                "id": action.id(),
                "label": action.label(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_type_roundtrip() {
        for action in AiActionType::all() {
            let id = action.id();
            let parsed = AiActionType::from_id(id);
            assert_eq!(parsed, Some(action), "roundtrip failed for {}", id);
        }
    }

    #[test]
    fn action_type_from_id_unknown() {
        assert_eq!(AiActionType::from_id("unknown_action"), None);
    }

    #[test]
    fn action_type_from_id_alternative_names() {
        assert_eq!(
            AiActionType::from_id("continue_writing"),
            Some(AiActionType::ContinueWriting)
        );
        assert_eq!(
            AiActionType::from_id("extract_todos"),
            Some(AiActionType::ExtractTodos)
        );
        assert_eq!(
            AiActionType::from_id("find_related_notes"),
            Some(AiActionType::FindRelatedNotes)
        );
    }

    #[test]
    fn all_actions_have_labels() {
        for action in AiActionType::all() {
            assert!(!action.label().is_empty(), "label empty for {:?}", action);
            assert!(!action.id().is_empty(), "id empty for {:?}", action);
        }
    }

    #[test]
    fn empty_text_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::Summarize,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            model: None,
        };
        let result = validate_request(&request);
        assert!(result.is_some(), "empty text should fail validation");
        assert!(result.unwrap().error.unwrap().contains("空"));
    }

    #[test]
    fn whitespace_text_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::Summarize,
            text: "   ".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            model: None,
        };
        let result = validate_request(&request);
        assert!(result.is_some(), "whitespace-only text should fail validation");
    }

    #[test]
    fn valid_text_passes_validation() {
        let request = AiActionRequest {
            action: AiActionType::Summarize,
            text: "Some real content".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            model: None,
        };
        assert!(validate_request(&request).is_none(), "valid text should pass validation");
    }

    #[test]
    fn find_related_notes_does_not_require_text() {
        let request = AiActionRequest {
            action: AiActionType::FindRelatedNotes,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            model: None,
        };
        // find_related_notes is exempt from the text requirement
        assert!(validate_request(&request).is_none());
    }

    #[test]
    fn list_actions_returns_valid_list() {
        let actions = list_ai_actions();
        assert!(!actions.is_empty(), "should return at least one action");
        for action in &actions {
            assert!(action.get("id").and_then(|v| v.as_str()).is_some(), "action missing id");
            assert!(action.get("label").and_then(|v| v.as_str()).is_some(), "action missing label");
        }
    }

    #[test]
    fn system_prompt_not_empty() {
        for action in AiActionType::all() {
            let prompt = system_prompt(action);
            assert!(!prompt.is_empty(), "system prompt empty for {:?}", action);
        }
    }

    #[test]
    fn user_prompt_contains_input_text() {
        let request = AiActionRequest {
            action: AiActionType::Summarize,
            text: "Hello world".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            model: None,
        };
        let prompt = user_prompt(AiActionType::Summarize, &request);
        assert!(prompt.contains("Hello world"));
    }

    #[test]
    fn user_prompt_with_tone() {
        let request = AiActionRequest {
            action: AiActionType::Rewrite,
            text: "Some text".to_string(),
            target_language: None,
            tone: Some("vivid".to_string()),
            note_id: None,
            model: None,
        };
        let prompt = user_prompt(AiActionType::Rewrite, &request);
        assert!(prompt.contains("vivid"));
    }

    #[test]
    fn user_prompt_with_language() {
        let request = AiActionRequest {
            action: AiActionType::Translate,
            text: "Hello".to_string(),
            target_language: Some("Chinese".to_string()),
            tone: None,
            note_id: None,
            model: None,
        };
        let prompt = user_prompt(AiActionType::Translate, &request);
        assert!(prompt.contains("Chinese"));
    }
}
