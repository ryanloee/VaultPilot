pub mod mcp_client;
pub mod provider;
pub mod settings;

// Re-export all public types for backward compatibility.
pub use provider::{
    default_base_url, default_model, default_timeout_ms, ProviderConfig, ProviderType,
};
pub use settings::{
    default_auto_check_updates, default_auto_wake_enabled, default_auto_wake_end_time,
    default_auto_wake_interval_minutes, default_auto_wake_model, default_auto_wake_prompt,
    default_auto_wake_start_time, default_compression_threshold, AppSettings, ModelRoutingConfig,
};

use serde::{Deserialize, Serialize};
use std::str::FromStr;

// ---------------------------------------------------------------------------
// ResponseStyle — quick-switch answer length/depth (#1965)
// ---------------------------------------------------------------------------

/// Response style for controlling AI answer length, structure, and format.
///
/// Each style maps to a system-prompt suffix that adjusts output
/// characteristics without changing the underlying model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ResponseStyle {
    /// Concise, direct answers with key points only.
    Brief,
    /// Balanced, natural answers (default).
    #[default]
    Standard,
    /// Thorough, structured answers with full explanations and examples.
    Detailed,
}

impl ResponseStyle {
    /// Return the CLI-style string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Brief => "brief",
            Self::Standard => "standard",
            Self::Detailed => "detailed",
        }
    }
}

impl FromStr for ResponseStyle {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "brief" => Ok(Self::Brief),
            "standard" => Ok(Self::Standard),
            "detailed" => Ok(Self::Detailed),
            other => Err(format!(
                "unknown response style: '{other}'; expected 'brief', 'standard', or 'detailed'"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// AiSubscription — AI Scheduled Research subscription model (#2167)
// ---------------------------------------------------------------------------

/// A subscription represents a recurring AI-powered research task.
/// Stored as a row in the `subscriptions` SQLite table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSubscription {
    pub id: String,
    /// Human-readable name (auto-generated from prompt).
    pub name: String,
    /// Cron expression for schedule (e.g. "0 9 * * 1").
    pub schedule: String,
    /// The AI prompt template (may contain {{placeholders}}).
    pub prompt: String,
    /// Comma-separated tool names allowed (e.g. "web_search,read_note").
    pub tools: String,
    /// Target collection name for result notes.
    pub target_collection: String,
    /// Whether this subscription is active.
    pub enabled: bool,
    /// ISO-8601 timestamp of the last successful run.
    pub last_run_at: String,
    /// ISO-8601 timestamp of the next scheduled run.
    pub next_run_at: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// ISO-8601 last-update timestamp.
    pub updated_at: String,
    /// Number of times this subscription has been executed.
    pub run_count: i64,
    /// Status of the last run: "success", "failed", "running", or "".
    pub last_status: String,
    /// Error message from last failed run (empty if last was successful).
    pub last_error: String,
}

/// A single RSS/Atom/JSON Feed subscription for auto-ingestion (#3041).
///
/// Feeds are polled periodically; new entries are converted to Markdown and
/// stored as vault notes. Incremental fetching relies on the conditional
/// headers (`etag`, `last_modified`) sent on the next request plus the
/// per-feed high-water mark (`last_entry_id`, `last_entry_date`) so already
/// seen entries are not re-ingested.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FeedSubscription {
    /// Unique id (UUID).
    pub id: String,
    /// Human-readable feed title (auto-detected if left empty on create).
    pub title: String,
    /// Feed URL (RSS/Atom/JSON).
    pub url: String,
    /// Feed kind: "rss" | "atom" | "json".
    pub kind: String,
    /// Target collection name for ingested notes.
    pub collection: String,
    /// Comma-separated default tags for ingested notes.
    pub tags: String,
    /// Polling interval in minutes.
    pub interval_minutes: i64,
    /// Whether this feed is active.
    pub enabled: bool,
    /// ISO-8601 timestamp of the last successful poll.
    pub last_fetched_at: String,
    /// ETag received from the last poll (sent back as If-None-Match).
    pub etag: String,
    /// Last-Modified received from the last poll (sent back as If-Modified-Since).
    pub last_modified: String,
    /// Id of the most recent entry seen (high-water mark for dedup).
    pub last_entry_id: String,
    /// Publish date (ISO-8601) of the most recent entry seen.
    pub last_entry_date: String,
    /// Status of the last poll: "success", "failed", "skipped", or "".
    pub last_status: String,
    /// Error message from last failed poll (empty if last was successful).
    pub last_error: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// ISO-8601 last-update timestamp.
    pub updated_at: String,
}

impl Default for AiSubscription {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            schedule: String::new(),
            prompt: String::new(),
            tools: String::new(),
            target_collection: String::new(),
            enabled: true,
            last_run_at: String::new(),
            next_run_at: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            run_count: 0,
            last_status: String::new(),
            last_error: String::new(),
        }
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
    #[serde(default)]
    pub collections: Vec<String>,
}

