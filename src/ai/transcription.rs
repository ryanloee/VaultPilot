//! Meeting Transcription & AI Summary Engine (#2072).
//!
//! Provides audio transcription via the OpenAI Whisper API (`/v1/audio/transcriptions`)
//! and AI-powered meeting summary generation using the existing LLM client.
//! Supports saving structured meeting notes as Markdown into the VaultPilot vault.
//!
//! # Usage
//!
//! ```ignore
//! let transcript = transcribe_audio("meeting.mp3", &provider_config).await?;
//! let summary = generate_meeting_summary(&transcript, &settings).await?;
//! let note = create_meeting_note(&context, &settings, &result).await?;
//! ```

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use crate::ai::client::{send_request_with_temperature, RequestUsage};
use crate::models::{AppSettings, NoteDocument, NoteMeta, ProviderConfig};
use crate::storage::{save_note_with_context, StorageContext};

// ── Data types ────────────────────────────────────────────────────────────

/// A single action item extracted from a meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingActionItem {
    /// Description of the action item.
    pub description: String,
    /// Person(s) assigned to this action item.
    #[serde(default)]
    pub assignees: Vec<String>,
    /// Optional due date or deadline.
    #[serde(default)]
    pub due_date: Option<String>,
}

/// Structured summary of a meeting, produced by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    /// Meeting title (extracted or inferred from the transcript).
    pub title: String,
    /// Date of the meeting (as stated in the transcript, or today if unknown).
    #[serde(default)]
    pub date: Option<String>,
    /// List of attendees mentioned in the meeting.
    #[serde(default)]
    pub attendees: Vec<String>,
    /// Overall summary / key points discussed.
    pub summary: String,
    /// Key decisions made during the meeting.
    #[serde(default)]
    pub key_decisions: Vec<String>,
    /// Action items with assignees.
    #[serde(default)]
    pub action_items: Vec<MeetingActionItem>,
    /// Next steps or follow-up items.
    #[serde(default)]
    pub next_steps: Vec<String>,
}

/// Complete result of a meeting transcription and summary workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTranscriptionResult {
    /// The raw transcribed text from the audio file.
    pub transcript: String,
    /// The AI-generated structured summary.
    pub summary: MeetingSummary,
    /// Token usage from the summary generation LLM call.
    #[serde(default)]
    pub usage: RequestUsage,
    /// Path to the saved note in the vault (populated after saving).
    #[serde(default)]
    pub note_path: Option<String>,
}

// ── Transcribe Audio ──────────────────────────────────────────────────────

/// Transcribe an audio file to text using the OpenAI Whisper API.
///
/// Sends a `POST /v1/audio/transcriptions` request with the audio file
/// as `multipart/form-data`. Only the `whisper-1` model is used.
///
/// `language` is an optional [ISO 639-1](https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes)
/// language code (e.g. `"en"`, `"zh"`) to hint the model.
///
/// The `provider_config` provides the API key and base URL for the request.
/// Only OpenAI-compatible provider configurations are supported for Whisper.
#[instrument(skip(provider_config))]
pub async fn transcribe_audio(
    audio_path: &str,
    provider_config: &ProviderConfig,
    language: Option<&str>,
) -> Result<String> {
    let audio_path = Path::new(audio_path);
    if !audio_path.exists() {
        return Err(anyhow::anyhow!(
            "audio file does not exist: {}",
            audio_path.display()
        ));
    }

    // Read the audio file into memory
    let audio_data = tokio::fs::read(audio_path)
        .await
        .with_context(|| format!("failed to read audio file: {}", audio_path.display()))?;

    if audio_data.is_empty() {
        return Err(anyhow::anyhow!(
            "audio file is empty: {}",
            audio_path.display()
        ));
    }

    let api_key = provider_config.api_key.trim();
    if api_key.is_empty() {
        return Err(anyhow::anyhow!(
            "API key is empty — cannot call Whisper API"
        ));
    }

    // Build the endpoint URL following the same pattern as the TTS module
    let base = provider_config.base_url.trim_end_matches('/');
    let endpoint = if base.contains("/v1") {
        format!("{}/audio/transcriptions", base)
    } else {
        format!("{}/v1/audio/transcriptions", base)
    };

    // Determine MIME type from file extension
    let extension = audio_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp3")
        .to_lowercase();

    let mime_type = match extension.as_str() {
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    };

    let file_name = audio_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio")
        .to_string();

    // Build multipart form: file + model (+ optional language)
    let file_part = reqwest::multipart::Part::bytes(audio_data)
        .file_name(file_name)
        .mime_str(mime_type)
        .context("invalid MIME type for audio file")?;

    let mut form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", "whisper-1");

    if let Some(lang) = language {
        let lang = lang.trim();
        if !lang.is_empty() {
            form = form.text("language", lang.to_string());
        }
    }

    // Build a dedicated HTTP client with a longer timeout for audio uploads
    let timeout_ms = std::cmp::max(provider_config.request_timeout_ms, 120_000);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .context("failed to build HTTP client for Whisper API")?;

    let response = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .send()
        .await
        .context("Whisper API request failed")?;

    if !response.status().is_success() {
        let status = response.status();
        let err_text = response
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_string());
        return Err(anyhow::anyhow!(
            "Whisper API returned {}: {}",
            status,
            crate::sanitize_error(&err_text)
        ));
    }

    // Parse the JSON response: { "text": "..." }
    #[derive(Deserialize)]
    struct WhisperResponse {
        text: String,
    }

    let whisper_resp: WhisperResponse = response
        .json()
        .await
        .context("failed to parse Whisper API response")?;

    let transcript = whisper_resp.text.trim().to_string();
    if transcript.is_empty() {
        return Err(anyhow::anyhow!("Whisper API returned an empty transcript"));
    }

    Ok(transcript)
}

