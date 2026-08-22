//! Audio Overview — AI-powered podcast audio from vault notes.
//!
//! Mirrors Google NotebookLM's Audio Overview feature:
//! 1. **Step A**: Use the existing LLM to structure notes into a podcast script
//!    (dual-host conversational format with topic/question/answer rounds).
//! 2. **Step B**: Call a TTS service to synthesize dual-voice audio and save to
//!    `attachments/audio/` in the vault.
//! 3. **Step C**: Generate an index note with embedded audio player + full script
//!    + source note wikilinks.
//!
//! ## Formats
//! - **Deep Dive** (default): dual-host, multi-topic conversation (~5-15 min).
//! - **The Brief**: single-host, under 2 min summary.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use tracing::instrument;
use uuid::Uuid;

use crate::ai::client::{send_request, RequestUsage};
use crate::ai::tts::{self, TtsConfig};
use crate::models::{AppSettings, NoteDocument, NoteMeta};
use crate::storage::{self, StorageContext};

// ── Format enum ──────────────────────────────────────────────────────────

/// Audio Overview output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioOverviewFormat {
    /// Dual-host deep-dive conversation (default).
    #[default]
    DeepDive,
    /// Single-host concise summary under 2 minutes.
    Brief,
}

impl AudioOverviewFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DeepDive => "deep-dive",
            Self::Brief => "brief",
        }
    }

    /// Human-readable label for display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::DeepDive => "Deep Dive",
            Self::Brief => "The Brief",
        }
    }

    /// Whether this format uses dual-host (two voices).
    pub fn is_dual_host(&self) -> bool {
        matches!(self, Self::DeepDive)
    }
}

impl std::str::FromStr for AudioOverviewFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "deep-dive" | "deep_dive" | "DeepDive" | "deep dive" => Ok(Self::DeepDive),
            "brief" | "Brief" | "the-brief" | "the_brief" => Ok(Self::Brief),
            _ => Err(format!("unknown AudioOverviewFormat variant: {s}")),
        }
    }
}

// ── Request / Response types ─────────────────────────────────────────────

/// Input parameters for generating an audio overview.
#[derive(Debug, Clone)]
pub struct AudioOverviewRequest {
    /// Collection ID or folder path — notes to include.
    pub source: AudioOverviewSource,
    /// Output format.
    pub format: AudioOverviewFormat,
    /// Optional TTS config override (uses defaults if not set).
    pub tts_config: Option<TtsConfig>,
    /// Optional model override for script generation.
    pub model: Option<String>,
    /// Language hint (e.g. "zh-CN", "en-US"). Auto-detected if empty.
    pub language: Option<String>,
}

/// How to select notes for the audio overview.
#[derive(Debug, Clone)]
pub enum AudioOverviewSource {
    /// A collection ID — fetch all notes in that collection.
    Collection { id: String },
    /// A specific folder path within the vault.
    Folder { path: String },
    /// Explicit list of note IDs.
    NoteIds { ids: Vec<String> },
}

/// Result of an audio overview generation.
#[derive(Debug, Clone)]
pub struct AudioOverviewResult {
    /// Path to the generated audio file (relative to vault).
    pub audio_path: String,
    /// Path to the generated index note (relative to vault).
    pub note_path: String,
    /// The full podcast script text.
    pub script: String,
    /// Source notes used (with their paths for wikilinks).
    pub source_notes: Vec<NoteMeta>,
    /// Duration of the audio in seconds (approximate).
    pub duration_secs: f64,
    /// Format used.
    pub format: AudioOverviewFormat,
    /// Token usage from script generation.
    pub usage: RequestUsage,
}

// ── Podcast script types ─────────────────────────────────────────────────

/// A single dialog turn in the podcast script.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DialogTurn {
    /// Speaker label: "Host A", "Host B", or "Host" (for Brief format).
    pub speaker: String,
    /// The spoken text.
    pub text: String,
    /// Optional cited note paths for this turn.
    #[serde(default)]
    pub citations: Vec<String>,
}

/// The full podcast script structure.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PodcastScript {
    /// Title of the episode.
    pub title: String,
    /// Brief description.
    pub description: String,
    /// Dialog turns.
    pub dialogs: Vec<DialogTurn>,
}

