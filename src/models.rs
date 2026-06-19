use serde::{Deserialize, Serialize};

/// The type of AI provider, used to select correct API headers and endpoint format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    /// Anthropic Messages API (x-api-key header, /v1/messages endpoint).
    #[default]
    Anthropic,
    /// OpenAI-compatible Chat Completions API (Bearer token, /v1/chat/completions endpoint).
    OpenAi,
}

impl ProviderType {
    /// Auto-detect provider type from the base URL.
    ///
    /// URLs containing "anthropic" → Anthropic; everything else → OpenAI
    /// (since OpenAI-compatible is the most common generic format).
    pub fn from_base_url(base_url: &str) -> Self {
        let lower = base_url.to_ascii_lowercase();
        if lower.contains("anthropic") {
            Self::Anthropic
        } else {
            Self::OpenAi
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default)]
    pub context_window_tokens: Option<usize>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// Explicit provider type override. When `None`, auto-detected from `base_url`.
    #[serde(default)]
    pub provider_type: Option<ProviderType>,
}

impl ProviderConfig {
    /// Return a clone with the API key masked for safe serialization.
    pub fn masked(&self) -> Self {
        Self {
            api_key: mask_secret(&self.api_key),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            request_timeout_ms: self.request_timeout_ms,
            context_window_tokens: self.context_window_tokens,
            max_output_tokens: self.max_output_tokens,
            provider_type: self.provider_type,
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: default_base_url(),
            model: default_model(),
            request_timeout_ms: default_timeout_ms(),
            context_window_tokens: None,
            max_output_tokens: None,
            provider_type: None,
        }
    }
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("api_key", &mask_secret(&self.api_key))
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("provider_type", &self.provider_type)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub vault_dir: String,
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default = "default_auto_check_updates")]
    pub auto_check_updates: bool,
    #[serde(default = "default_auto_wake_enabled")]
    pub auto_wake_enabled: bool,
    #[serde(default = "default_auto_wake_interval_minutes")]
    pub auto_wake_interval_minutes: u64,
    #[serde(default = "default_auto_wake_model")]
    pub auto_wake_model: String,
    #[serde(default = "default_auto_wake_start_time")]
    pub auto_wake_start_time: String,
    #[serde(default = "default_auto_wake_end_time")]
    pub auto_wake_end_time: String,
    /// Prompt sent to the AI when auto-wake fires (#861).
    #[serde(default = "default_auto_wake_prompt")]
    pub auto_wake_prompt: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            vault_dir: String::new(),
            provider: ProviderConfig::default(),
            auto_check_updates: default_auto_check_updates(),
            auto_wake_enabled: default_auto_wake_enabled(),
            auto_wake_interval_minutes: default_auto_wake_interval_minutes(),
            auto_wake_model: default_auto_wake_model(),
            auto_wake_start_time: default_auto_wake_start_time(),
            auto_wake_end_time: default_auto_wake_end_time(),
            auto_wake_prompt: default_auto_wake_prompt(),
        }
    }
}

impl ProviderConfig {
    /// Return the effective provider type, using the explicit override if set,
    /// otherwise auto-detecting from the base URL.
    pub fn effective_provider_type(&self) -> ProviderType {
        self.provider_type
            .unwrap_or_else(|| ProviderType::from_base_url(&self.base_url))
    }

    /// Validate provider configuration, returning a list of error messages.
    /// An empty list means the configuration is valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Validate base_url is a valid HTTP(S) URL (if non-empty).
        let url = self.base_url.trim();
        if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
            errors.push(format!(
                "provider.base_url must be an HTTP or HTTPS URL, got: {}",
                self.base_url
            ));
        }

        // Validate request_timeout_ms is in a reasonable range (1s to 10min).
        if self.request_timeout_ms < 1_000 {
            errors.push(format!(
                "provider.request_timeout_ms is too low ({}ms); minimum is 1000ms",
                self.request_timeout_ms
            ));
        } else if self.request_timeout_ms > 600_000 {
            errors.push(format!(
                "provider.request_timeout_ms is too high ({}ms); maximum is 600000ms",
                self.request_timeout_ms
            ));
        }

        errors
    }
}