/// A named group of notes — a flat, many-to-many organizational layer
/// separate from the filesystem folder hierarchy (#2042).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
    /// Number of notes belonging to this collection (populated by list queries).
    #[serde(default)]
    pub note_count: usize,
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
    /// Relevance score from the ranking pipeline (higher = more relevant).
    /// `None` when the document was not retrieved via search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_score: Option<i64>,
}

/// A note recommended as related to the current note, with a relevance score.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedNote {
    pub meta: NoteMeta,
    /// Relevance score — higher means more related.
    pub score: i64,
    /// Optional snippet showing why this note is related.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// A wikilink target found inside a note body — either resolved to a
/// note (when `note` is `Some`) or unresolved (dangling link).
///
/// Used by the `notes.follow_links` and `notes.backlinks` MCP tools (#1829).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikilinkRef {
    /// The raw link target text inside `[[…]]` (before any `|` alias).
    pub target: String,
    /// Optional display alias (`[[target|alias]]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Resolved note metadata if the target matches a note in the vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<NoteMeta>,
}

/// A note that links **to** a given target note (a backlink).
///
/// Used by the `notes.backlinks` MCP tool (#1829).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklinkEntry {
    /// The note that contains the `[[link]]`.
    pub meta: NoteMeta,
    /// The raw link target text inside `[[…]]`.
    pub link_target: String,
}

/// An unlinked mention: a note whose title appears as **plain text** in another
/// note's body (not wrapped in `[[ ]]` wikilinks).
///
/// Used by the `notes.unlinked_mentions` MCP tool and the Graph View (#2832) to
/// surface latent connections the user hasn't formalised into wikilinks yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlinkedMention {
    /// The note that contains the text mention (source).
    pub meta: NoteMeta,
    /// The matched title text as it appears in the body.
    pub matched_title: String,
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
    /// Whether this session has been flagged as unhealthy (repetition, loops, etc.).
    /// See `orchestration/recovery.rs` for detection logic (#3103).
    #[serde(default)]
    pub unhealthy: bool,
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

pub fn default_ai_source() -> String {
    "captured".to_string()
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
    /// When true, also kick off async vector/semantic search
    /// after the initial FTS5 keyword results (#2033).
    #[serde(default)]
    pub deep_search: bool,
}

/// A single event in the progressive search SSE stream (#2033).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressiveSearchEvent {
    /// Stage name: "keyword", "loading", "semantic", or "done"
    pub stage: String,
    /// Keyword or semantic search results (present for "keyword" and "semantic" stages)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<SearchResult>,
    /// Loading message (present for "loading" stage)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
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
    /// Relevance score (0.0–1.0) from search ranking (#1704).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
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
    /// Whether the session has been flagged as unhealthy after this exchange.
    /// Frontends should surface a recovery prompt when true (#3103).
    #[serde(default)]
    pub unhealthy: bool,
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

// ---------------------------------------------------------------------------
// Self-Organizing Vault — Feature #2176
// ---------------------------------------------------------------------------

/// Status of a weak link between two notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WeakLinkStatus {
    /// Awaiting user review.
    #[default]
    Pending,
    /// User confirmed the association.
    Confirmed,
    /// User dismissed the suggestion.
    Dismissed,
}

impl WeakLinkStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WeakLinkStatus::Pending => "pending",
            WeakLinkStatus::Confirmed => "confirmed",
            WeakLinkStatus::Dismissed => "dismissed",
        }
    }
}

impl From<String> for WeakLinkStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "confirmed" => WeakLinkStatus::Confirmed,
            "dismissed" => WeakLinkStatus::Dismissed,
            _ => WeakLinkStatus::Pending,
        }
    }
}

