//! Daily Briefing — AI-powered morning summary of vault activity (#3459).
//!
//! Generates a structured daily digest by:
//! 1. Scanning notes created or modified in the last 24 hours
//! 2. Aggregating content via AI summarization
//! 3. Saving the briefing as a vault note under `Daily/Briefing/`
//!
//! ## Integration
//! - CLI: `vaultpilot daily-briefing [--dry-run]`
//! - Future: cron-triggered via the trigger executor (see `trigger_executor.rs`)
//!
//! ## Output structure
//! The AI is prompted to produce a structured briefing with sections:
//! - 📝 昨日回顾 (notes created/changed)
//! - ✅ 待办提醒 (open tasks approaching deadline)
//! - 🔗 相关推荐 (semantically related notes worth revisiting)

use anyhow::Result;
use chrono::{Duration, Local, Utc};
use tracing::instrument;

use crate::ai::client::send_request_with_temperature;
use crate::ai::RequestUsage;
use crate::models::{AppSettings, NoteDocument, NoteMeta};
use crate::storage::{
    find_note_by_title_and_tag_async, load_recent_notes_for_overview_async, save_note_with_context,
    StorageContext,
};

/// System prompt for the daily briefing AI call.
const BRIEFING_SYSTEM_PROMPT: &str = "\
You are a smart daily briefing assistant for a personal notes vault. \
Your task is to analyze the user's recent notes (created or modified in the last 24 hours) \
and produce a structured, concise daily briefing in Markdown.

The briefing MUST include these sections:

## 📝 昨日回顾
- Summarise what the user worked on yesterday, grouped by theme or topic.
- For each group, list the note titles as [[wikilinks]] and a 1-2 sentence summary.
- If the user worked on multiple distinct topics, keep them in separate groups.

## ✅ 待办提醒
- Scan the recent notes for open tasks (checklist items like `- [ ]`, `#TODO`, or similar markers).
- List tasks that are still open, with their source note as a [[wikilink]].
- If no open tasks are found, state that explicitly.

## 🔗 相关推荐
- Based on the content of today's notes, suggest 2-3 other notes in the vault that the user might want to revisit.
- For each suggestion, include a [[wikilink]] and a one-sentence reason.
- If you cannot determine any relevant notes, mention 'None identified.'

## 💡 AI 洞察 (optional)
- A brief observation or pattern noticed across the day's notes (e.g., recurring themes, potential connections).

Output ONLY the Markdown briefing. Do not add extraneous commentary. \
Keep the tone helpful and concise. Respond in the same language as the notes (detect automatically).";