// ── Main generation function ─────────────────────────────────────────────

/// Generate an Audio Overview from vault notes.
///
/// This is the main entry point. It:
/// 1. Loads notes from the given source
/// 2. Calls the LLM to produce a podcast script (Step A)
/// 3. Calls TTS to synthesize audio (Step B)
/// 4. Writes an index note with embedded player + script (Step C)
#[instrument(skip(context, settings, request))]
pub async fn generate_audio_overview(
    context: &StorageContext,
    settings: &AppSettings,
    request: &AudioOverviewRequest,
) -> Result<AudioOverviewResult> {
    // ── Step 0: Load source notes ──────────────────────────────────────────
    let vault_dir = PathBuf::from(&settings.vault_dir);
    let notes = load_source_notes(context, &request.source, &vault_dir).await?;
    if notes.is_empty() {
        return Err(anyhow::anyhow!(
            "no notes found for the given source — Audio Overview needs at least one note"
        ));
    }

    // Load the full content of each note using the existing async wrapper
    let mut docs: Vec<NoteDocument> = Vec::with_capacity(notes.len());
    for meta in &notes {
        match storage::load_note_async(context, &meta.id).await {
            Ok(doc) => docs.push(doc),
            Err(e) => tracing::warn!("failed to load note '{}': {}", meta.id, e),
        }
    }
    if docs.is_empty() {
        return Err(anyhow::anyhow!(
            "failed to load any note content from the given source"
        ));
    }

    // ── Step A: Generate podcast script ────────────────────────────────────
    let script = generate_podcast_script(settings, &docs, request, &notes).await?;

    // ── Step B: Synthesize audio ──────────────────────────────────────────
    let audio_result = synthesize_podcast_audio(settings, &script, request).await?;

    // ── Step C: Write audio file and index note ───────────────────────────
    let vault_dir = PathBuf::from(&settings.vault_dir);
    let audio_rel = save_audio_file(&vault_dir, &audio_result.audio_data, &request.format).await?;
    let note_path = write_index_note(context, &vault_dir, &script, &audio_rel, &notes).await?;

    Ok(AudioOverviewResult {
        audio_path: audio_rel,
        note_path,
        script: render_script_to_text(&script),
        source_notes: notes,
        duration_secs: audio_result.duration_secs,
        format: request.format,
        usage: RequestUsage::default(),
    })
}

// ── Step A: Script generation ────────────────────────────────────────────

/// Call the LLM to generate a podcast script from the given notes.
#[instrument(skip(settings, docs, request, all_notes))]
async fn generate_podcast_script(
    settings: &AppSettings,
    docs: &[NoteDocument],
    request: &AudioOverviewRequest,
    all_notes: &[NoteMeta],
) -> Result<PodcastScript> {
    let mut llm_settings = settings.clone();
    if let Some(ref model) = request.model {
        if !model.trim().is_empty() {
            llm_settings.effective_provider_mut().model = model.clone();
        }
    }

    let system_prompt = build_script_system_prompt(request);
    let user_prompt = build_script_user_prompt(docs, all_notes);

    let response = send_request(&llm_settings, &system_prompt, &user_prompt, &[])
        .await
        .context("LLM call for podcast script generation failed")?;

    let script = parse_script_response(&response.text).context(
        "failed to parse podcast script from LLM response — expected JSON with title, description, and dialogs array",
    )?;

    Ok(script)
}