impl AppSettings {
    /// Validate settings after deserialization, returning all error messages at once.
    /// An empty list means the settings are valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Validate vault_dir exists and is a directory (if non-empty).
        let vault = self.vault_dir.trim();
        if !vault.is_empty() {
            let path = std::path::Path::new(vault);
            if !path.exists() {
                errors.push(format!("vault_dir does not exist: {}", self.vault_dir));
            } else if !path.is_dir() {
                errors.push(format!("vault_dir is not a directory: {}", self.vault_dir));
            }
        }

        // Validate api_key is non-empty.
        if self.provider.api_key.trim().is_empty() {
            errors.push("provider.api_key is empty; an API key is required".to_string());
        }

        // Delegate provider-specific validation.
        errors.extend(self.provider.validate());

        errors
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NoteMeta {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub board: String,
    #[serde(default)]
    pub kernel: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NoteDocument {
    #[serde(default)]
    pub meta: NoteMeta,
    #[serde(default)]
    pub body: String,
    /// FTS5-generated snippet with `==highlight==` markers around matched terms.
    /// `None` when the note was not returned from a text search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurn {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub citations: Vec<AnswerCitation>,
    #[serde(default)]
    pub saved_note: Option<NoteMeta>,
    #[serde(default)]
    pub thinking_trace: Option<ThinkingTrace>,
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
    #[serde(default)]
    pub created_at: String,
    /// Origin of this turn: empty for manual, "scheduled_wake" for auto-wake (#861).
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub covered_turn_count: usize,
    #[serde(default)]
    pub compression_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub turns: Vec<ChatTurn>,
    #[serde(default)]
    pub summary: Option<ConversationSummary>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatState {
    #[serde(default)]
    pub current_session_id: String,
    #[serde(default)]
    pub sessions: Vec<ChatSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionOverview {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub turn_count: usize,
    #[serde(default)]
    pub has_summary: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StructuredNoteDraft {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub board: String,
    #[serde(default)]
    pub kernel: String,
    #[serde(default)]
    pub status: String,
    #[serde(default = "default_ai_source")]
    pub source: String,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub limit: Option<usize>,
    /// Number of results to skip (for pagination). Defaults to 0.
    pub offset: Option<usize>,
    /// Filter notes created on or after this ISO-8601 timestamp.
    pub created_after: Option<String>,
    /// Filter notes created on or before this ISO-8601 timestamp.
    pub created_before: Option<String>,
    /// Filter notes modified on or after this ISO-8601 timestamp.
    pub modified_after: Option<String>,
    /// Filter notes modified on or before this ISO-8601 timestamp.
    pub modified_before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(default)]
    pub notes: Vec<NoteMeta>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnswerCitation {
    #[serde(default)]
    pub note_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GroundedAnswer {
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub citations: Vec<AnswerCitation>,
    #[serde(default)]
    pub saved_note: Option<NoteMeta>,
    #[serde(default)]
    pub thinking_trace: Option<ThinkingTrace>,
    #[serde(default)]
    pub context_status: Option<ContextStatus>,
    pub used_context_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatExchangeResult {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub session_title: String,
    #[serde(default)]
    pub created_session: bool,
    #[serde(default)]
    pub answer: GroundedAnswer,
    #[serde(default)]
    pub state: ChatState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ContextStatus {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub context_window_tokens: usize,
    #[serde(default)]
    pub live_tokens: usize,
    #[serde(default)]
    pub threshold_tokens: usize,
    #[serde(default)]
    pub threshold_percent: u8,
    #[serde(default)]
    pub usage_percent: f64,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub precise: bool,
    #[serde(default)]
    pub last_request_input_tokens: Option<usize>,
    #[serde(default)]
    pub last_request_output_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusEvent {
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingTrace {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub steps: Vec<ThinkingTraceStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingTraceStep {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub exported: usize,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VaultExportResult {
    /// Number of notes exported
    pub notes_exported: usize,
    /// Number of chat sessions exported
    pub sessions_exported: usize,
    /// Path to the output zip file
    pub output_path: String,
    /// Size of the zip file in bytes
    pub file_size_bytes: u64,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub scanned: usize,
    pub indexed: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiSkill {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub guardrails: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiWorkflowManual {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub skills: Vec<AiSkill>,
}

/// Mask a secret string for safe display: show first 4 and last 4 chars.
fn mask_secret(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 12 {
        if chars.is_empty() {
            return String::new();
        }
        return "*".repeat(chars.len());
    }
    let prefix: String = chars[..4].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{}…{}", prefix, suffix)
}

pub fn default_base_url() -> String {
    "https://api.anthropic.com/v1/messages".to_string()
}

pub fn default_model() -> String {
    "claude-3-5-sonnet-latest".to_string()
}

pub fn default_timeout_ms() -> u64 {
    60_000
}

pub fn default_ai_source() -> String {
    "captured".to_string()
}

pub fn default_auto_check_updates() -> bool {
    true
}

pub fn default_auto_wake_enabled() -> bool {
    false
}

pub fn default_auto_wake_interval_minutes() -> u64 {
    30
}

pub fn default_auto_wake_model() -> String {
    String::new()
}

pub fn default_auto_wake_start_time() -> String {
    String::new()
}

pub fn default_auto_wake_end_time() -> String {
    String::new()
}

pub fn default_auto_wake_prompt() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_settings_round_trips_with_camel_case() {
        let settings = AppSettings {
            vault_dir: "D:\\Vault".to_string(),
            provider: ProviderConfig {
                api_key: "test-key".to_string(),
                base_url: "https://api.example.com".to_string(),
                model: "test-model".to_string(),
                request_timeout_ms: 30_000,
                context_window_tokens: Some(128_000),
                max_output_tokens: Some(16384),
                provider_type: None,
            },
            auto_check_updates: false,
            auto_wake_enabled: true,
            auto_wake_interval_minutes: 60,
            auto_wake_model: "claude-3-5-haiku-latest".to_string(),
            auto_wake_start_time: "05:00".to_string(),
            auto_wake_end_time: "23:00".to_string(),
            auto_wake_prompt: String::new(),
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        assert!(json.contains("\"vaultDir\""));
        assert!(json.contains("\"apiKey\""));
        assert!(json.contains("\"baseUrl\""));
        assert!(json.contains("\"requestTimeoutMs\""));
        assert!(json.contains("\"contextWindowTokens\""));
        assert!(json.contains("\"maxOutputTokens\""));
        assert!(json.contains("\"autoCheckUpdates\""));
        assert!(json.contains("\"autoWakeEnabled\""));
        assert!(json.contains("\"autoWakeIntervalMinutes\""));
        assert!(json.contains("\"autoWakeModel\""));
        assert!(json.contains("\"autoWakeStartTime\""));
        assert!(json.contains("\"autoWakeEndTime\""));

        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.vault_dir, settings.vault_dir);
        assert_eq!(parsed.provider.api_key, settings.provider.api_key);
        assert_eq!(parsed.provider.context_window_tokens, Some(128_000));
        assert_eq!(parsed.provider.max_output_tokens, Some(16384));
    }

    #[test]
    fn note_document_round_trips_all_fields() {
        let doc = NoteDocument {
            meta: NoteMeta {
                id: "n1".to_string(),
                title: "Test Note".to_string(),
                tags: vec!["tag1".to_string()],
                keywords: vec!["kw1".to_string()],
                platform: "arm".to_string(),
                board: "evk".to_string(),
                kernel: "5.10".to_string(),
                status: "active".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-02T00:00:00Z".to_string(),
                source: "manual".to_string(),
                path: "/vault/note.md".to_string(),
                summary: "A test note".to_string(),
            },
            body: "Some content".to_string(),
            search_snippet: None,
        };
        let json = serde_json::to_string(&doc).expect("serialize");
        let parsed: NoteDocument = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.meta.id, "n1");
        assert_eq!(parsed.meta.tags, vec!["tag1"]);
        assert_eq!(parsed.body, "Some content");
    }

    #[test]
    fn chat_state_round_trips_with_nested_sessions() {
        let state = ChatState {
            current_session_id: "s1".to_string(),
            sessions: vec![ChatSession {
                id: "s1".to_string(),
                title: "Session One".to_string(),
                turns: vec![ChatTurn {
                    id: "t1".to_string(),
                    role: "user".to_string(),
                    text: "hello".to_string(),
                    citations: vec![AnswerCitation {
                        note_id: "n1".to_string(),
                        title: "Note".to_string(),
                        path: "/n.md".to_string(),
                        snippet: "snippet".to_string(),
                    }],
                    saved_note: None,
                    thinking_trace: Some(ThinkingTrace {
                        summary: "thought".to_string(),
                        steps: vec![ThinkingTraceStep {
                            title: "step".to_string(),
                            detail: "detail".to_string(),
                        }],
                    }),
                    attachments: vec![],
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    source: String::new(),
                }],
                summary: Some(ConversationSummary {
                    text: "summary".to_string(),
                    generated_at: "2026-01-01T00:00:00Z".to_string(),
                    covered_turn_count: 2,
                    compression_count: 1,
                }),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            }],
        };
        let json = serde_json::to_string(&state).expect("serialize");
        assert!(json.contains("\"currentSessionId\""));
        assert!(json.contains("\"thinkingTrace\""));
        assert!(json.contains("\"coveredTurnCount\""));

        let parsed: ChatState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.current_session_id, "s1");
        assert_eq!(parsed.sessions[0].turns[0].citations.len(), 1);
        assert!(parsed.sessions[0].summary.is_some());
    }

    #[test]
    fn grounded_answer_handles_optional_fields() {
        let with_none = GroundedAnswer {
            answer: "ok".to_string(),
            citations: vec![],
            saved_note: None,
            thinking_trace: None,
            context_status: None,
            used_context_count: 0,
        };
        let json = serde_json::to_string(&with_none).expect("serialize");
        let parsed: GroundedAnswer = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.saved_note.is_none());
        assert!(parsed.thinking_trace.is_none());

        let with_some = GroundedAnswer {
            answer: "ok".to_string(),
            citations: vec![],
            saved_note: Some(NoteMeta::default()),
            thinking_trace: Some(ThinkingTrace {
                summary: "s".to_string(),
                steps: vec![],
            }),
            context_status: Some(ContextStatus {
                model: "m".to_string(),
                context_window_tokens: 100,
                live_tokens: 50,
                threshold_tokens: 95,
                threshold_percent: 95,
                usage_percent: 50.0,
                source: "test".to_string(),
                precise: true,
                last_request_input_tokens: Some(50),
                last_request_output_tokens: Some(100),
            }),
            used_context_count: 3,
        };
        let json2 = serde_json::to_string(&with_some).expect("serialize");
        let parsed2: GroundedAnswer = serde_json::from_str(&json2).expect("deserialize");
        assert!(parsed2.saved_note.is_some());
        assert!(parsed2.thinking_trace.is_some());
        assert_eq!(parsed2.used_context_count, 3);
    }

    #[test]
    fn search_query_handles_optional_limit() {
        let no_limit = SearchQuery {
            text: "test".to_string(),
            tags: vec![],
            keywords: vec![],
            limit: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&no_limit).expect("serialize");
        // serde serializes Option<T> as null when None
        assert!(json.contains("\"limit\":null"));

        let with_limit = SearchQuery {
            text: "test".to_string(),
            tags: vec![],
            keywords: vec![],
            limit: Some(10),
            ..Default::default()
        };
        let json2 = serde_json::to_string(&with_limit).expect("serialize");
        assert!(json2.contains("\"limit\":10"));

        // Round-trip both
        let parsed_none: SearchQuery = serde_json::from_str(&json).expect("deserialize none");
        assert!(parsed_none.limit.is_none());
        let parsed_some: SearchQuery = serde_json::from_str(&json2).expect("deserialize some");
        assert_eq!(parsed_some.limit, Some(10));
    }

    #[test]
    fn structured_note_draft_default_source_is_empty() {
        let draft = StructuredNoteDraft::default();
        // Default trait gives empty string; "captured" comes from serde default
        assert!(draft.source.is_empty());
        // Verify serde default when deserializing
        let json = "{}";
        let from_json: StructuredNoteDraft = serde_json::from_str(json).expect("parse");
        assert_eq!(from_json.source, "captured");
    }

    #[test]
    fn default_values_are_correct() {
        let settings = AppSettings::default();
        assert!(settings.vault_dir.is_empty());
        assert_eq!(settings.provider.base_url, default_base_url());
        assert_eq!(settings.provider.model, default_model());
        assert_eq!(settings.provider.request_timeout_ms, default_timeout_ms());
        assert!(settings.provider.context_window_tokens.is_none());
        assert!(settings.auto_check_updates);
        assert!(!settings.auto_wake_enabled);
        assert_eq!(settings.auto_wake_interval_minutes, 30);
        assert!(settings.auto_wake_model.is_empty());
        assert!(settings.auto_wake_start_time.is_empty());
        assert!(settings.auto_wake_end_time.is_empty());
        assert!(settings.auto_wake_prompt.is_empty());
        assert_eq!(default_model(), "claude-3-5-sonnet-latest");
        assert_eq!(default_timeout_ms(), 60_000);
        assert_eq!(default_ai_source(), "captured");
        assert!(default_auto_check_updates());
    }

    #[test]
    fn import_result_and_index_stats_serialize() {
        let import = ImportResult {
            imported: 3,
            skipped: 1,
            errors: vec!["err".to_string()],
        };
        let json = serde_json::to_string(&import).expect("serialize");
        assert!(json.contains("\"imported\":3"));
        assert!(json.contains("\"skipped\":1"));

        let stats = IndexStats {
            scanned: 10,
            indexed: 8,
            removed: 2,
        };
        let json2 = serde_json::to_string(&stats).expect("serialize");
        assert!(json2.contains("\"indexed\":8"));
    }

    #[test]
    fn validate_accepts_valid_settings() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "sk-test-key".to_string(),
                base_url: "https://api.anthropic.com/v1/messages".to_string(),
                request_timeout_ms: 60_000,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        // vault_dir is empty so it's skipped; api_key + base_url + timeout are valid
        assert!(settings.validate().is_empty());
    }

    #[test]
    fn validate_catches_empty_api_key() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: String::new(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors.iter().any(|e| e.contains("api_key")));
    }

    #[test]
    fn validate_catches_whitespace_only_api_key() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "   ".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors.iter().any(|e| e.contains("api_key")));
    }

    #[test]
    fn validate_catches_invalid_base_url_scheme() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                base_url: "ftp://example.com".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors.iter().any(|e| e.contains("base_url")));
    }

    #[test]
    fn validate_accepts_http_base_url() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                base_url: "http://localhost:8080/v1".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        // Only non-base_url errors should appear
        assert!(!errors.iter().any(|e| e.contains("base_url")));
    }

    #[test]
    fn validate_catches_timeout_too_low() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                request_timeout_ms: 500,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors
            .iter()
            .any(|e| e.contains("request_timeout_ms") && e.contains("too low")));
    }

    #[test]
    fn validate_catches_timeout_too_high() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                request_timeout_ms: 999_999,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors
            .iter()
            .any(|e| e.contains("request_timeout_ms") && e.contains("too high")));
    }

    #[test]
    fn validate_catches_nonexistent_vault_dir() {
        let settings = AppSettings {
            vault_dir: "/nonexistent/path/that/does/not/exist".to_string(),
            provider: ProviderConfig {
                api_key: "key".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors
            .iter()
            .any(|e| e.contains("vault_dir") && e.contains("not exist")));
    }

    #[test]
    fn validate_returns_all_errors_at_once() {
        let settings = AppSettings {
            vault_dir: "/nonexistent/path".to_string(),
            provider: ProviderConfig {
                api_key: String::new(),
                base_url: "ftp://bad".to_string(),
                request_timeout_ms: 0,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        // Should have errors for: vault_dir, api_key, base_url, timeout
        assert!(
            errors.len() >= 4,
            "expected at least 4 errors, got: {}",
            errors.len()
        );
        assert!(errors.iter().any(|e| e.contains("vault_dir")));
        assert!(errors.iter().any(|e| e.contains("api_key")));
        assert!(errors.iter().any(|e| e.contains("base_url")));
        assert!(errors.iter().any(|e| e.contains("request_timeout_ms")));
    }
}