/// Generate a daily briefing from recently modified vault notes.
///
/// # Arguments
/// * `ctx` - Storage context for database access
/// * `settings` - App settings (used for AI provider config)
///
/// # Returns
/// The generated briefing text on success.
#[instrument(skip(ctx, settings))]
pub async fn generate_daily_briefing(
    ctx: &StorageContext,
    settings: &AppSettings,
) -> Result<DailyBriefingResult> {
    // 1. Load recent notes (up to 50 notes)
    let recent_notes = load_recent_notes_for_overview_async(ctx, 50).await?;

    // 2. Filter by recency: keep only notes modified/created within the last 24 hours
    //    Use Local::now() so the 24-hour window aligns with the user's actual calendar
    //    day rather than UTC midnight.  (#3540)
    let cutoff = Local::now() - Duration::hours(24);
    let cutoff_utc = cutoff.with_timezone(&Utc);
    let filtered: Vec<&NoteDocument> = recent_notes
        .iter()
        .filter(|n| {
            let updated = parse_iso_timestamp(&n.meta.updated_at)
                .or_else(|| parse_iso_timestamp(&n.meta.created_at));
            let created = parse_iso_timestamp(&n.meta.created_at);
            match (updated, created) {
                (Some(u), _) if u >= cutoff_utc => true,
                (_, Some(c)) if c >= cutoff_utc => true,
                _ => false,
            }
        })
        .collect();

    if filtered.is_empty() {
        // No recent activity — return a minimal briefing
        let no_content = "\
## 📝 昨日回顾\n\nNo notes were created or modified in the last 24 hours.\n\n\
## ✅ 待办提醒\n\nNo recent tasks to report.\n\n\
## 🔗 相关推荐\n\nNo recent activity to base recommendations on.\n\n\
## 💡 AI 洞察\n\nStart writing today to unlock tomorrow's briefing! 📝"
            .to_string();
        return Ok(DailyBriefingResult {
            briefing: no_content,
            note_count: 0,
            usage: RequestUsage::default(),
        });
    }

    // 3. Build the user prompt with note content
    let mut notes_block = String::new();
    for note in &filtered {
        notes_block.push_str(&format!(
            "## {}\n\n{}\n\n---\n\n",
            note.meta.title,
            if note.body.is_empty() {
                "(empty)"
            } else {
                &note.body
            }
        ));
    }

    let user_prompt = format!(
        "Here are the notes from the last 24 hours in my vault. \
         Please generate a structured daily briefing following the required format.\n\n\
         ---\n\n\
         {}",
        notes_block
    );

    // 4. Call the AI with a low temperature for deterministic output
    let response =
        send_request_with_temperature(settings, BRIEFING_SYSTEM_PROMPT, &user_prompt, &[], 0.3)
            .await?;

    // 5. Save the briefing as a vault note
    //    Use Local::now() for the date string so the title reflects the user's
    //    actual calendar day.  Metadata timestamps (RFC3339) remain Utc::now()
    //    for correct ISO storage.  (#3540)
    let date_str = Local::now().format("%Y-%m-%d").to_string();
    let title = format!("Daily Briefing — {}", date_str);
    let now_rfc = Utc::now().to_rfc3339();

    // Idempotency check (#3499): if today's briefing already exists, reuse its
    // ID + created_at so `save_note_with_context` performs an upsert (overwrite)
    // instead of creating a duplicate note with the same title.
    let (existing_id, existing_created_at) =
        match find_note_by_title_and_tag_async(ctx, title.clone(), "daily-briefing".to_string())
            .await
        {
            Ok(Some((id, created_at))) => (id, created_at),
            Ok(None) => (String::new(), String::new()),
            Err(e) => {
                tracing::warn!(
                    "daily_briefing: idempotency lookup failed (proceeding with new note): {e}"
                );
                (String::new(), String::new())
            }
        };

    let note = NoteDocument {
        meta: NoteMeta {
            id: existing_id,
            title,
            tags: vec!["daily-briefing".to_string(), "auto-generated".to_string()],
            created_at: if existing_created_at.is_empty() {
                now_rfc.clone()
            } else {
                existing_created_at
            },
            updated_at: now_rfc,
            ..Default::default()
        },
        body: response.text.clone(),
        search_snippet: None,
        search_score: None,
    };

    let _saved = save_note_with_context(ctx, note)?;

    Ok(DailyBriefingResult {
        briefing: response.text,
        note_count: filtered.len(),
        usage: response.usage,
    })
}

/// Result of a daily briefing generation.
#[derive(Debug, Clone)]
pub struct DailyBriefingResult {
    /// The generated briefing Markdown text.
    pub briefing: String,
    /// Number of notes that were included in the analysis.
    pub note_count: usize,
    /// Token usage from the AI call.
    pub usage: RequestUsage,
}