/// Build the system prompt for podcast script generation.
fn build_script_system_prompt(request: &AudioOverviewRequest) -> String {
    let date = Utc::now().format("%Y-%m-%d");
    let (format_desc, format_instructions) = match request.format {
        AudioOverviewFormat::DeepDive => (
            "Deep Dive — a dual-host in-depth conversation",
            "\
- Use TWO hosts: Host A (the lead) and Host B (the co-host).
- Host A introduces topics, asks questions, and guides the discussion.
- Host B provides analysis, examples, and connects topics.
- The conversation should feel natural, engaging, and explore connections between notes.
- Each turn should cite the source note path(s) it draws from.
- Include 5-10 dialog rounds covering the main themes.
- Target length: 5-15 minutes of spoken content (roughly 800-2500 words total).",
        ),
        AudioOverviewFormat::Brief => (
            "The Brief — a single-host concise summary under 2 minutes",
            "\
- Use ONE host (just 'Host').
- Summarize the key points from all source notes concisely.
- Prioritize the most important insights.
- Target length: under 2 minutes (roughly 250-350 words total).
- Include source note citations where relevant.",
        ),
    };

    format!(
        r#"You are a podcast script writer for "VaultPilot Audio Overview".
Date: {date}

Format: {format_desc}

## Rules
- Base the script STRICTLY on the provided vault note content. Do not invent facts.
- Cite source notes by their title or path in each turn.
- Use natural, conversational language — this is a spoken podcast, not a report.
- Output ONLY a valid JSON object with no markdown fences or extra text.
- The JSON schema is:
  {{
    "title": "Episode title (catchy but descriptive)",
    "description": "One-sentence summary",
    "dialogs": [
      {{"speaker": "Host A", "text": "spoken content", "citations": ["NoteTitle1", "NoteTitle2"]}},
      ...
    ]
  }}

{format_instructions}

## Language
Generate the script in {language} unless all source content is in a single other language — then use that language.
Respond in the language the notes are written in.
"#,
        date = date,
        format_desc = format_desc,
        format_instructions = format_instructions,
        language = request.language.as_deref().unwrap_or("the user's language"),
    )
}

/// Build the user prompt with note content.
fn build_script_user_prompt(docs: &[NoteDocument], all_notes: &[NoteMeta]) -> String {
    let mut note_sections = Vec::new();

    for (i, doc) in docs.iter().enumerate() {
        let meta = &doc.meta;
        let title = if meta.title.is_empty() {
            &meta.path
        } else {
            &meta.title
        };
        note_sections.push(format!(
            "--- Note {}: {} ---\n{}\n--- End Note {} ---",
            i + 1,
            title,
            doc.body,
            i + 1
        ));
    }

    let all_titles: Vec<String> = all_notes
        .iter()
        .map(|n| {
            if n.title.is_empty() {
                n.path.clone()
            } else {
                n.title.clone()
            }
        })
        .collect();

    format!(
        r#"Generate a podcast script from the following vault notes.

Source notes (available for citation): {notes_list}

{note_content}

Remember: Output ONLY valid JSON matching the schema. No markdown fences, no extra text.
"#,
        notes_list = all_titles.join(", "),
        note_content = note_sections.join("\n\n"),
    )
}

/// Parse the LLM response into a PodcastScript.
fn parse_script_response(text: &str) -> Result<PodcastScript> {
    // Try to extract JSON if wrapped in markdown fences
    let cleaned = if let Some(json) = extract_json_block(text) {
        json
    } else {
        text.trim().to_string()
    };

    let script: PodcastScript = serde_json::from_str(&cleaned).with_context(|| {
        format!(
            "invalid podcast script JSON. First 200 chars of response: {}",
            crate::sanitize_error(&text.chars().take(200).collect::<String>())
        )
    })?;

    if script.dialogs.is_empty() {
        return Err(anyhow::anyhow!("podcast script has zero dialog turns"));
    }

    Ok(script)
}

/// Extract JSON from text, handling markdown code fences.
fn extract_json_block(text: &str) -> Option<String> {
    let text = text.trim();
    // Try ```json ... ``` fence
    if let Some(start) = text.find("```json") {
        let after_fence = &text[start + 7..];
        if let Some(end) = after_fence.find("```") {
            let json = after_fence[..end].trim();
            if !json.is_empty() {
                return Some(json.to_string());
            }
        }
    }
    // Try ``` ... ``` (language-agnostic)
    if let Some(start) = text.find("```") {
        let after_fence = &text[start + 3..];
        if let Some(end) = after_fence.find("```") {
            let json = after_fence[..end].trim();
            if json.starts_with('{') {
                return Some(json.to_string());
            }
        }
    }
    // If the whole text looks like JSON
    let trimmed = text.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed.to_string());
    }
    None
}