// ── Generate Meeting Summary ──────────────────────────────────────────────

/// Use the LLM to extract a structured meeting summary from a transcript.
///
/// The LLM is prompted to produce a JSON object conforming to [`MeetingSummary`].
/// A low temperature (0.1) is used for more deterministic/extractable output.
#[instrument(skip(settings))]
pub async fn generate_meeting_summary(
    transcript: &str,
    settings: &AppSettings,
) -> Result<MeetingSummary> {
    let system = "You are a meeting summarization assistant. Extract a structured \
                   summary from the meeting transcript provided by the user. \
                   Output ONLY valid JSON with no markdown fences or extra text.";

    let user_prompt = format!(
        r#"Extract a structured meeting summary from the following transcript.

Transcript:
```
{transcript}
```

Respond with a JSON object in the following schema (camelCase keys):
{{
  "title": "Meeting title (descriptive, inferred from content)",
  "date": "Date mentioned in the meeting, or null if unknown",
  "attendees": ["List of attendee names mentioned"],
  "summary": "Concise summary of what was discussed (2-5 sentences)",
  "keyDecisions": ["Key decisions made during the meeting"],
  "actionItems": [
    {{ "description": "What needs to be done",
       "assignees": ["Who is responsible"],
       "dueDate": "Optional due date or deadline" }}
  ],
  "nextSteps": ["Follow-up items or next steps"]
}}

If no attendees, decisions, action items, or next steps are mentioned, use empty arrays.
Set date to null if no date is mentioned.
Output ONLY valid JSON — no markdown fences, no extra text."#,
        transcript = transcript
    );

    let response = send_request_with_temperature(settings, system, &user_prompt, &[], 0.1)
        .await
        .context("LLM call for meeting summary generation failed")?;

    // Try to extract JSON from the response (handles possible markdown fences)
    let json_text = extract_json_from_response(&response.text).ok_or_else(|| {
        anyhow::anyhow!(
            "model did not return valid JSON for meeting summary. Response: {}",
            crate::sanitize_error(&response.text.chars().take(300).collect::<String>())
        )
    })?;

    let summary: MeetingSummary = serde_json::from_str(&json_text).with_context(|| {
        format!(
            "failed to parse meeting summary JSON. First 200 chars: {}",
            crate::sanitize_error(&json_text.chars().take(200).collect::<String>())
        )
    })?;

    if summary.title.trim().is_empty() || summary.summary.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "model returned a meeting summary with empty title or summary"
        ));
    }

    Ok(summary)
}

/// Try to extract a JSON object from the model response text.
///
/// Handles responses wrapped in ```json ... ``` fences or standalone JSON.
fn extract_json_from_response(text: &str) -> Option<String> {
    let text = text.trim();

    // Try ```json ... ``` fence
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            let json = after[..end].trim();
            if !json.is_empty() {
                return Some(json.to_string());
            }
        }
    }

    // Try ``` ... ``` (language-agnostic)
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            let json = after[..end].trim();
            if json.starts_with('{') {
                return Some(json.to_string());
            }
        }
    }

    // If the whole text looks like JSON
    if text.starts_with('{') && text.ends_with('}') {
        return Some(text.to_string());
    }

    None
}

// ── Create Meeting Note ───────────────────────────────────────────────────