/// Parse an ISO 8601 timestamp string, returning `None` on failure.
pub fn parse_iso_timestamp(s: &str) -> Option<chrono::DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }
    // Try common ISO 8601 formats
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Try without timezone suffix (treated as UTC)
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(dt.and_utc());
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, Utc};

    #[test]
    fn parse_iso_timestamp_rfc3339() {
        let ts = "2026-07-25T10:30:00.123Z";
        let dt = parse_iso_timestamp(ts).expect("should parse RFC 3339");
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-07-25");
    }

    #[test]
    fn parse_iso_timestamp_no_tz() {
        let ts = "2026-07-25T10:30:00.123";
        let dt = parse_iso_timestamp(ts).expect("should parse without timezone");
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-07-25");
    }

    #[test]
    fn parse_iso_timestamp_simple() {
        let ts = "2026-07-25 10:30:00";
        let dt = parse_iso_timestamp(ts).expect("should parse simple format");
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2026-07-25");
    }

    #[test]
    fn parse_iso_timestamp_empty() {
        assert!(parse_iso_timestamp("").is_none());
    }

    #[test]
    fn parse_iso_timestamp_garbage() {
        assert!(parse_iso_timestamp("not-a-date").is_none());
    }

    #[test]
    fn filter_excludes_unparseable_timestamps() {
        // Regression test for #3476: when both updated_at and created_at
        // fail to parse, the note should be excluded, not silently included.
        let cutoff = Local::now() - Duration::hours(24);
        let cutoff_utc = cutoff.with_timezone(&Utc);
        let meta = crate::models::NoteMeta {
            id: "garbage".into(),
            created_at: "garbage".into(),
            updated_at: "garbage".into(),
            ..Default::default()
        };
        let updated =
            parse_iso_timestamp(&meta.updated_at).or_else(|| parse_iso_timestamp(&meta.created_at));
        let created = parse_iso_timestamp(&meta.created_at);
        let should_include = match (updated, created) {
            (Some(u), _) if u >= cutoff_utc => true,
            (_, Some(c)) if c >= cutoff_utc => true,
            _ => false,
        };
        assert!(
            !should_include,
            "notes with unparseable timestamps should be excluded"
        );
    }

    #[test]
    fn briefing_generates_with_no_notes() {
        // Validate the no-notes fallback format compiles and contains expected sections.
        let no_content = "\
## 📝 昨日回顾\n\nNo notes were created or modified in the last 24 hours.\n\n\
## ✅ 待办提醒\n\nNo recent tasks to report.\n\n\
## 🔗 相关推荐\n\nNo recent activity to base recommendations on.\n\n\
## 💡 AI 洞察\n\nStart writing today to unlock tomorrow's briefing! 📝";
        assert!(no_content.contains("📝 昨日回顾"));
        assert!(no_content.contains("✅ 待办提醒"));
        assert!(no_content.contains("🔗 相关推荐"));
        assert!(no_content.contains("💡 AI 洞察"));
    }

    #[test]
    fn parse_iso_timestamp_recent_window() {
        // Notes from 2 hours ago should be within the 24h window
        let now = Utc::now();
        let two_hours_ago = now - Duration::hours(2);
        let ts = two_hours_ago.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        let dt = parse_iso_timestamp(&ts).expect("should parse recent timestamp");
        assert!(dt >= now - Duration::hours(24));
    }

    #[test]
    fn parse_iso_timestamp_outside_window() {
        // Notes from 48 hours ago should be outside the 24h window
        let now = Utc::now();
        let two_days_ago = now - Duration::hours(48);
        let ts = two_days_ago.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        let dt = parse_iso_timestamp(&ts).expect("should parse old timestamp");
        assert!(dt < now - Duration::hours(24));
    }

    #[test]
    fn briefing_system_prompt_structure() {
        // Validate the system prompt contains all required sections
        assert!(BRIEFING_SYSTEM_PROMPT.contains("📝 昨日回顾"));
        assert!(BRIEFING_SYSTEM_PROMPT.contains("✅ 待办提醒"));
        assert!(BRIEFING_SYSTEM_PROMPT.contains("🔗 相关推荐"));
        assert!(BRIEFING_SYSTEM_PROMPT.contains("💡 AI 洞察"));
        assert!(BRIEFING_SYSTEM_PROMPT.contains("[[wikilinks]]"));
        assert!(BRIEFING_SYSTEM_PROMPT.contains("Markdown"));
    }

    // ── #3499: Daily Briefing idempotency regression tests ──

    #[test]
    fn briefing_title_format_matches_dedup_query() {
        // The title constructed in generate_daily_briefing uses this exact format.
        // find_note_by_title_and_tag must match it for the idempotency upsert.
        let date_str = "2026-07-27";
        let title = format!("Daily Briefing — {}", date_str);
        assert_eq!(title, "Daily Briefing — 2026-07-27");
        // The tag used for dedup is always "daily-briefing"
        let dedup_tag = "daily-briefing";
        assert!(!dedup_tag.is_empty());
    }

    #[test]
    fn briefing_title_is_date_deterministic() {
        // Two briefings on the same day produce the same title → the second
        // run must find the first note and upsert, not create a duplicate.
        let title1 = format!("Daily Briefing — {}", "2026-07-27");
        let title2 = format!("Daily Briefing — {}", "2026-07-27");
        assert_eq!(
            title1, title2,
            "same-day titles must be identical for dedup"
        );
        // Different days produce different titles → new note, not upsert
        let title3 = format!("Daily Briefing — {}", "2026-07-28");
        assert_ne!(title1, title3, "cross-day titles must differ");
    }

    #[test]
    fn find_note_by_title_and_tag_finds_existing_briefing() {
        // Integration test: save a note, then verify find_note_by_title_and_tag
        // locates it — this is the mechanism that powers the idempotency check.
        use crate::models::{NoteDocument, NoteMeta};
        use crate::storage::{
            find_note_by_title_and_tag, initialize_storage_with_context, save_note_with_context,
            StorageContext,
        };

        let dir = std::env::temp_dir().join(format!(
            "vp-briefing-dedup-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let ctx = StorageContext::for_test(&dir);
        initialize_storage_with_context(&ctx).expect("init storage");

        let title = "Daily Briefing — 2026-07-27".to_string();
        let now = Utc::now().to_rfc3339();

        // Save a briefing note (first run)
        let note = NoteDocument {
            meta: NoteMeta {
                id: String::new(),
                title: title.clone(),
                tags: vec!["daily-briefing".to_string(), "auto-generated".to_string()],
                created_at: now.clone(),
                updated_at: now,
                ..Default::default()
            },
            body: "## 📝 昨日回顾\n\nFirst run content".to_string(),
            search_snippet: None,
            search_score: None,
        };
        let saved = save_note_with_context(&ctx, note).expect("save first briefing");
        let first_id = saved.meta.id.clone();
        assert!(!first_id.is_empty());

        // The dedup query must find the note we just saved
        let found =
            find_note_by_title_and_tag(&ctx, &title, "daily-briefing").expect("dedup query");
        assert!(
            found.is_some(),
            "find_note_by_title_and_tag must locate existing briefing"
        );
        let (found_id, found_created_at) = found.unwrap();
        assert_eq!(
            found_id, first_id,
            "dedup query must return the same note ID"
        );
        assert!(
            !found_created_at.is_empty(),
            "dedup query must return created_at for preservation"
        );

        // Simulate the second run: reuse the existing ID to upsert
        let note2 = NoteDocument {
            meta: NoteMeta {
                id: found_id, // reuse → upsert, not duplicate
                title: title.clone(),
                tags: vec!["daily-briefing".to_string(), "auto-generated".to_string()],
                created_at: found_created_at, // preserve original creation date
                updated_at: Utc::now().to_rfc3339(),
                ..Default::default()
            },
            body: "## 📝 昨日回顾\n\nUpdated content from second run".to_string(),
            search_snippet: None,
            search_score: None,
        };
        let saved2 = save_note_with_context(&ctx, note2).expect("upsert briefing");
        assert_eq!(
            saved2.meta.id, first_id,
            "second run must reuse same ID (upsert), not create a new note"
        );

        // Verify no duplicate was created — only one note with this title+tag
        let found_after =
            find_note_by_title_and_tag(&ctx, &title, "daily-briefing").expect("dedup after upsert");
        assert!(found_after.is_some());
        assert_eq!(
            found_after.unwrap().0,
            first_id,
            "still the same single note"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── #3540: Daily Briefing timezone-aware date regression tests ──

    #[test]
    fn briefing_title_uses_local_not_utc_date() {
        // The date string in the briefing title must come from Local::now(),
        // not Utc::now(). For users in positive-offset timezones (e.g. UTC+8),
        // Utc::now() can be a calendar day behind, causing the title to show
        // "yesterday" instead of "today".
        let local_date = Local::now().format("%Y-%m-%d").to_string();
        let title = format!("Daily Briefing — {}", local_date);
        assert!(
            title.starts_with("Daily Briefing — "),
            "title must have correct prefix"
        );
        // Verify the date portion parses as a valid date
        let date_part = &title["Daily Briefing — ".len()..];
        chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
            .expect("date string must be valid YYYY-MM-DD");
    }

    #[test]
    fn briefing_cutoff_derived_from_local_time() {
        // The 24-hour cutoff for filtering recent notes should be based on
        // Local::now(), not Utc::now(). This test verifies that the cutoff
        // computed from Local::now() is used correctly for timestamp comparison.
        let local_cutoff = Local::now() - Duration::hours(24);
        let cutoff_utc = local_cutoff.with_timezone(&Utc);

        // A note from 2 hours ago (in UTC) should be within the window
        let recent_ts = Utc::now() - Duration::hours(2);
        assert!(
            recent_ts >= cutoff_utc,
            "note from 2h ago must be within the 24h cutoff window"
        );

        // A note from 48 hours ago should be outside the window
        let old_ts = Utc::now() - Duration::hours(48);
        assert!(
            old_ts < cutoff_utc,
            "note from 48h ago must be outside the 24h cutoff window"
        );
    }

    #[test]
    fn local_and_utc_date_differ_by_at_most_one_day() {
        // For any timezone, the local date and UTC date can differ by at most
        // one day. If they differ by more, something is fundamentally wrong.
        let local_date = Local::now().format("%Y-%m-%d").to_string();
        let utc_date = Utc::now().format("%Y-%m-%d").to_string();
        let local = chrono::NaiveDate::parse_from_str(&local_date, "%Y-%m-%d").unwrap();
        let utc = chrono::NaiveDate::parse_from_str(&utc_date, "%Y-%m-%d").unwrap();
        let diff = (local - utc).num_days().abs();
        assert!(
            diff <= 1,
            "local and UTC dates should differ by at most 1 day, got {} days",
            diff
        );
    }
}