/// Render the script to plain text for the index note.
fn render_script_to_text(script: &PodcastScript) -> String {
    let mut lines = Vec::new();
    lines.push(format!("# {}\n", script.title));
    lines.push(format!("> {}\n", script.description));
    lines.push(String::new());

    for turn in &script.dialogs {
        let speaker = &turn.speaker;
        lines.push(format!("**{}:** {}", speaker, turn.text));
        if !turn.citations.is_empty() {
            let cites: Vec<&str> = turn.citations.iter().map(|s| s.as_str()).collect();
            lines.push(format!("  📎 *Sources: {}*", cites.join(", ")));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

// ── Step B: Audio synthesis ──────────────────────────────────────────────

struct SynthesizedAudio {
    audio_data: Vec<u8>,
    duration_secs: f64,
}

/// Synthesize the podcast script into audio using the TTS provider.
async fn synthesize_podcast_audio(
    settings: &AppSettings,
    script: &PodcastScript,
    request: &AudioOverviewRequest,
) -> Result<SynthesizedAudio> {
    let tts_config = request.tts_config.as_ref().cloned().unwrap_or_default();

    // Build TTS provider from the active provider config
    let provider_config = settings.effective_provider().clone();
    let tts_provider =
        tts::create_tts_provider(provider_config).context("failed to create TTS provider")?;

    // Determine voice mapping based on format
    let (voice_a, voice_b) = if request.format.is_dual_host() {
        (tts_config.voice_a, Some(tts_config.voice_b))
    } else {
        (tts_config.voice_a, None)
    };

    // Synthesize each dialog turn, concatenate into one audio file
    let mut all_audio = Vec::new();
    let mut total_duration = 0.0;

    for turn in &script.dialogs {
        let text = turn.text.trim();
        if text.is_empty() {
            continue;
        }

        // Determine which voice to use
        let voice = if turn.speaker.contains("B") || turn.speaker.contains("Co-host") {
            voice_b.unwrap_or(voice_a)
        } else {
            voice_a
        };

        let result = tts_provider
            .synthesize(text, voice, &tts_config)
            .await
            .context("TTS synthesis failed")?;

        all_audio.extend(result.audio_data);
        total_duration += result.duration_secs;
    }

    Ok(SynthesizedAudio {
        audio_data: all_audio,
        duration_secs: total_duration,
    })
}

// ── Step C: Save files and create index note ─────────────────────────────

/// Save the audio file to `attachments/audio/` in the vault.
async fn save_audio_file(
    vault_dir: &Path,
    audio_data: &[u8],
    format: &AudioOverviewFormat,
) -> Result<String> {
    let audio_dir = vault_dir.join("attachments").join("audio");
    tokio::fs::create_dir_all(&audio_dir)
        .await
        .context("failed to create attachments/audio directory")?;

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let format_suffix = format.as_str();
    let filename = format!("audio_overview_{}_{}.mp3", format_suffix, timestamp);
    let audio_path = audio_dir.join(&filename);

    tokio::fs::write(&audio_path, audio_data)
        .await
        .with_context(|| format!("failed to write audio file: {}", audio_path.display()))?;

    // Return relative path
    let rel = audio_path
        .strip_prefix(vault_dir)
        .unwrap_or(&audio_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Write the index note with embedded audio player + full script + source wikilinks.
async fn write_index_note(
    context: &StorageContext,
    _vault_dir: &Path,
    script: &PodcastScript,
    audio_rel_path: &str,
    source_notes: &[NoteMeta],
) -> Result<String> {
    // Build wikilinks for source notes
    let source_wikilinks: Vec<String> = source_notes
        .iter()
        .map(|note| {
            let title = if note.title.is_empty() {
                &note.path
            } else {
                &note.title
            };
            format!("- [[{}]]", title)
        })
        .collect();

    let source_section = if source_wikilinks.is_empty() {
        String::new()
    } else {
        format!("\n## Source Notes\n{}\n", source_wikilinks.join("\n"))
    };

    let script_text = render_script_to_text(script);

    let note_body = format!(
        r#"# 🎙️ Audio Overview: {title}

> {description}

## Audio Player

🎧 [Listen to Audio]({audio_rel})

*Generated on {date} · {duration}*

---

## Full Script

{script}

{source_section}

---

*Generated by VaultPilot Audio Overview. Based on {note_count} source note(s).*
"#,
        title = script.title,
        description = script.description,
        audio_rel = audio_rel_path,
        date = Utc::now().format("%Y-%m-%d %H:%M:%S"),
        duration = format_duration(estimate_total_duration(script)),
        script = script_text,
        source_section = source_section,
        note_count = source_notes.len(),
    );

    let note_title = format!("Audio Overview: {}", script.title);
    let now = Utc::now().to_rfc3339();

    let note = NoteDocument {
        meta: NoteMeta {
            id: Uuid::new_v4().to_string(),
            title: note_title,
            tags: vec!["audio-overview".to_string()],
            keywords: vec!["audio-overview".to_string(), "podcast".to_string()],
            platform: String::new(),
            board: String::new(),
            kernel: String::new(),
            status: "published".to_string(),
            created_at: now.clone(),
            updated_at: now,
            source: "audio-overview".to_string(),
            path: String::new(),
            summary: script.description.clone(),
            collections: vec![],
        },
        body: note_body,
        search_snippet: None,
        search_score: None,
    };

    let saved = storage::save_note_async(context, note)
        .await
        .context("failed to save Audio Overview index note")?;

    Ok(saved.meta.path)
}

/// Estimate total audio duration from script text length (rough).
fn estimate_total_duration(script: &PodcastScript) -> f64 {
    let total_chars: usize = script.dialogs.iter().map(|d| d.text.len()).sum();
    total_chars as f64 / 180.0 // ~180 chars/sec at normal speaking pace
}

/// Format seconds into a human-readable duration string.
fn format_duration(secs: f64) -> String {
    let total_secs = secs as u64;
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    if minutes > 0 {
        format!("{} min {} sec", minutes, seconds)
    } else {
        format!("{} sec", seconds)
    }
}

// ── Helper: Load notes ───────────────────────────────────────────────────

/// Load note metadata from the given source.
async fn load_source_notes(
    context: &StorageContext,
    source: &AudioOverviewSource,
    vault_dir: &Path,
) -> Result<Vec<NoteMeta>> {
    match source {
        AudioOverviewSource::Collection { id } => {
            let id = id.clone();
            let ctx = context.clone();
            let notes: Vec<NoteMeta> = tokio::task::spawn_blocking(move || {
                storage::list_notes_in_collection_with_context(&ctx, &id, 100, 0)
            })
            .await
            .context("spawn_blocking for collection notes")?
            .context("failed to list notes in collection")?;
            Ok(notes)
        }
        AudioOverviewSource::Folder { path } => {
            let path = path.clone();
            let vault_dir = vault_dir.to_path_buf();
            let ctx = context.clone();
            let notes: Vec<NoteMeta> = tokio::task::spawn_blocking(move || {
                let mut result = Vec::new();
                let folder_path = vault_dir.join(&path);
                if !folder_path.exists() || !folder_path.is_dir() {
                    return Err(anyhow::anyhow!(
                        "folder '{}' does not exist in vault",
                        folder_path.display()
                    ));
                }
                if let Ok(entries) = std::fs::read_dir(&folder_path) {
                    for entry in entries.flatten() {
                        let entry_path = entry.path();
                        if entry_path.extension().and_then(|e| e.to_str()) == Some("md") {
                            match storage::load_note_with_context(
                                &ctx,
                                &entry_path.to_string_lossy(),
                            ) {
                                Ok(doc) => result.push(doc.meta),
                                Err(e) => tracing::warn!(
                                    "failed to load note '{}': {}",
                                    entry_path.display(),
                                    e
                                ),
                            }
                        }
                    }
                }
                if result.is_empty() {
                    Err(anyhow::anyhow!(
                        "no .md notes found in folder '{}'",
                        folder_path.display()
                    ))
                } else {
                    Ok(result)
                }
            })
            .await
            .context("spawn_blocking for folder notes")??;
            Ok(notes)
        }
        AudioOverviewSource::NoteIds { ids } => {
            let ids = ids.clone();
            let ctx = context.clone();
            let notes: Vec<NoteMeta> =
                tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<NoteMeta>> {
                    let mut result = Vec::new();
                    for id in &ids {
                        match storage::load_note_with_context(&ctx, id) {
                            Ok(doc) => result.push(doc.meta),
                            Err(e) => tracing::warn!("failed to load note '{}': {}", id, e),
                        }
                    }
                    Ok(result)
                })
                .await
                .context("spawn_blocking for note IDs")??;
            Ok(notes)
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_roundtrip() {
        for f in &[AudioOverviewFormat::DeepDive, AudioOverviewFormat::Brief] {
            let s = f.as_str();
            let parsed: Result<AudioOverviewFormat, _> = s.parse();
            assert_eq!(parsed.ok(), Some(*f), "roundtrip failed for {}", s);
        }
    }

    #[test]
    fn format_default_is_deep_dive() {
        assert_eq!(
            AudioOverviewFormat::default(),
            AudioOverviewFormat::DeepDive
        );
    }

    #[test]
    fn deep_dive_is_dual_host() {
        assert!(AudioOverviewFormat::DeepDive.is_dual_host());
        assert!(!AudioOverviewFormat::Brief.is_dual_host());
    }

    #[test]
    fn format_labels() {
        assert_eq!(AudioOverviewFormat::DeepDive.label(), "Deep Dive");
        assert_eq!(AudioOverviewFormat::Brief.label(), "The Brief");
    }

    #[test]
    fn empty_script_returns_error() {
        let result = parse_script_response("invalid text");
        assert!(result.is_err());
    }

    #[test]
    fn parse_valid_script_json() {
        let json = r#"{"title":"Test","description":"A test","dialogs":[{"speaker":"Host A","text":"Hello","citations":[]}]}"#;
        let script = parse_script_response(json).unwrap();
        assert_eq!(script.title, "Test");
        assert_eq!(script.dialogs.len(), 1);
        assert_eq!(script.dialogs[0].speaker, "Host A");
    }

    #[test]
    fn parse_script_with_code_fence() {
        let text = "Here is the result:\n```json\n{\"title\":\"From Fence\",\"description\":\"Test\",\"dialogs\":[{\"speaker\":\"Host B\",\"text\":\"World\",\"citations\":[]}]}\n```\nEnjoy!";
        let script = parse_script_response(text).unwrap();
        assert_eq!(script.title, "From Fence");
        assert_eq!(script.dialogs[0].text, "World");
    }

    #[test]
    fn extract_json_from_fenced_block() {
        let text = "Some text\n```json\n{\"key\":\"value\"}\n```\nmore text";
        let result = extract_json_block(text);
        assert!(result.is_some());
        assert!(result.unwrap().contains("\"key\""));
    }

    #[test]
    fn extract_json_from_bare_text() {
        let text = "{\"key\":\"value\"}";
        let result = extract_json_block(text);
        assert!(result.is_some());
    }

    #[test]
    fn extract_json_returns_none_for_plain_text() {
        let result = extract_json_block("just some text without json");
        assert!(result.is_none());
    }

    #[test]
    fn render_script_to_text_includes_title() {
        let script = PodcastScript {
            title: "My Episode".to_string(),
            description: "Desc".to_string(),
            dialogs: vec![DialogTurn {
                speaker: "Host A".to_string(),
                text: "Hello listeners".to_string(),
                citations: vec!["Note1".to_string()],
            }],
        };
        let text = render_script_to_text(&script);
        assert!(text.contains("My Episode"));
        assert!(text.contains("Hello listeners"));
        assert!(text.contains("Note1"));
    }

    #[test]
    fn format_duration_basic() {
        assert_eq!(format_duration(60.0), "1 min 0 sec");
        assert_eq!(format_duration(90.0), "1 min 30 sec");
        assert_eq!(format_duration(30.0), "30 sec");
        assert_eq!(format_duration(0.0), "0 sec");
    }

    #[test]
    fn source_from_str_handles_all_formats() {
        assert_eq!(
            "deep-dive".parse::<AudioOverviewFormat>().ok(),
            Some(AudioOverviewFormat::DeepDive)
        );
        assert_eq!(
            "deep_dive".parse::<AudioOverviewFormat>().ok(),
            Some(AudioOverviewFormat::DeepDive)
        );
        assert_eq!(
            "brief".parse::<AudioOverviewFormat>().ok(),
            Some(AudioOverviewFormat::Brief)
        );
        assert_eq!("unknown".parse::<AudioOverviewFormat>().ok(), None);
    }
}