/// Save a structured Markdown meeting note to the vault.
///
/// Builds a [`NoteDocument`] containing a well-formatted meeting summary in
/// Markdown, saves it via [`save_note_with_context`], and returns the saved note.
#[instrument(skip(context, _settings, result))]
pub fn create_meeting_note(
    context: &StorageContext,
    _settings: &AppSettings,
    result: &MeetingTranscriptionResult,
) -> Result<NoteDocument> {
    let now = Utc::now();
    let date_str = result
        .summary
        .date
        .as_deref()
        .unwrap_or(&now.format("%Y-%m-%d").to_string())
        .to_string();

    let title = result.summary.title.trim();
    let note_title = if title.is_empty() {
        format!("Meeting Notes — {}", date_str)
    } else {
        format!("Meeting: {}", title)
    };

    let mut body = String::new();

    // Header
    body.push_str(&format!("# {}\n\n", note_title));
    body.push_str(&format!("**Date:** {}  \n", date_str));

    // Attendees
    if !result.summary.attendees.is_empty() {
        body.push_str("**Attendees:** ");
        body.push_str(&result.summary.attendees.join(", "));
        body.push_str("  \n");
    }
    body.push('\n');

    // Summary
    body.push_str("## Summary\n\n");
    body.push_str(&result.summary.summary);
    body.push_str("\n\n");

    // Key Decisions
    if !result.summary.key_decisions.is_empty() {
        body.push_str("## Key Decisions\n\n");
        for decision in &result.summary.key_decisions {
            body.push_str(&format!("- {}\n", decision));
        }
        body.push('\n');
    }

    // Action Items
    if !result.summary.action_items.is_empty() {
        body.push_str("## Action Items\n\n");
        body.push_str("| # | Description | Assignees | Due Date |\n");
        body.push_str("|---|-------------|-----------|----------|\n");
        for (i, item) in result.summary.action_items.iter().enumerate() {
            let desc = &item.description;
            let assignees = if item.assignees.is_empty() {
                "—".to_string()
            } else {
                item.assignees.join(", ")
            };
            let due = item.due_date.as_deref().unwrap_or("—");
            body.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                i + 1,
                desc,
                assignees,
                due
            ));
        }
        body.push('\n');
    }

    // Next Steps
    if !result.summary.next_steps.is_empty() {
        body.push_str("## Next Steps\n\n");
        for step in &result.summary.next_steps {
            body.push_str(&format!("- [ ] {}\n", step));
        }
        body.push('\n');
    }

    // Full transcript section (collapsed)
    body.push_str("## Raw Transcript\n\n");
    body.push_str("<details>\n\n");
    body.push_str(&result.transcript);
    body.push_str("\n\n</details>\n");

    let note = NoteDocument {
        meta: NoteMeta {
            id: Uuid::new_v4().to_string(),
            title: note_title,
            tags: vec!["meeting".to_string(), "transcription".to_string()],
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            source: "transcription".to_string(),
            summary: result.summary.summary.chars().take(200).collect(),
            ..Default::default()
        },
        body,
        search_snippet: None,
    };

    let saved = save_note_with_context(context, note)?;
    Ok(saved)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_handles_fenced_json() {
        let text = "```json\n{\"title\": \"Test\"}\n```";
        let extracted = extract_json_from_response(text);
        assert!(extracted.is_some());
        let json = extracted.unwrap();
        assert!(json.contains("\"title\""));
        assert!(json.contains("\"Test\""));
    }

    #[test]
    fn extract_json_handles_bare_json() {
        let text = r#"{"title":"Test","summary":"Something"}"#;
        let extracted = extract_json_from_response(text);
        assert!(extracted.is_some());
        let json = extracted.unwrap();
        assert!(json.contains("\"title\""));
    }

    #[test]
    fn extract_json_handles_fence_without_language() {
        let text = "```\n{\"title\": \"Test\"}\n```";
        let extracted = extract_json_from_response(text);
        assert!(extracted.is_some());
    }

    #[test]
    fn extract_json_returns_none_for_non_json() {
        assert!(extract_json_from_response("just some text").is_none());
    }

    #[test]
    fn extract_json_skips_fence_without_json_start() {
        let text = "```\njust text\n```";
        assert!(extract_json_from_response(text).is_none());
    }

    #[test]
    fn meeting_summary_defaults_roundtrip() {
        let summary = MeetingSummary {
            title: "Sprint Review".to_string(),
            date: Some("2024-07-01".to_string()),
            attendees: vec!["Alice".to_string(), "Bob".to_string()],
            summary: "Reviewed sprint progress.".to_string(),
            key_decisions: vec!["Ship v2.0 next week".to_string()],
            action_items: vec![MeetingActionItem {
                description: "Update changelog".to_string(),
                assignees: vec!["Alice".to_string()],
                due_date: Some("2024-07-05".to_string()),
            }],
            next_steps: vec!["Schedule retro".to_string()],
        };
        let json = serde_json::to_string(&summary).unwrap();
        let deserialized: MeetingSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.title, "Sprint Review");
        assert_eq!(deserialized.action_items.len(), 1);
        assert_eq!(deserialized.action_items[0].assignees[0], "Alice");
    }

    #[test]
    fn meeting_action_item_defaults() {
        let item = MeetingActionItem {
            description: "Do something".to_string(),
            assignees: vec![],
            due_date: None,
        };
        assert!(item.assignees.is_empty());
        assert!(item.due_date.is_none());
    }

    #[test]
    fn meeting_transcription_result_defaults() {
        let result = MeetingTranscriptionResult {
            transcript: "hello".to_string(),
            summary: MeetingSummary {
                title: "Test".to_string(),
                date: None,
                attendees: vec![],
                summary: "A test meeting.".to_string(),
                key_decisions: vec![],
                action_items: vec![],
                next_steps: vec![],
            },
            usage: RequestUsage::default(),
            note_path: None,
        };
        assert!(result.note_path.is_none());
        assert_eq!(result.transcript, "hello");
    }
}