/// A pending association (weak link) between two notes, awaiting user
/// confirmation before becoming a real relationship.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeakLink {
    pub id: String,
    pub source_note_id: String,
    pub target_note_id: String,
    /// Type of link (e.g. "content_similarity", "topic_related", "duplicate").
    pub link_type: String,
    /// Confidence score 0.0 (low) to 1.0 (high).
    pub score: f64,
    pub status: WeakLinkStatus,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// MessageV2 — Unified cross-platform message schema (#1239)
// ---------------------------------------------------------------------------

/// Maximum serialized size of the `metadata` field (64 KB) to prevent
/// malicious payloads from bloating storage or transit.
const MESSAGE_V2_METADATA_MAX_BYTES: usize = 64 * 1024;

/// Role of the message author.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageV2Role {
    #[default]
    User,
    Assistant,
    System,
}

/// Attachment type discriminator.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageV2AttachmentType {
    Image,
    #[default]
    File,
}

/// An attachment referenced by a message.
///
/// `url` MUST use the `local://` scheme to prevent path-traversal attacks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MessageV2Attachment {
    #[serde(default, rename = "type")]
    pub kind: MessageV2AttachmentType,
    /// Resource locator. Must start with `local://`.
    #[serde(default)]
    pub url: String,
    /// MIME type (e.g. "image/png", "application/pdf").
    #[serde(default)]
    pub mime: String,
}

impl MessageV2Attachment {
    /// Returns `Ok(())` if the URL uses the `local://` scheme.
    pub fn validate_url(&self) -> Result<(), String> {
        if !self.url.starts_with("local://") {
            return Err(format!(
                "attachment url must use local:// scheme, got: {}",
                self.url
            ));
        }
        Ok(())
    }
}

/// Arbitrary key-value metadata attached to a message.
///
/// The serialized size is capped at 64 KB (see `MESSAGE_V2_METADATA_MAX_BYTES`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MessageV2Metadata {
    /// Model that generated this message (empty for user messages).
    #[serde(default)]
    pub model: String,
    /// Total token count for this message.
    #[serde(default)]
    pub tokens: u64,
    /// Provider-specific extra fields.
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

impl MessageV2Metadata {
    /// Validate that the serialized metadata does not exceed the size cap.
    pub fn validate_size(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        if bytes.len() > MESSAGE_V2_METADATA_MAX_BYTES {
            return Err(format!(
                "metadata exceeds {} byte limit (got {} bytes)",
                MESSAGE_V2_METADATA_MAX_BYTES,
                bytes.len()
            ));
        }
        Ok(())
    }
}

/// Unified cross-platform message schema.
///
/// This is the canonical wire format shared by Rust, WinUI, and Mobile.
/// See issue #1239 for the full specification.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MessageV2 {
    /// Unique message identifier (UUID).
    #[serde(default)]
    pub id: String,
    /// Who produced this message.
    #[serde(default)]
    pub role: MessageV2Role,
    /// Markdown content body.
    #[serde(default)]
    pub content: String,
    /// Attached files / images.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MessageV2Attachment>,
    /// Provider and token metadata.
    #[serde(default)]
    pub metadata: MessageV2Metadata,
    /// Reserved extension point for the plugin system.
    /// Currently unused — always `{}`.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub extensions: std::collections::HashMap<String, serde_json::Value>,
}

impl MessageV2 {
    /// Full structural validation: checks attachment URLs and metadata size.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for att in &self.attachments {
            if let Err(e) = att.validate_url() {
                errors.push(e);
            }
        }
        if let Err(e) = self.metadata.validate_size() {
            errors.push(e);
        }
        errors
    }
}

// ---------------------------------------------------------------------------
// HealthReport — Vault Health Dashboard (#2014)
// ---------------------------------------------------------------------------

