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
//! let note = create_meeting_note(&context, &result)?;
//! ```

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use crate::ai::client::{send_request_with_temperature, RequestUsage};
use crate::capture::handle_capture;
use crate::models::{AppSettings, NoteDocument, NoteMeta, ProviderConfig};
use crate::people_index::PersonAliasMap;
use crate::storage::{save_note_with_context, StorageContext};

/// Join separator for annotated transcript segments.
///
/// Must be used in ALL places where `annotated_transcript` is built
/// (`diarize_transcript`, `map_speakers_to_people`, `annotated_transcript_to_string`).
/// A shared constant prevents the separator drift regression seen in #3622 / #3709.
const ANNOTATED_TRANSCRIPT_SEP: &str = "\n\n";

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

/// A single speaker-labeled segment of a diarized transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizedSegment {
    /// Speaker identifier (e.g. "Speaker A", or a resolved name like "Alice").
    pub speaker: String,
    /// The text spoken by this speaker in this segment.
    pub text: String,
}

/// Result of speaker diarization: a list of segments each tagged with a speaker label.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizationResult {
    /// Segments in chronological order, each annotated with a speaker.
    pub segments: Vec<DiarizedSegment>,
    /// The full annotated transcript (with speaker labels inline).
    pub annotated_transcript: String,
    /// Raw speaker names extracted (pre-mapping to vault people).
    pub raw_speakers: Vec<String>,
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

    // #4072/#4085: an undecryptable ENC blob must never be sent to the
    // provider — see src/ai/client.rs for the matching guard on the chat path.
    if crate::crypto::is_encrypted(api_key) {
        return Err(anyhow::anyhow!(
            "API key is unavailable: stored key cannot be decrypted (machine key changed?). \
             Re-enter the key in Settings"
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

// ── Speaker Diarization (#3588) ────────────────────────────────────────────

/// Use the LLM to identify speakers in a raw transcript and annotate it with speaker labels.
///
/// The LLM is prompted to split the transcript into segments, assigning a speaker label
/// to each (e.g. "Speaker A", "Speaker B", or inferred names like "Alice"). This does
/// **not** use audio-based diarization; it relies on the LLM's ability to infer speaker
/// changes from conversational context.
///
/// Returns the diarized result with raw speaker labels (not yet mapped to vault people).
#[instrument(skip(settings))]
pub async fn diarize_transcript(
    transcript: &str,
    settings: &AppSettings,
) -> Result<DiarizationResult> {
    let system = "You are a meeting transcription assistant. Your task is to annotate a raw \
                  transcript with speaker labels. Analyze the conversation flow to identify \
                  distinct speakers and label them consistently.\n\
                  Return ONLY valid JSON with no markdown fences or extra text.";

    let user_prompt = format!(
        r#"Analyze the following meeting transcript and annotate it with speaker labels.

Transcript:
```
{transcript}
```

Identify distinct speakers from the conversation. Label speakers consistently —
use real names if they can be inferred from context (e.g. "Alice", "Bob"), otherwise
use "Speaker A", "Speaker B", etc. Do NOT create more distinct speakers than the
conversation warrants.

Respond with a JSON object:
{{
  "segments": [
    {{ "speaker": "Speaker A", "text": "Hello, let's start the meeting." }},
    {{ "speaker": "Speaker B", "text": "Sounds good, what's on the agenda?" }}
  ],
  "rawSpeakers": ["Speaker A", "Speaker B"]
}}

IMPORTANT: The "rawSpeakers" list should contain the UNIQUE speaker labels used in the
segments, in order of first appearance.
Output ONLY valid JSON — no markdown fences, no extra text."#,
        transcript = transcript
    );

    let response = send_request_with_temperature(settings, system, &user_prompt, &[], 0.1)
        .await
        .context("LLM call for speaker diarization failed")?;

    // Try to extract JSON from the response (handles possible markdown fences)
    let json_text = extract_json_from_response(&response.text).ok_or_else(|| {
        anyhow::anyhow!(
            "model did not return valid JSON for diarization. Response: {}",
            crate::sanitize_error(&response.text.chars().take(300).collect::<String>())
        )
    })?;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DiarizationResponse {
        segments: Vec<DiarizedSegment>,
        #[serde(default)]
        raw_speakers: Vec<String>,
    }

    let parsed: DiarizationResponse = serde_json::from_str(&json_text).with_context(|| {
        format!(
            "failed to parse diarization JSON. First 200 chars: {}",
            crate::sanitize_error(&json_text.chars().take(200).collect::<String>())
        )
    })?;

    if parsed.segments.is_empty() {
        anyhow::bail!("model returned a diarization result with no segments");
    }

    // Build the annotated transcript from the segments
    let annotated_transcript = parsed
        .segments
        .iter()
        .map(|seg| format!("{}: {}", seg.speaker, seg.text))
        .collect::<Vec<_>>()
        .join(ANNOTATED_TRANSCRIPT_SEP);

    Ok(DiarizationResult {
        segments: parsed.segments,
        annotated_transcript,
        raw_speakers: parsed.raw_speakers,
    })
}

/// Map raw speaker labels (e.g. "Speaker A", "Alice") to known vault people
/// using the alias map from the people index.
///
/// Speaker labels that match a known person (or alias) are replaced with the
/// canonical name. Unknown speakers are left as-is.
pub fn map_speakers_to_people(result: &mut DiarizationResult, alias_map: &PersonAliasMap) {
    for segment in &mut result.segments {
        let resolved = alias_map.resolve(&segment.speaker);
        if resolved != segment.speaker {
            segment.speaker = resolved.clone();
        }
    }

    // Also resolve the raw_speakers list
    result.raw_speakers = result
        .raw_speakers
        .iter()
        .map(|s| alias_map.resolve(s))
        .collect();

    // Rebuild the annotated transcript with resolved speaker names
    result.annotated_transcript = result
        .segments
        .iter()
        .map(|seg| format!("{}: {}", seg.speaker, seg.text))
        .collect::<Vec<_>>()
        .join(ANNOTATED_TRANSCRIPT_SEP);
}

/// Build the annotated transcript string from diarized segments.
pub fn annotated_transcript_to_string(segments: &[DiarizedSegment]) -> String {
    segments
        .iter()
        .map(|seg| format!("{}: {}", seg.speaker, seg.text))
        .collect::<Vec<_>>()
        .join(ANNOTATED_TRANSCRIPT_SEP)
}

// ── Create Meeting Note ───────────────────────────────────────────────────

/// Construct the markdown title and body for a meeting note.
/// Returns `(note_title, body)`.
fn format_meeting_note_body(
    result: &MeetingTranscriptionResult,
    date_str: &str,
) -> (String, String) {
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

    (note_title, body)
}

/// Save a structured Markdown meeting note to the vault.
///
/// Builds a [`NoteDocument`] containing a well-formatted meeting summary in
/// Markdown, saves it via [`save_note_with_context`], and returns the saved note.
#[instrument(skip(context, result))]
pub fn create_meeting_note(
    context: &StorageContext,
    result: &MeetingTranscriptionResult,
) -> Result<NoteDocument> {
    let now = Utc::now();
    let date_str = result
        .summary
        .date
        .as_deref()
        .unwrap_or(&now.format("%Y-%m-%d").to_string())
        .to_string();

    let (note_title, body) = format_meeting_note_body(result, &date_str);

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
        search_score: None,
    };

    let saved = save_note_with_context(context, note)?;
    Ok(saved)
}

// ── Voice Note Capture (#2012) ────────────────────────────────────────────

/// `source` marker written onto every voice-note capture (#2012).
pub const VOICE_NOTE_SOURCE: &str = "voice";

/// Tag applied to every voice-note capture.
pub const VOICE_NOTE_TAG: &str = "voice";

/// Maximum number of characters taken from the transcript when deriving a
/// voice-note title — keeps the title / filename readable.
const VOICE_NOTE_MAX_TITLE_LEN: usize = 60;

/// Result of capturing a voice note: the saved note's id, its title, and the
/// raw transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceNoteResult {
    /// Id of the note saved into the vault.
    pub note_id: String,
    /// Title of the saved note.
    pub title: String,
    /// The transcribed text.
    pub transcript: String,
}

/// Derive a human-readable note title from a transcript.
///
/// Uses the first non-empty line of the transcript, truncated to
/// [`VOICE_NOTE_MAX_TITLE_LEN`] characters (an ellipsis is appended when
/// truncation occurs). Returns `None` when the transcript contains no
/// meaningful text — callers should then fall back to a timestamp-based title.
fn derive_voice_note_title(transcript: &str) -> Option<String> {
    let first_line = transcript
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let truncated: String = first_line.chars().take(VOICE_NOTE_MAX_TITLE_LEN).collect();
    if first_line.chars().count() > VOICE_NOTE_MAX_TITLE_LEN {
        Some(format!("{truncated}…"))
    } else {
        Some(truncated)
    }
}

/// Build the note body for a voice-note capture.
///
/// Voice notes are raw captures (no AI summary / action items, unlike meeting
/// notes), so the body is simply the transcript, trimmed and normalized.
fn format_voice_note_body(transcript: &str) -> String {
    transcript.trim().to_string()
}

/// Build and save a voice-note capture from an already-transcribed transcript.
///
/// This is the synchronous note-building half of [`transcribe_voice_note`],
/// factored out so it can be unit-tested without invoking the speech-to-text
/// provider. It creates a regular note tagged with `source = "voice"` and
/// returns the saved note.
#[instrument(skip(context))]
pub fn create_voice_note(
    context: &StorageContext,
    transcript: &str,
    title_override: Option<&str>,
) -> Result<NoteDocument> {
    let now = Utc::now();
    let note_title = title_override
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .or_else(|| derive_voice_note_title(transcript))
        .unwrap_or_else(|| format!("语音笔记 — {}", now.format("%Y-%m-%d %H:%M")));

    let body = format_voice_note_body(transcript);
    let note = NoteDocument {
        meta: NoteMeta {
            id: Uuid::new_v4().to_string(),
            title: note_title,
            tags: vec![VOICE_NOTE_TAG.to_string()],
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            source: VOICE_NOTE_SOURCE.to_string(),
            summary: transcript.chars().take(200).collect(),
            ..Default::default()
        },
        body,
        search_snippet: None,
        search_score: None,
    };

    let saved = save_note_with_context(context, note)?;
    Ok(saved)
}

/// Transcribe an audio file and save it as a voice note in the vault (#2012).
///
/// This is a generic voice-note capture: it reuses the same Whisper-based
/// speech-to-text path as [`transcribe_audio`], but — unlike
/// [`create_meeting_note`] — performs **no** AI summarization. The transcript
/// is saved verbatim as a regular note tagged with `source = "voice"`.
///
/// `title_override` (when non-empty) is used verbatim; otherwise the title is
/// derived from the first line of the transcript, falling back to a
/// timestamped placeholder when the transcript is empty.
#[instrument(skip(provider_config, context, settings))]
pub async fn transcribe_voice_note(
    audio_path: &str,
    provider_config: &ProviderConfig,
    language: Option<&str>,
    context: &StorageContext,
    title_override: Option<&str>,
    settings: &AppSettings,
    cleanup: bool,
) -> Result<VoiceNoteResult> {
    // 1. Transcribe via the shared Whisper provider path.
    let transcript = transcribe_audio(audio_path, provider_config, language).await?;

    // 2. Optional AI cleanup — run the CleanUp action on the raw transcript
    //    to fix typos, improve structure, and add headings/lists (#3536).
    let final_transcript = if cleanup {
        let request = crate::ai::actions::AiActionRequest {
            action: crate::ai::actions::AiActionType::CleanUp,
            text: transcript.clone(),
            target_language: language.map(|l| l.to_string()),
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = crate::ai::actions::execute_ai_action(settings, &request).await;
        if let Some(ref err) = result.error {
            tracing::warn!("AI cleanup failed (falling back to raw transcript): {err}");
            transcript.clone()
        } else {
            result.result
        }
    } else {
        transcript.clone()
    };

    // 3. Persist (sync file/SQLite I/O → spawn_blocking). Note-building is
    //    delegated to create_voice_note so it stays unit-testable.
    let ctx = context.clone();
    let final_owned = final_transcript.clone();
    let title_owned = title_override.map(|s| s.to_string());
    let saved = tokio::task::spawn_blocking(move || {
        create_voice_note(&ctx, &final_owned, title_owned.as_deref())
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))??;

    Ok(VoiceNoteResult {
        note_id: saved.meta.id,
        title: saved.meta.title,
        transcript: final_transcript,
    })
}

/// Transcribe audio and append the transcript as a capture entry in a target
/// note (daily/inbox), rather than saving it as a standalone voice note (#3333).
///
/// This bridges voice capture with the existing text-capture flow: after
/// transcription, the transcript is appended via [`handle_capture`] so it
/// appears in the user's daily note or inbox under the specified section.
///
/// Returns the same [`VoiceNoteResult`] shape (note_id refers to the target
/// note, not a newly-created one).
#[instrument(skip(provider_config, context, settings))]
#[allow(clippy::too_many_arguments)]
pub async fn transcribe_and_capture_to_target(
    audio_path: &str,
    provider_config: &ProviderConfig,
    language: Option<&str>,
    context: &StorageContext,
    target: &str,
    section: &str,
    settings: &AppSettings,
    cleanup: bool,
) -> Result<VoiceNoteResult> {
    // 1. Transcribe via the shared Whisper provider path.
    let transcript = transcribe_audio(audio_path, provider_config, language).await?;

    // 2. Optional AI cleanup (#3536).
    let final_transcript = if cleanup {
        let request = crate::ai::actions::AiActionRequest {
            action: crate::ai::actions::AiActionType::CleanUp,
            text: transcript.clone(),
            target_language: language.map(|l| l.to_string()),
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = crate::ai::actions::execute_ai_action(settings, &request).await;
        if let Some(ref err) = result.error {
            tracing::warn!("AI cleanup failed (falling back to raw transcript): {err}");
            transcript.clone()
        } else {
            result.result
        }
    } else {
        transcript.clone()
    };

    let trimmed = final_transcript.trim();
    if trimmed.is_empty() {
        anyhow::bail!("voice capture produced empty transcript");
    }

    // 2. Append to the target note via handle_capture (sync, on the blocking
    //    pool) so the transcript lands in the daily note / inbox section.
    let ctx = context.clone();
    let target_owned = target.to_string();
    let section_owned = section.to_string();
    let transcript_owned = trimmed.to_string();
    let result = tokio::task::spawn_blocking(move || {
        handle_capture(&ctx, &transcript_owned, &target_owned, &section_owned)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))??;

    let note_id = result["note_id"].as_str().unwrap_or("unknown").to_string();

    Ok(VoiceNoteResult {
        note_id,
        title: format!(
            "🎤 Voice capture → {}",
            trimmed.chars().take(60).collect::<String>()
        ),
        transcript: final_transcript,
    })
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

    // ── Tests for create_meeting_note / format_meeting_note_body ─────────

    /// Helper to create a basic MeetingTranscriptionResult for testing.
    fn make_test_result() -> MeetingTranscriptionResult {
        MeetingTranscriptionResult {
            transcript: "This is the meeting transcript with important details.".to_string(),
            summary: MeetingSummary {
                title: "Sprint Review".to_string(),
                date: Some("2024-07-01".to_string()),
                attendees: vec!["Alice".to_string(), "Bob".to_string()],
                summary: "Reviewed sprint progress and planned next iteration.".to_string(),
                key_decisions: vec!["Ship v2.0 next week".to_string()],
                action_items: vec![MeetingActionItem {
                    description: "Update changelog".to_string(),
                    assignees: vec!["Alice".to_string()],
                    due_date: Some("2024-07-05".to_string()),
                }],
                next_steps: vec!["Schedule retro".to_string()],
            },
            usage: RequestUsage::default(),
            note_path: None,
        }
    }

    #[test]
    fn format_body_empty_attendees() {
        let mut result = make_test_result();
        result.summary.attendees.clear();

        let (note_title, body) = format_meeting_note_body(&result, "2024-07-01");

        assert_eq!(note_title, "Meeting: Sprint Review");
        assert!(
            !body.contains("**Attendees:**"),
            "body should not contain attendees line"
        );
    }

    #[test]
    fn format_body_empty_key_decisions() {
        let mut result = make_test_result();
        result.summary.key_decisions.clear();

        let (_, body) = format_meeting_note_body(&result, "2024-07-01");

        assert!(
            !body.contains("## Key Decisions"),
            "body should not contain Key Decisions section"
        );
    }

    #[test]
    fn format_body_empty_action_items() {
        let mut result = make_test_result();
        result.summary.action_items.clear();

        let (_, body) = format_meeting_note_body(&result, "2024-07-01");

        assert!(
            !body.contains("## Action Items"),
            "body should not contain Action Items section"
        );
    }

    #[test]
    fn format_body_action_items_empty_assignees_and_due_date() {
        let mut result = make_test_result();
        result.summary.action_items = vec![
            MeetingActionItem {
                description: "Fix bug".to_string(),
                assignees: vec![],
                due_date: None,
            },
            MeetingActionItem {
                description: "Deploy release".to_string(),
                assignees: vec!["Charlie".to_string()],
                due_date: Some("2024-07-10".to_string()),
            },
        ];

        let (_, body) = format_meeting_note_body(&result, "2024-07-01");

        // First row: empty assignees and due_date should use em-dash
        assert!(body.contains("| 1 | Fix bug | — | — |"));
        // Second row: normal
        assert!(body.contains("| 2 | Deploy release | Charlie | 2024-07-10 |"));
    }

    #[test]
    fn format_body_empty_next_steps() {
        let mut result = make_test_result();
        result.summary.next_steps.clear();

        let (_, body) = format_meeting_note_body(&result, "2024-07-01");

        assert!(
            !body.contains("## Next Steps"),
            "body should not contain Next Steps section"
        );
    }

    #[test]
    fn format_body_full_transcript_section() {
        let result = make_test_result();

        let (_, body) = format_meeting_note_body(&result, "2024-07-01");

        assert!(body.contains("## Raw Transcript"));
        assert!(body.contains("<details>"));
        assert!(body.contains("This is the meeting transcript with important details."));
        assert!(body.contains("</details>"));
    }

    #[test]
    fn format_body_title_fallback() {
        let mut result = make_test_result();
        result.summary.title = "   ".to_string(); // empty-ish title

        let (note_title, body) = format_meeting_note_body(&result, "2024-07-01");

        assert_eq!(note_title, "Meeting Notes — 2024-07-01");
        assert!(body.starts_with("# Meeting Notes — 2024-07-01\n"));
    }

    #[test]
    fn create_meeting_note_date_fallback() {
        use std::fs;

        use crate::storage::StorageContext;

        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-meeting-note-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);

        let mut result = make_test_result();
        result.summary.date = None;
        result.summary.title.clear(); // so the fallback date appears in the title

        let saved = create_meeting_note(&ctx, &result).expect("create_meeting_note should succeed");

        // Title should contain today's date fallback
        let today = Utc::now().format("%Y-%m-%d").to_string();
        assert!(
            saved.meta.title.contains(&today)
                || saved.meta.title == format!("Meeting Notes — {}", today),
            "title should contain fallback date: {}",
            saved.meta.title
        );

        // Body should contain today's date
        assert!(
            saved.body.contains(&today),
            "body should contain fallback date"
        );

        // Cleanup
        let _ = fs::remove_dir_all(&temp);
    }

    // ── Regression: transcribe_and_capture_to_target (#3333) ───────────

    #[tokio::test]
    async fn transcribe_and_capture_to_target_rejects_nonexistent_audio() {
        // Verify the function exists and fails on I/O (file not found)
        // before reaching any transcription / storage logic.
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-voice-capture-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = crate::storage::StorageContext::for_test(&temp);

        let provider = crate::models::ProviderConfig::default();

        let settings = crate::models::AppSettings::default();

        let result = transcribe_and_capture_to_target(
            "/nonexistent/audio/file.wav",
            &provider,
            None,
            &ctx,
            "daily",
            "Voice Capture",
            &settings,
            false,
        )
        .await;

        assert!(result.is_err(), "should fail — file does not exist");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("No such file")
                || err.contains("failed to read")
                || err.contains("audio file")
                || err.contains("NotFound")
                || err.contains("Cannot find")
                || err.contains("system cannot find"),
            "error should mention file I/O: got: {err}"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// Regression test for #4085: an undecryptable ENC:v1: blob must never be
    /// sent to the Whisper API. The function must return an error mentioning
    /// "Re-enter the key in Settings" before making any HTTP request.
    #[tokio::test]
    async fn transcribe_audio_rejects_encrypted_api_key() {
        // Create a small non-empty temp audio file so we get past the
        // existence/empty-file checks and actually reach the API-key guard.
        let temp_dir = std::env::temp_dir().join(format!(
            "vaultpilot-test-enc-key-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp_dir).expect("temp dir");
        let audio_path = temp_dir.join("audio.mp3");
        std::fs::write(&audio_path, b"fake-audio-bytes").expect("write audio");

        let provider = ProviderConfig {
            api_key: "ENC:v1:fake".to_string(),
            ..ProviderConfig::default()
        };

        let result = transcribe_audio(audio_path.to_str().unwrap(), &provider, None).await;

        assert!(result.is_err(), "should reject encrypted API key");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Re-enter the key in Settings"),
            "error should mention re-entering the key, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    // ── Speaker Diarization Tests (#3588) ──────────────────────────────

    #[test]
    fn diarized_segment_serialization_roundtrip() {
        let segment = DiarizedSegment {
            speaker: "Alice".to_string(),
            text: "Let's review the Q3 results.".to_string(),
        };
        let json = serde_json::to_string(&segment).unwrap();
        let deserialized: DiarizedSegment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.speaker, "Alice");
        assert_eq!(deserialized.text, "Let's review the Q3 results.");
    }

    #[test]
    fn diarization_result_defaults() {
        let result = DiarizationResult {
            segments: vec![
                DiarizedSegment {
                    speaker: "Speaker A".to_string(),
                    text: "Hello".to_string(),
                },
                DiarizedSegment {
                    speaker: "Speaker B".to_string(),
                    text: "Hi there".to_string(),
                },
            ],
            annotated_transcript: "Speaker A: Hello\nSpeaker B: Hi there".to_string(),
            raw_speakers: vec!["Speaker A".to_string(), "Speaker B".to_string()],
        };
        assert_eq!(result.segments.len(), 2);
        assert_eq!(result.raw_speakers.len(), 2);
        assert!(!result.annotated_transcript.is_empty());
    }

    #[test]
    fn map_speakers_to_people_resolves_known_speakers() {
        let mut aliases = PersonAliasMap::new();
        aliases.add_alias("老王", "王明");
        aliases.add_alias("Speaker A", "Alice");
        aliases.add_alias("Speaker B", "Bob");

        let mut result = DiarizationResult {
            segments: vec![
                DiarizedSegment {
                    speaker: "Speaker A".to_string(),
                    text: "Let's start.".to_string(),
                },
                DiarizedSegment {
                    speaker: "Speaker B".to_string(),
                    text: "I agree.".to_string(),
                },
                DiarizedSegment {
                    speaker: "老王".to_string(),
                    text: "没问题。".to_string(),
                },
            ],
            annotated_transcript: String::new(),
            raw_speakers: vec![
                "Speaker A".to_string(),
                "Speaker B".to_string(),
                "老王".to_string(),
            ],
        };

        map_speakers_to_people(&mut result, &aliases);

        // Speaker A → Alice
        assert_eq!(result.segments[0].speaker, "Alice");
        // Speaker B → Bob
        assert_eq!(result.segments[1].speaker, "Bob");
        // 老王 → 王明
        assert_eq!(result.segments[2].speaker, "王明");

        // raw_speakers should also be resolved
        assert_eq!(result.raw_speakers, vec!["Alice", "Bob", "王明"]);

        // annotated_transcript should be rebuilt with resolved names
        assert!(result.annotated_transcript.contains("Alice:"));
        assert!(result.annotated_transcript.contains("Bob:"));
        assert!(result.annotated_transcript.contains("王明:"));
    }

    #[test]
    fn map_speakers_to_people_leaves_unknown_unchanged() {
        let aliases = PersonAliasMap::new(); // empty — no aliases registered

        let mut result = DiarizationResult {
            segments: vec![DiarizedSegment {
                speaker: "Unknown Person".to_string(),
                text: "Hello world".to_string(),
            }],
            annotated_transcript: String::new(),
            raw_speakers: vec!["Unknown Person".to_string()],
        };

        map_speakers_to_people(&mut result, &aliases);

        // Unknown speaker stays as-is (trimmed)
        assert_eq!(result.segments[0].speaker, "Unknown Person");
        assert_eq!(result.raw_speakers[0], "Unknown Person");
    }

    #[test]
    fn annotated_transcript_to_string_formats_correctly() {
        let segments = vec![
            DiarizedSegment {
                speaker: "Alice".to_string(),
                text: "First point.".to_string(),
            },
            DiarizedSegment {
                speaker: "Bob".to_string(),
                text: "Second point.".to_string(),
            },
        ];
        let output = annotated_transcript_to_string(&segments);
        assert_eq!(output, "Alice: First point.\n\nBob: Second point.");
    }

    /// Regression test for #3622: verify that map_speakers_to_people()
    /// uses the same join separator ("\n\n") as diarize_transcript(),
    /// so annotated_transcript is consistent regardless of whether
    /// speaker-to-person mapping was applied.
    #[test]
    fn annotated_transcript_separator_consistent_after_map_speakers() {
        let mut aliases = PersonAliasMap::new();
        aliases.add_alias("Speaker A", "Alice");
        aliases.add_alias("Speaker B", "Bob");

        // Simulate what diarize_transcript() produces
        let mut result = DiarizationResult {
            segments: vec![
                DiarizedSegment {
                    speaker: "Speaker A".to_string(),
                    text: "Hello there".to_string(),
                },
                DiarizedSegment {
                    speaker: "Speaker B".to_string(),
                    text: "Nice to meet you".to_string(),
                },
            ],
            annotated_transcript: "Speaker A: Hello there\n\nSpeaker B: Nice to meet you"
                .to_string(),
            raw_speakers: vec!["Speaker A".to_string(), "Speaker B".to_string()],
        };

        let before_mapping = result.annotated_transcript.clone();

        map_speakers_to_people(&mut result, &aliases);

        // After mapping, the segment count is unchanged
        let after_segments = result.segments.len();
        assert_eq!(after_segments, 2, "segment count must not change");

        // The annotated_transcript after mapping must use "\n\n" as separator,
        // consistent with what diarize_transcript() produces
        assert!(
            result.annotated_transcript.contains("\n\n"),
            "annotated_transcript after map_speakers should use \\n\\n separator, got: {:?}",
            result.annotated_transcript
        );
        assert!(
            !result.annotated_transcript.contains("\n\n\n"),
            "annotated_transcript should not contain triple newlines"
        );

        // Before mapping already used "\n\n" (as diarize_transcript does)
        assert!(
            before_mapping.contains("\n\n"),
            "before-mapping annotated_transcript must use \\n\\n separator"
        );

        // Both strings should have the exact same paragraph structure
        let before_paras: Vec<&str> = before_mapping.split("\n\n").collect();
        let after_paras: Vec<&str> = result.annotated_transcript.split("\n\n").collect();
        assert_eq!(
            before_paras.len(),
            after_paras.len(),
            "paragraph count must be identical before and after speaker mapping"
        );
    }

    /// Regression test for #3709: verify that the join separator is consistent
    /// across all annotated-transcript-building functions. Since
    /// `diarize_transcript()` requires an LLM call and cannot be unit-tested,
    /// we instead verify the invariant at the data level: all three functions
    /// must use the same `ANNOTATED_TRANSCRIPT_SEP` constant.
    #[test]
    fn annotated_transcript_separator_constant_drift_detection() {
        // The separator constant must be "\n\n" — single newline would be a regression.
        assert_eq!(
            ANNOTATED_TRANSCRIPT_SEP, "\n\n",
            "ANNOTATED_TRANSCRIPT_SEP must remain \"\\n\\n\" to stay consistent \
             across all transcript-building functions (#3622 / #3709)"
        );

        // map_speakers_to_people and annotated_transcript_to_string must agree
        let segments = vec![
            DiarizedSegment {
                speaker: "Alice".to_string(),
                text: "First.".to_string(),
            },
            DiarizedSegment {
                speaker: "Bob".to_string(),
                text: "Second.".to_string(),
            },
        ];

        let direct = annotated_transcript_to_string(&segments);

        let mut result = DiarizationResult {
            segments: segments.clone(),
            annotated_transcript: String::new(),
            raw_speakers: vec!["Alice".to_string(), "Bob".to_string()],
        };
        map_speakers_to_people(&mut result, &PersonAliasMap::new());

        // The separator in the mapped result must match the direct build
        assert_eq!(
            result.annotated_transcript, direct,
            "map_speakers_to_people and annotated_transcript_to_string must \
             produce identical output for the same segments (separator drift)"
        );

        // Count separators — exactly one between two segments
        let sep_count = direct.matches(ANNOTATED_TRANSCRIPT_SEP).count();
        assert_eq!(
            sep_count, 1,
            "two segments joined should have exactly one separator occurrence"
        );
    }
}