/// Comprehensive health report for the vault knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    /// Total number of notes in the vault.
    pub total_notes: usize,
    /// Total number of collections.
    pub total_collections: usize,
    /// Total number of unique tags across all notes.
    pub total_tags: usize,
    /// Notes that have no tags and no wiki-links to/from other notes.
    pub orphan_notes: Vec<NoteMeta>,
    /// Knowledge density score from 0.0 (sparse) to 1.0 (dense).
    pub knowledge_density_score: f64,
    /// AI-generated suggestions for improving vault health.
    pub suggestions: Vec<String>,
    /// Groups of note IDs whose titles are highly similar (potential duplicates).
    pub duplicate_clusters: Vec<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
                collections: Vec::new(),
            },
            body: "Some content".to_string(),
            search_snippet: None,
            search_score: None,
        };
        let json = serde_json::to_string(&doc).expect("serialize");
        let parsed: NoteDocument = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.meta.id, "n1");
        assert_eq!(parsed.meta.tags, vec!["tag1"]);
        assert_eq!(parsed.body, "Some content");
    }

    #[test]
    fn weak_link_status_from_string() {
        assert_eq!(
            WeakLinkStatus::from("pending".to_string()),
            WeakLinkStatus::Pending
        );
        assert_eq!(
            WeakLinkStatus::from("confirmed".to_string()),
            WeakLinkStatus::Confirmed
        );
        assert_eq!(
            WeakLinkStatus::from("dismissed".to_string()),
            WeakLinkStatus::Dismissed
        );
        assert_eq!(
            WeakLinkStatus::from("unknown".to_string()),
            WeakLinkStatus::Pending
        );
    }

    #[test]
    fn weak_link_round_trips() {
        let link = WeakLink {
            id: "wl-1".to_string(),
            source_note_id: "n1".to_string(),
            target_note_id: "n2".to_string(),
            link_type: "content_similarity".to_string(),
            score: 0.85,
            status: WeakLinkStatus::Pending,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&link).expect("serialize");
        let parsed: WeakLink = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.id, "wl-1");
        assert_eq!(parsed.score, 0.85);
        assert_eq!(parsed.status.as_str(), "pending");
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
                        score: None,
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
                unhealthy: false,
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

        let parsed_none: SearchQuery = serde_json::from_str(&json).expect("deserialize none");
        assert!(parsed_none.limit.is_none());
        let parsed_some: SearchQuery = serde_json::from_str(&json2).expect("deserialize some");
        assert_eq!(parsed_some.limit, Some(10));
    }

    #[test]
    fn structured_note_draft_default_source_is_empty() {
        let draft = StructuredNoteDraft::default();
        assert!(draft.source.is_empty());
        let json = "{}";
        let from_json: StructuredNoteDraft = serde_json::from_str(json).expect("parse");
        assert_eq!(from_json.source, "captured");
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

    // ── MessageV2 roundtrip tests (#1239) ────────────────────────────────

    #[test]
    fn message_v2_roundtrip_user_text_only() {
        let msg = MessageV2 {
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            role: MessageV2Role::User,
            content: "Hello **world**".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("\"id\""));
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\""));
        assert!(!json.contains("\"attachments\""));
        assert!(!json.contains("\"extensions\""));

        let parsed: MessageV2 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.id, msg.id);
        assert_eq!(parsed.role, MessageV2Role::User);
        assert_eq!(parsed.content, "Hello **world**");
        assert!(parsed.attachments.is_empty());
        assert!(parsed.extensions.is_empty());
    }

    #[test]
    fn message_v2_roundtrip_assistant_with_attachment() {
        let msg = MessageV2 {
            id: "a1".to_string(),
            role: MessageV2Role::Assistant,
            content: "Here is the image:".to_string(),
            attachments: vec![MessageV2Attachment {
                kind: MessageV2AttachmentType::Image,
                url: "local://vault/images/chart.png".to_string(),
                mime: "image/png".to_string(),
            }],
            metadata: MessageV2Metadata {
                model: "deepseek-v4".to_string(),
                tokens: 42,
                extra: std::collections::HashMap::new(),
            },
            extensions: std::collections::HashMap::new(),
        };
        let json = serde_json::to_string_pretty(&msg).expect("serialize");
        let parsed: MessageV2 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].kind, MessageV2AttachmentType::Image);
        assert_eq!(parsed.attachments[0].url, "local://vault/images/chart.png");
        assert_eq!(parsed.attachments[0].mime, "image/png");
        assert_eq!(parsed.metadata.model, "deepseek-v4");
        assert_eq!(parsed.metadata.tokens, 42);
    }

    #[test]
    fn message_v2_roundtrip_system_role() {
        let msg = MessageV2 {
            role: MessageV2Role::System,
            content: "You are a helpful assistant.".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("\"role\":\"system\""));
        let parsed: MessageV2 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.role, MessageV2Role::System);
    }

    #[test]
    fn message_v2_roundtrip_with_extensions() {
        let mut ext = std::collections::HashMap::new();
        ext.insert("plugin_x".to_string(), serde_json::json!({"enabled": true}));
        let msg = MessageV2 {
            id: "ext1".to_string(),
            role: MessageV2Role::User,
            content: "test".to_string(),
            extensions: ext,
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("\"extensions\""));
        let parsed: MessageV2 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.extensions.len(), 1);
        assert_eq!(
            parsed.extensions["plugin_x"],
            serde_json::json!({"enabled": true})
        );
    }

    #[test]
    fn message_v2_attachment_rejects_non_local_url() {
        let att = MessageV2Attachment {
            kind: MessageV2AttachmentType::File,
            url: "https://evil.com/payload".to_string(),
            mime: "text/plain".to_string(),
        };
        assert!(att.validate_url().is_err());

        let good = MessageV2Attachment {
            url: "local://vault/doc.pdf".to_string(),
            ..Default::default()
        };
        assert!(good.validate_url().is_ok());
    }

    #[test]
    fn message_v2_validate_catches_bad_attachment_urls() {
        let msg = MessageV2 {
            attachments: vec![
                MessageV2Attachment {
                    url: "local://ok.png".to_string(),
                    ..Default::default()
                },
                MessageV2Attachment {
                    url: "/etc/passwd".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let errors = msg.validate();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("local://"));
    }

    #[test]
    fn message_v2_default_values() {
        let msg = MessageV2::default();
        assert!(msg.id.is_empty());
        assert_eq!(msg.role, MessageV2Role::User);
        assert!(msg.content.is_empty());
        assert!(msg.attachments.is_empty());
        assert!(msg.metadata.model.is_empty());
        assert_eq!(msg.metadata.tokens, 0);
        assert!(msg.extensions.is_empty());
        assert!(msg.validate().is_empty());
    }

    #[test]
    fn message_v2_deserializes_minimal_json() {
        let json = r#"{"content":"hi"}"#;
        let msg: MessageV2 = serde_json::from_str(json).expect("parse");
        assert_eq!(msg.content, "hi");
        assert_eq!(msg.role, MessageV2Role::User);
        assert!(msg.id.is_empty());
    }

    #[test]
    fn message_v2_role_serialization() {
        assert_eq!(
            serde_json::to_string(&MessageV2Role::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&MessageV2Role::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(
            serde_json::to_string(&MessageV2Role::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            "\"user\"",
            &serde_json::to_string(&MessageV2Role::default()).unwrap()
        );
    }

    #[test]
    fn message_v2_attachment_type_serialization() {
        assert_eq!(
            serde_json::to_string(&MessageV2AttachmentType::Image).unwrap(),
            "\"image\""
        );
        assert_eq!(
            serde_json::to_string(&MessageV2AttachmentType::File).unwrap(),
            "\"file\""
        );
    }

    #[test]
    fn message_v2_shared_fixtures_parse() {
        let raw = std::fs::read_to_string("tests/fixtures/message_v2_fixtures.json")
            .expect("fixture file must exist");
        let root: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
        let fixtures = root["fixtures"].as_array().expect("fixtures array");

        for fixture in fixtures {
            let name = fixture["name"].as_str().unwrap_or("<unnamed>");
            let json_val = &fixture["json"];
            let msg: MessageV2 = serde_json::from_value(json_val.clone())
                .unwrap_or_else(|e| panic!("fixture '{}': failed to parse MessageV2: {}", name, e));
            assert!(
                !msg.id.is_empty() || name == "empty_content",
                "fixture '{}': id should not be empty",
                name
            );
            let serialized = serde_json::to_string(&msg).expect("serialize");
            let reparsed: MessageV2 = serde_json::from_str(&serialized).expect("roundtrip");
            assert_eq!(reparsed.role, msg.role, "fixture '{}': role mismatch", name);
            assert_eq!(
                reparsed.content, msg.content,
                "fixture '{}': content mismatch",
                name
            );
        }
    }

    #[test]
    fn default_ai_source_value() {
        assert_eq!(default_ai_source(), "captured");
    }
}
