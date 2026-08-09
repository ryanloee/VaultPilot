//! Calendar integration foundation — ICS provider, meeting metadata, agenda API.
//!
//! # Phase 1 scope
//! - `IcsCalendarProvider`: real RFC 5545 iCalendar (.ics) parser (no new deps)
//! - `CalendarProvider` trait for pluggable providers (future OAuth/Graph)
//! - `LocalCalendarProvider` for in-memory/testing use
//! - SQLite cache for synced events (`calendar_events` table)
//! - `today_agenda` / `today_agenda_cached` for dashboard & briefings
//! - `attach_meeting_metadata` to merge calendar metadata into markdown notes
//!
//! # Future work (out of scope for this foundation)
//! - Google Calendar OAuth connector
//! - Microsoft Graph API connector
//! - Android system calendar ContentProvider fallback

use crate::storage::StorageContext;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Data types ───────────────────────────────────────────────────

/// A single calendar event parsed from a provider or loaded from cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    /// Unique identifier (provider-qualified when loaded from cache).
    pub id: String,
    /// The event ID as given by the source calendar (e.g. ICS UID).
    pub provider_event_id: String,
    /// Event title (ICS SUMMARY).
    pub title: String,
    /// Start time in UTC.
    pub start: DateTime<Utc>,
    /// End time in UTC (falls back to `start` when absent).
    pub end: DateTime<Utc>,
    /// Optional location (ICS LOCATION).
    pub location: Option<String>,
    /// Optional description (ICS DESCRIPTION).
    pub description: Option<String>,
    /// Attendee display names (ICS ATTENDEE CN or mailto value).
    pub attendees: Vec<String>,
    /// Calendar / source name this event originated from.
    pub source: String,
    /// Whether this is an all-day event (ICS VALUE=DATE).
    pub all_day: bool,
}

// ─── Provider trait ───────────────────────────────────────────────

/// Pluggable calendar data source.  Implementations include
/// [`IcsCalendarProvider`] (local .ics files) and
/// [`LocalCalendarProvider`] (in-memory, for tests / programmatic use).
/// Future implementations: Google OAuth, Microsoft Graph.
#[async_trait]
pub trait CalendarProvider: Send + Sync {
    /// Return events whose `[start, end]` overlaps `[range_start, range_end]`.
    async fn list_events(
        &self,
        range_start: DateTime<Utc>,
        range_end: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>>;
}

// ─── ICS Provider ─────────────────────────────────────────────────

/// Reads and parses iCalendar (.ics) content using only std + chrono.
pub struct IcsCalendarProvider {
    ics_content: String,
    calendar_name: String,
}

impl IcsCalendarProvider {
    /// Build from raw ICS text.
    pub fn from_content(content: impl Into<String>, calendar_name: impl Into<String>) -> Self {
        Self {
            ics_content: content.into(),
            calendar_name: calendar_name.into(),
        }
    }

    /// Build by reading a `.ics` file from disk.
    pub fn from_file(path: &std::path::Path, calendar_name: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read ICS file: {}", path.display()))?;
        Ok(Self::from_content(content, calendar_name))
    }

    /// Parse all VEVENT blocks.  Errors in individual date/time fields fall
    /// back to the Unix epoch rather than aborting the entire parse.
    pub fn parse_events(&self) -> Vec<CalendarEvent> {
        parse_ics(&self.ics_content, &self.calendar_name)
    }
}

#[async_trait]
impl CalendarProvider for IcsCalendarProvider {
    async fn list_events(
        &self,
        range_start: DateTime<Utc>,
        range_end: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>> {
        let mut events = self.parse_events();
        events.retain(|e| e.start <= range_end && e.end >= range_start);
        Ok(events)
    }
}

// ─── Local Provider (in-memory) ───────────────────────────────────

/// Holds events in memory; useful for tests and programmatic providers.
pub struct LocalCalendarProvider {
    events: Vec<CalendarEvent>,
}

impl LocalCalendarProvider {
    pub fn new(events: Vec<CalendarEvent>) -> Self {
        Self { events }
    }
}

#[async_trait]
impl CalendarProvider for LocalCalendarProvider {
    async fn list_events(
        &self,
        range_start: DateTime<Utc>,
        range_end: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>> {
        let mut events: Vec<CalendarEvent> = self
            .events
            .iter()
            .filter(|e| e.start <= range_end && e.end >= range_start)
            .cloned()
            .collect();
        events.sort_by_key(|e| e.start);
        Ok(events)
    }
}

// ─── ICS Parser (RFC 5545) ────────────────────────────────────────

/// Unfold RFC 5545 folded lines: a line beginning with SP/HTAB is a
/// continuation of the previous logical line (the leading whitespace is
/// removed and the remainder appended).
fn unfold_lines(content: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for line in content.lines() {
        if (line.starts_with(' ') || line.starts_with('\t')) && !result.is_empty() {
            // Continuation — strip the single leading whitespace char and append.
            if let Some(last) = result.last_mut() {
                last.push_str(&line[1..]);
            }
        } else {
            result.push(line.trim_end_matches('\r').to_string());
        }
    }
    result
}

/// Extract a named parameter value (e.g. `CN` from `ATTENDEE;CN=...`).
fn extract_param(params_str: &str, key: &str) -> Option<String> {
    for param in params_str.split(';') {
        if let Some((k, v)) = param.split_once('=') {
            if k.eq_ignore_ascii_case(key) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Parse an ICS date-time value into `DateTime<Utc>`.
///
/// Handles:
/// - `20260701T100000Z` (UTC basic format)
/// - `20260701T100000`  (floating time — treated as UTC)
/// - `2026-07-01T10:00:00+08:00` (RFC 3339)
/// - `20260701` (all-day date, when `is_date_only` is true)
fn parse_ics_datetime(value: &str, is_date_only: bool) -> DateTime<Utc> {
    let v = value.trim();
    if is_date_only {
        return NaiveDate::parse_from_str(v, "%Y%m%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|dt| dt.and_utc())
            .unwrap_or_else(epoch);
    }
    // RFC 3339 (handles timezone offsets like +08:00)
    if let Ok(dt) = DateTime::parse_from_rfc3339(v) {
        return dt.with_timezone(&Utc);
    }
    // Basic ICS UTC format: YYYYMMDDTHHMMSSZ
    if let Some(utc_part) = v.strip_suffix('Z') {
        if let Ok(dt) = NaiveDateTime::parse_from_str(utc_part, "%Y%m%dT%H%M%S") {
            return dt.and_utc();
        }
    }
    // Floating time: YYYYMMDDTHHMMSS (assume UTC)
    if let Ok(dt) = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S") {
        return dt.and_utc();
    }
    epoch()
}

/// Unescape ICS text per RFC 5545 §3.3.11: `\n`→newline, `\,`→comma, etc.
fn unescape_ics_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap()
}

/// Parse all VEVENT blocks from unfolded ICS content.
fn parse_ics(content: &str, source: &str) -> Vec<CalendarEvent> {
    let lines = unfold_lines(content);
    let mut events: Vec<CalendarEvent> = Vec::new();
    let mut in_event = false;

    let mut uid = String::new();
    let mut summary = String::new();
    let mut dtstart: Option<DateTime<Utc>> = None;
    let mut dtend: Option<DateTime<Utc>> = None;
    let mut location: Option<String> = None;
    let mut description: Option<String> = None;
    let mut attendees: Vec<String> = Vec::new();
    let mut all_day = false;

    for line in &lines {
        let upper = line.to_uppercase();
        if upper == "BEGIN:VEVENT" {
            in_event = true;
            uid.clear();
            summary.clear();
            dtstart = None;
            dtend = None;
            location = None;
            description = None;
            attendees.clear();
            all_day = false;
            continue;
        }
        if upper == "END:VEVENT" {
            if in_event {
                let start = dtstart.unwrap_or_else(epoch);
                let end = dtend.unwrap_or(start);
                events.push(CalendarEvent {
                    id: format!("ics-{}", uid),
                    provider_event_id: uid.clone(),
                    title: summary.clone(),
                    start,
                    end,
                    location: location.clone(),
                    description: description.clone(),
                    attendees: attendees.clone(),
                    source: source.to_string(),
                    all_day,
                });
            }
            in_event = false;
            continue;
        }
        if !in_event {
            continue;
        }

        // Split property into name[;params] and value at first ':'.
        let Some((name_params, value)) = line.split_once(':') else {
            continue;
        };
        let (prop_name_part, params_part) =
            name_params.split_once(';').unwrap_or((name_params, ""));
        let prop_name = prop_name_part.to_uppercase();
        let is_date = params_part.to_uppercase().contains("VALUE=DATE");

        match prop_name.as_str() {
            "UID" => uid = unescape_ics_text(value),
            "SUMMARY" => summary = unescape_ics_text(value),
            "DESCRIPTION" => description = Some(unescape_ics_text(value)),
            "LOCATION" => location = Some(unescape_ics_text(value)),
            "DTSTART" => {
                all_day = is_date;
                dtstart = Some(parse_ics_datetime(value, is_date));
            }
            "DTEND" => {
                dtend = Some(parse_ics_datetime(value, is_date));
            }
            "ATTENDEE" => {
                let name = extract_param(params_part, "CN")
                    .map(|cn| unescape_ics_text(&cn))
                    .unwrap_or_else(|| value.strip_prefix("mailto:").unwrap_or(value).to_string());
                if !name.is_empty() {
                    attendees.push(name);
                }
            }
            _ => {}
        }
    }
    events
}

// ─── Agenda helpers ───────────────────────────────────────────────

/// Fetch today's events from a live provider (e.g. ICS file on disk).
/// Events are sorted by start time.
pub async fn today_agenda(
    provider: &dyn CalendarProvider,
    now: DateTime<Utc>,
) -> Result<Vec<CalendarEvent>> {
    let day_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let day_end = now.date_naive().and_hms_opt(23, 59, 59).unwrap().and_utc();
    let mut events = provider.list_events(day_start, day_end).await?;
    events.sort_by_key(|e| e.start);
    Ok(events)
}

/// Return today's events from the SQLite cache (no provider needed).
/// Sorted by start time.
pub fn today_agenda_cached(
    context: &StorageContext,
    now: DateTime<Utc>,
) -> Result<Vec<CalendarEvent>> {
    let conn = db_conn(context)?;
    ensure_calendar_tables(&conn)?;
    let day_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let day_end = now.date_naive().and_hms_opt(23, 59, 59).unwrap().and_utc();
    let mut stmt = conn.prepare(
        "SELECT id, provider_event_id, title, start_utc, end_utc,
                location, description, attendees_json, all_day, source
         FROM calendar_events
         WHERE start_utc <= ?2 AND end_utc >= ?1
         ORDER BY start_utc ASC",
    )?;
    let events = stmt.query_map(
        params![day_start.to_rfc3339(), day_end.to_rfc3339()],
        row_to_event,
    )?;
    let mut result = Vec::new();
    for event in events {
        result.push(event?);
    }
    Ok(result)
}

// ─── Storage ──────────────────────────────────────────────────────

pub(crate) const CALENDAR_SCHEMA_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS calendar_events (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL DEFAULT '',
    provider_event_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    start_utc TEXT NOT NULL,
    end_utc TEXT NOT NULL,
    location TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    attendees_json TEXT NOT NULL DEFAULT '[]',
    all_day INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL DEFAULT '',
    synced_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_calendar_prov_event
    ON calendar_events(provider, provider_event_id);
CREATE INDEX IF NOT EXISTS idx_calendar_start ON calendar_events(start_utc);
"#;

/// Idempotently create calendar tables.  Called lazily by every public
/// DB-touching function and wired into `pool::ensure_schema` by the
/// orchestrator.
pub(crate) fn ensure_calendar_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(CALENDAR_SCHEMA_DDL)
        .context("failed to ensure calendar tables")?;
    Ok(())
}

fn db_conn(
    context: &StorageContext,
) -> Result<r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>> {
    context
        .get_connection()
        .context("failed to get database connection")
}

/// Upsert a slice of events into the cache.  The DB `id` is qualified with
/// `provider_name` so events from different calendars with the same UID do
/// not collide.  Returns the number of rows written.
pub fn sync_events(
    context: &StorageContext,
    provider_name: &str,
    events: &[CalendarEvent],
) -> Result<usize> {
    let conn = db_conn(context)?;
    ensure_calendar_tables(&conn)?;
    let now = Utc::now().to_rfc3339();
    let mut count = 0usize;
    for event in events {
        let id = format!("{}:{}", provider_name, event.provider_event_id);
        let attendees_json =
            serde_json::to_string(&event.attendees).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "INSERT OR REPLACE INTO calendar_events
                (id, provider, provider_event_id, title, start_utc, end_utc,
                 location, description, attendees_json, all_day, source, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                provider_name,
                event.provider_event_id,
                event.title,
                event.start.to_rfc3339(),
                event.end.to_rfc3339(),
                event.location.as_deref().unwrap_or(""),
                event.description.as_deref().unwrap_or(""),
                attendees_json,
                event.all_day as i64,
                event.source,
                now,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

// ── Local agent calendar CRUD (#3603) ──────────────────────────────
//
// The Agent Calendar Tools integration operates on the same `calendar_events`
// cache, using the reserved provider name "agent". Events created here are
// local (not synced to an external provider) until a Google/Outlook connector
// lands.

const AGENT_CALENDAR_PROVIDER: &str = "agent";

/// Query events cached in the local store overlapping `[start, end]`.
pub fn list_cached_events(
    context: &StorageContext,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<CalendarEvent>> {
    let conn = db_conn(context)?;
    ensure_calendar_tables(&conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, provider_event_id, title, start_utc, end_utc,
                location, description, attendees_json, all_day, source
         FROM calendar_events
         WHERE start_utc <= ?1 AND end_utc >= ?2
         ORDER BY start_utc",
    )?;
    let rows = stmt.query_map(params![end.to_rfc3339(), start.to_rfc3339()], |row| {
        let attendees_json: String = row.get(7)?;
        let attendees: Vec<String> = serde_json::from_str(&attendees_json).unwrap_or_default();
        Ok(CalendarEvent {
            id: row.get(0)?,
            provider_event_id: row.get(1)?,
            title: row.get(2)?,
            start: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(3)?)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            end: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            location: row.get(5)?,
            description: row.get(6)?,
            attendees,
            all_day: row.get::<_, i64>(8)? != 0,
            source: row.get(9)?,
        })
    })?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}

/// Create a new event in the local agent calendar (#3603).
pub fn create_agent_event(
    context: &StorageContext,
    title: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    location: Option<String>,
    description: Option<String>,
) -> Result<CalendarEvent> {
    if title.trim().is_empty() {
        anyhow::bail!("calendar event title must not be empty");
    }
    if end <= start {
        anyhow::bail!("calendar event end must be after start");
    }
    let provider_event_id = Uuid::new_v4().to_string();
    let event = CalendarEvent {
        id: format!("{AGENT_CALENDAR_PROVIDER}:{provider_event_id}"),
        provider_event_id,
        title: title.trim().to_string(),
        start,
        end,
        location,
        description,
        attendees: Vec::new(),
        source: "agent".to_string(),
        all_day: false,
    };
    let written = sync_events(context, AGENT_CALENDAR_PROVIDER, &[event.clone()])?;
    if written == 0 {
        anyhow::bail!("failed to persist calendar event");
    }
    Ok(event)
}

/// Move an event's start/end in the local agent calendar (#3603).
pub fn move_agent_event(
    context: &StorageContext,
    event_id: &str,
    new_start: DateTime<Utc>,
    new_end: DateTime<Utc>,
) -> Result<CalendarEvent> {
    if new_end <= new_start {
        anyhow::bail!("calendar event end must be after start");
    }
    let conn = db_conn(context)?;
    ensure_calendar_tables(&conn)?;
    let existing = conn.query_row(
        "SELECT id, provider_event_id, title, location, description,
                attendees_json, all_day, source
         FROM calendar_events WHERE id = ?1",
        params![event_id],
        |row| {
            let attendees_json: String = row.get(5)?;
            let attendees: Vec<String> =
                serde_json::from_str(&attendees_json).unwrap_or_default();
            Ok(CalendarEvent {
                id: row.get(0)?,
                provider_event_id: row.get(1)?,
                title: row.get(2)?,
                start: new_start,
                end: new_end,
                location: row.get(3)?,
                description: row.get(4)?,
                attendees,
                all_day: row.get::<_, i64>(6)? != 0,
                source: row.get(7)?,
            })
        },
    )?;
    let updated = conn.execute(
        "UPDATE calendar_events
         SET start_utc = ?1, end_utc = ?2, synced_at = ?3
         WHERE id = ?4",
        params![
            new_start.to_rfc3339(),
            new_end.to_rfc3339(),
            Utc::now().to_rfc3339(),
            event_id
        ],
    )?;
    if updated == 0 {
        anyhow::bail!("calendar event '{event_id}' not found");
    }
    Ok(existing)
}

/// Cancel (delete) an event from the local agent calendar (#3603).
pub fn cancel_agent_event(context: &StorageContext, event_id: &str) -> Result<bool> {
    let conn = db_conn(context)?;
    ensure_calendar_tables(&conn)?;
    let deleted = conn.execute(
        "DELETE FROM calendar_events WHERE id = ?1",
        params![event_id],
    )?;
    Ok(deleted > 0)
}

/// Find free time slots of at least `duration_minutes` within `window_start`
/// .. `window_end` (local day boundaries) (#3603).
pub fn find_free_slots(
    context: &StorageContext,
    day: NaiveDate,
    duration_minutes: i64,
    window_start: Option<chrono::NaiveTime>,
    window_end: Option<chrono::NaiveTime>,
) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>> {
    let start_time = window_start.unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    let end_time = window_end.unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(18, 0, 0).unwrap());
    if end_time <= start_time {
        anyhow::bail!("free-slot window end must be after start");
    }
    let day_start = day.and_time(start_time);
    let day_end = day.and_time(end_time);
    let start = Utc.from_utc_datetime(&day_start);
    let end = Utc.from_utc_datetime(&day_end);

    let existing = list_cached_events(context, start, end)?;
    let mut busy: Vec<(DateTime<Utc>, DateTime<Utc>)> = existing
        .into_iter()
        .map(|e| (e.start, e.end))
        .collect();
    busy.sort_by_key(|(s, _)| *s);

    let mut slots = Vec::new();
    let mut cursor = start;
    for (busy_start, busy_end) in busy {
        if busy_start > cursor && busy_start.signed_duration_since(cursor).num_minutes() >= duration_minutes {
            slots.push((cursor, busy_start));
        }
        if busy_end > cursor {
            cursor = busy_end;
        }
    }
    if end.signed_duration_since(cursor).num_minutes() >= duration_minutes {
        slots.push((cursor, end));
    }
    Ok(slots)
}

/// Convenience: fetch from a provider and sync to cache in one call.
pub async fn sync_from_provider(
    context: &StorageContext,
    provider_name: &str,
    provider: &dyn CalendarProvider,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<usize> {
    let events = provider.list_events(range_start, range_end).await?;
    sync_events(context, provider_name, &events)
}

fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<CalendarEvent> {
    let id: String = row.get(0)?;
    let provider_event_id: String = row.get(1)?;
    let title: String = row.get(2)?;
    let start_str: String = row.get(3)?;
    let end_str: String = row.get(4)?;
    let location: String = row.get(5)?;
    let description: String = row.get(6)?;
    let attendees_json: String = row.get(7)?;
    let all_day: i64 = row.get(8)?;
    let source: String = row.get(9)?;

    let start = DateTime::parse_from_rfc3339(&start_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| epoch());
    let end = DateTime::parse_from_rfc3339(&end_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| epoch());
    let attendees: Vec<String> = match serde_json::from_str(&attendees_json) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse attendees JSON, falling back to empty vec");
            Vec::new()
        }
    };

    Ok(CalendarEvent {
        id,
        provider_event_id,
        title,
        start,
        end,
        location: if location.is_empty() {
            None
        } else {
            Some(location)
        },
        description: if description.is_empty() {
            None
        } else {
            Some(description)
        },
        attendees,
        source,
        all_day: all_day != 0,
    })
}

// ─── Meeting Source Card ──────────────────────────────────────────

/// Structured metadata describing a calendar event attached to a note.
/// Stored in note YAML frontmatter and used by AI for search/context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSourceCard {
    pub event_id: String,
    pub title: String,
    pub organizer: Option<String>,
    pub attendees: Vec<String>,
    pub calendar_source: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub location: Option<String>,
    pub meeting_url: Option<String>,
}

impl CalendarEvent {
    /// Convert to a [`MeetingSourceCard`] for frontmatter attachment.
    /// The card is the persistent form; the event is the runtime form.
    pub fn to_source_card(&self) -> MeetingSourceCard {
        MeetingSourceCard {
            event_id: self.provider_event_id.clone(),
            title: self.title.clone(),
            organizer: None, // CalendarEvent doesn't track organizer yet
            attendees: self.attendees.clone(),
            calendar_source: self.source.clone(),
            start: self.start,
            end: self.end,
            location: self.location.clone(),
            meeting_url: None, // could be extracted from description in future
        }
    }
}

/// Detect calendar events that overlap `now` (i.e. currently in progress).
///
/// Returns events whose `[start, end]` interval contains `now`, ordered
/// by start time ascending.  Uses the cached event table via
/// [`today_agenda_cached`].
pub fn detect_current_meetings(ctx: &StorageContext, now: DateTime<Utc>) -> Vec<CalendarEvent> {
    let agenda = today_agenda_cached(ctx, now).unwrap_or_default();
    agenda
        .into_iter()
        .filter(|e| e.start <= now && e.end >= now)
        .collect()
}

/// Parse meeting metadata from note frontmatter back into a
/// [`MeetingSourceCard`].  Returns `None` if the note has no meeting keys.
pub fn extract_source_card(body: &str) -> Option<MeetingSourceCard> {
    let lines: Vec<&str> = body.lines().collect();
    if lines.first()? != &"---" {
        return None;
    }
    let close_rel = lines.iter().skip(1).position(|l| l.trim() == "---")?;
    let fm_lines = &lines[1..=close_rel];

    fn val<'a>(k: &str, lines: &[&'a str]) -> Option<&'a str> {
        lines.iter().find_map(|l| {
            let l = l.trim();
            Some(l.strip_prefix(k)?.strip_prefix(':')?.trim())
        })
    }

    let event_id = val("meeting_event_id", fm_lines)?.to_string();
    let title = val("meeting_title", fm_lines)?.to_string();
    let calendar_source = val("calendar_source", fm_lines).unwrap_or("").to_string();
    let organizer = val("meeting_organizer", fm_lines).map(|s| s.to_string());
    let location = val("meeting_location", fm_lines).map(|s| s.to_string());
    let meeting_url = val("meeting_url", fm_lines).map(|s| s.to_string());

    // Parse attendees — look for indented list under meeting_attendees:
    let mut attendees: Vec<String> = Vec::new();
    let mut in_attendees = false;
    for line in fm_lines {
        let trimmed = line.trim();
        if trimmed.starts_with("meeting_attendees:") && !trimmed.contains("[]") {
            in_attendees = true;
            continue;
        }
        if in_attendees {
            if line.starts_with(' ') || line.starts_with('\t') {
                let name = trimmed.trim_start_matches("- ").to_string();
                if !name.is_empty() {
                    attendees.push(name);
                }
            } else {
                in_attendees = false;
            }
        }
    }

    let start = val("meeting_start", fm_lines)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.to_utc())?;
    let end = val("meeting_end", fm_lines)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.to_utc())?;

    Some(MeetingSourceCard {
        event_id,
        title,
        organizer,
        attendees,
        calendar_source,
        start,
        end,
        location,
        meeting_url,
    })
}

/// Merge calendar meeting metadata into a markdown note body as YAML
/// frontmatter keys (`meeting_event_id`, `meeting_title`, `meeting_start`,
/// `meeting_end`, `meeting_attendees`).
///
/// If the note already has frontmatter, the meeting keys are updated and
/// existing non-meeting keys are preserved.  If not, a new frontmatter
/// block is prepended.  The note body content is always preserved.
pub fn attach_meeting_metadata(note_body: &str, event: &CalendarEvent) -> String {
    let meeting_lines = build_meeting_yaml_lines(event);
    let lines: Vec<&str> = note_body.lines().collect();

    let has_fence = lines
        .first()
        .is_some_and(|l| l.trim_end_matches('\r') == "---");

    if has_fence {
        // Find the closing fence (first line after position 0 that is "---").
        if let Some(close_rel) = lines
            .iter()
            .skip(1)
            .position(|l| l.trim_end_matches('\r') == "---")
        {
            let close_idx = close_rel + 1;
            let fm_lines = &lines[1..close_idx];
            let body_lines = &lines[close_idx + 1..];
            let body = body_lines.join("\n").trim_start_matches('\n').to_string();

            let preserved = filter_meeting_keys(fm_lines);

            let mut out = String::from("---\n");
            for line in &preserved {
                out.push_str(line);
                out.push('\n');
            }
            for line in &meeting_lines {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("---\n\n");
            out.push_str(&body);
            return out;
        }
    }

    // No frontmatter — create new.
    let mut out = String::from("---\n");
    for line in &meeting_lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("---\n\n");
    out.push_str(note_body);
    out
}

/// Build YAML key-value lines for meeting metadata from a
/// [`MeetingSourceCard`] (includes organizer, calendar_source, meeting_url).
pub fn build_source_card_yaml(card: &MeetingSourceCard) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("meeting_event_id: {}", yaml_scalar(&card.event_id)));
    lines.push(format!("meeting_title: {}", yaml_scalar(&card.title)));
    if let Some(ref org) = card.organizer {
        lines.push(format!("meeting_organizer: {}", yaml_scalar(org)));
    }
    if !card.calendar_source.is_empty() {
        lines.push(format!(
            "calendar_source: {}",
            yaml_scalar(&card.calendar_source)
        ));
    }
    if let Some(ref loc) = card.location {
        lines.push(format!("meeting_location: {}", yaml_scalar(loc)));
    }
    if let Some(ref url) = card.meeting_url {
        lines.push(format!("meeting_url: {}", yaml_scalar(url)));
    }
    lines.push(format!("meeting_start: \"{}\"", card.start.to_rfc3339()));
    lines.push(format!("meeting_end: \"{}\"", card.end.to_rfc3339()));
    if card.attendees.is_empty() {
        lines.push("meeting_attendees: []".to_string());
    } else {
        lines.push("meeting_attendees:".to_string());
        for a in &card.attendees {
            lines.push(format!("  - {}", yaml_scalar(a)));
        }
    }
    lines
}

/// Build YAML key-value lines for meeting metadata from a [`CalendarEvent`].
fn build_meeting_yaml_lines(event: &CalendarEvent) -> Vec<String> {
    let card = event.to_source_card();
    build_source_card_yaml(&card)
}

/// Minimal YAML scalar escaping: quote if the value contains characters
/// that would be ambiguous in YAML.
fn yaml_scalar(s: &str) -> String {
    let needs_quote = s.contains(':')
        || s.contains('#')
        || s.contains('"')
        || s.contains('\'')
        || s.starts_with('-')
        || s.starts_with(' ')
        || s.starts_with('!')
        || s.starts_with('{')
        || s.starts_with('[')
        || s.starts_with('&')
        || s.starts_with('*')
        || s.starts_with('?')
        || s.starts_with('|')
        || s.starts_with('>');
    if needs_quote {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Remove existing `meeting_*` keys (including their multi-line list items)
/// from frontmatter lines.  Returns the preserved lines.
fn filter_meeting_keys(lines: &[&str]) -> Vec<String> {
    let mut result = Vec::new();
    let mut skip_indented = false;

    for line in lines {
        let trimmed = line.trim_start();
        let is_meeting = trimmed.starts_with("meeting_");
        let is_indented = line.starts_with(' ') || line.starts_with('\t');

        if is_meeting {
            // If this is a multi-line attendees list, skip subsequent indented lines.
            skip_indented = trimmed.starts_with("meeting_attendees:") && !trimmed.contains("[]");
            continue;
        }
        if skip_indented && is_indented {
            continue;
        }
        skip_indented = false;
        result.push((*line).to_string());
    }
    result
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn epoch() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap()
    }

    fn make_event(id: &str, title: &str) -> CalendarEvent {
        CalendarEvent {
            id: id.to_string(),
            provider_event_id: id.to_string(),
            title: title.to_string(),
            start: epoch(),
            end: epoch(),
            location: None,
            description: None,
            attendees: vec![],
            source: "test".to_string(),
            all_day: false,
        }
    }

    #[test]
    fn parse_ics_multi_event() {
        let ics = "BEGIN:VCALENDAR\r
VERSION:2.0\r
BEGIN:VEVENT\r
UID:event-1@vp\r
DTSTART:20260701T100000Z\r
DTEND:20260701T110000Z\r
SUMMARY:Sprint Planning\r
LOCATION:Room A\r
DESCRIPTION:Weekly sync\r
ATTENDEE;CN=张三:mailto:zhangsan@example.com\r
ATTENDEE;CN=John Doe:mailto:john@example.com\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:event-2@vp\r
DTSTART;VALUE=DATE:20260701\r
SUMMARY:All Day Event\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:event-3@vp\r
DTSTART:20260701T140000Z\r
DTEND:20260701T150000Z\r
SUMMARY:This is a long summary that spans multiple lines because \r
 of RFC 5545 line folding and should be joined\r
END:VEVENT\r
END:VCALENDAR\r
";
        let events = parse_ics(ics, "test-cal");
        assert_eq!(events.len(), 3, "should parse 3 events");

        // Event 1
        let e1 = &events[0];
        assert_eq!(e1.provider_event_id, "event-1@vp");
        assert_eq!(e1.title, "Sprint Planning");
        assert_eq!(e1.location.as_deref(), Some("Room A"));
        assert_eq!(e1.description.as_deref(), Some("Weekly sync"));
        assert_eq!(e1.attendees, vec!["张三", "John Doe"]);
        assert!(!e1.all_day);
        let expected_start = NaiveDate::from_ymd_opt(2026, 7, 1)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_utc();
        assert_eq!(e1.start, expected_start);

        // Event 2 — all-day
        let e2 = &events[1];
        assert_eq!(e2.title, "All Day Event");
        assert!(e2.all_day);

        // Event 3 — folded line
        let e3 = &events[2];
        assert_eq!(
            e3.title,
            "This is a long summary that spans multiple lines because of RFC 5545 line folding and should be joined"
        );
    }

    #[test]
    fn parse_ics_rfc3339_timezone() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:tz-1@vp\nDTSTART:2026-07-01T10:00:00+08:00\nSUMMARY:TZ Event\nEND:VEVENT\nEND:VCALENDAR\n";
        let events = parse_ics(ics, "tz");
        assert_eq!(events.len(), 1);
        // +08:00 → UTC is 02:00
        let expected = NaiveDate::from_ymd_opt(2026, 7, 1)
            .unwrap()
            .and_hms_opt(2, 0, 0)
            .unwrap()
            .and_utc();
        assert_eq!(events[0].start, expected);
    }

    #[test]
    fn attach_metadata_no_frontmatter() {
        let event = make_event("mtg-1", "Sprint Planning");
        let note = "# My Note\nSome content here";
        let result = attach_meeting_metadata(note, &event);

        assert!(result.starts_with("---\n"), "should start with frontmatter");
        assert!(result.contains("meeting_event_id: mtg-1"));
        assert!(result.contains("meeting_title: Sprint Planning"));
        assert!(result.contains("meeting_attendees: []"));
        // Body preserved
        assert!(result.contains("# My Note"));
        assert!(result.contains("Some content here"));
    }

    #[test]
    fn attach_metadata_with_existing_frontmatter() {
        let event = make_event("mtg-2", "Review");
        event_attendees_set(&mut event.clone());
        let note = "---\ntitle: My Note\ntags: [work]\n---\n# Content\nBody text";
        let result = attach_meeting_metadata(note, &event);

        assert!(result.contains("title: My Note"), "existing key preserved");
        assert!(result.contains("tags: [work]"), "existing key preserved");
        assert!(result.contains("meeting_event_id: mtg-2"));
        assert!(result.contains("meeting_title: Review"));
        assert!(result.contains("# Content"), "body preserved");
        assert!(result.contains("Body text"), "body preserved");
    }

    fn event_attendees_set(_event: &mut CalendarEvent) {
        // Helper kept for potential future use; current test uses empty attendees.
    }

    #[test]
    fn attach_metadata_cjk_preserved() {
        let mut event = make_event("mtg-3", "");
        event.title = "产品评审会议".to_string();
        event.attendees = vec!["张三".to_string(), "李四".to_string()];
        let note = "会议内容";
        let result = attach_meeting_metadata(note, &event);

        assert!(result.contains("meeting_title: 产品评审会议"));
        assert!(result.contains("  - 张三"));
        assert!(result.contains("  - 李四"));
        assert!(result.contains("会议内容"));
    }

    #[test]
    fn attach_metadata_replaces_old_meeting_keys() {
        let event = make_event("mtg-new", "New Title");
        let note =
            "---\nmeeting_event_id: mtg-old\nmeeting_title: Old Title\ntitle: Keep\n---\nBody";
        let result = attach_meeting_metadata(note, &event);

        assert!(!result.contains("mtg-old"), "old meeting_event_id removed");
        assert!(!result.contains("Old Title"), "old meeting_title removed");
        assert!(result.contains("meeting_event_id: mtg-new"));
        assert!(result.contains("meeting_title: New Title"));
        assert!(result.contains("title: Keep"), "non-meeting key preserved");
    }

    #[test]
    fn unfold_line_folding() {
        let content = "FIRST:this is a \r\n very long line \r\n that continues\r\nSECOND:value\r\n";
        let lines = unfold_lines(content);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "FIRST:this is a very long line that continues");
        assert_eq!(lines[1], "SECOND:value");
    }

    #[test]
    fn row_to_event_invalid_attendees_json_returns_empty_vec() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS calendar_events (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL DEFAULT '',
                provider_event_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                start_utc TEXT NOT NULL,
                end_utc TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '',
                description TEXT NOT NULL DEFAULT '',
                attendees_json TEXT NOT NULL DEFAULT '[]',
                all_day INTEGER NOT NULL DEFAULT 0,
                source TEXT NOT NULL DEFAULT '',
                synced_at TEXT NOT NULL
            );",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO calendar_events
             (id, provider, provider_event_id, title, start_utc, end_utc,
              location, description, attendees_json, all_day, source, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                "bad-attendees-1",
                "test",
                "evt-1",
                "Broken Attendees",
                "2026-07-01T10:00:00+00:00",
                "2026-07-01T11:00:00+00:00",
                "",
                "",
                "not-valid-json!!",
                0,
                "test",
                "2026-07-01T00:00:00Z",
            ],
        )
        .unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT id, provider_event_id, title, start_utc, end_utc,
                        location, description, attendees_json, all_day, source
                 FROM calendar_events WHERE id = ?1",
            )
            .unwrap();
        let events: Vec<CalendarEvent> = stmt
            .query_map(params!["bad-attendees-1"], row_to_event)
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(events.len(), 1);
        assert!(
            events[0].attendees.is_empty(),
            "invalid JSON should yield empty attendees vec"
        );
        assert_eq!(events[0].title, "Broken Attendees");
        assert_eq!(events[0].id, "bad-attendees-1");
    }

    #[test]
    fn sync_and_cached_agenda() {
        let temp = std::env::temp_dir().join(format!(
            "vp-cal-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");

        // Insert a today event
        let now = Utc::now();
        let event = CalendarEvent {
            id: "ics-today@vp".to_string(),
            provider_event_id: "today@vp".to_string(),
            title: "Today Meeting".to_string(),
            start: now.date_naive().and_hms_opt(10, 0, 0).unwrap().and_utc(),
            end: now.date_naive().and_hms_opt(11, 0, 0).unwrap().and_utc(),
            location: Some("Room B".to_string()),
            description: None,
            attendees: vec!["Alice".to_string()],
            source: "test".to_string(),
            all_day: false,
        };
        let count = sync_events(&ctx, "ics", &[event]).expect("sync");
        assert_eq!(count, 1);

        let agenda = today_agenda_cached(&ctx, now).expect("cached agenda");
        assert_eq!(agenda.len(), 1);
        assert_eq!(agenda[0].title, "Today Meeting");
        assert_eq!(agenda[0].attendees, vec!["Alice"]);

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn agent_calendar_crud_round_trip() {
        // #3603: create → list → move → cancel against the local agent
        // calendar store.
        let temp = std::env::temp_dir().join(format!("vp_cal_crud_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");

        let day = Utc::now().date_naive();
        let start = day.and_hms_opt(10, 0, 0).unwrap().and_utc();
        let end = day.and_hms_opt(11, 0, 0).unwrap().and_utc();
        let created = create_agent_event(
            &ctx,
            "Agent Standup",
            start,
            end,
            Some("Room A".to_string()),
            Some("Daily sync".to_string()),
        )
        .expect("create");
        assert!(created.id.starts_with("agent:"));
        assert_eq!(created.title, "Agent Standup");

        // Create must reject invalid ranges.
        assert!(create_agent_event(&ctx, "Bad", end, start, None, None).is_err());
        assert!(create_agent_event(&ctx, "  ", start, end, None, None).is_err());

        let listed = list_cached_events(&ctx, start, end).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Agent Standup");

        let new_start = day.and_hms_opt(14, 0, 0).unwrap().and_utc();
        let new_end = day.and_hms_opt(15, 0, 0).unwrap().and_utc();
        let moved = move_agent_event(&ctx, &created.id, new_start, new_end).expect("move");
        assert_eq!(moved.title, "Agent Standup");
        assert_eq!(moved.start, new_start);

        assert!(move_agent_event(&ctx, "agent:nope", new_start, new_end).is_err());
        assert!(cancel_agent_event(&ctx, &created.id).expect("cancel"));
        assert!(!cancel_agent_event(&ctx, &created.id).expect("cancel again"));
        assert!(list_cached_events(&ctx, start, end).expect("list empty").is_empty());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn find_free_slots_skips_busy_ranges() {
        // #3603: a 1h meeting at 10:00-11:00 leaves 09:00-10:00 and
        // 11:00-18:00 free for a 1h duration.
        let temp = std::env::temp_dir().join(format!("vp_cal_slots_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");

        let day = Utc::now().date_naive();
        let start = day.and_hms_opt(10, 0, 0).unwrap().and_utc();
        let end = day.and_hms_opt(11, 0, 0).unwrap().and_utc();
        create_agent_event(&ctx, "Busy", start, end, None, None).expect("create");

        let slots = find_free_slots(&ctx, day, 60, None, None).expect("slots");
        assert_eq!(slots.len(), 2, "expected two free slots, got {slots:?}");
        assert_eq!(slots[0].0.time(), chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        assert_eq!(slots[0].1.time(), chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap());
        assert_eq!(slots[1].0.time(), chrono::NaiveTime::from_hms_opt(11, 0, 0).unwrap());
        assert_eq!(slots[1].1.time(), chrono::NaiveTime::from_hms_opt(18, 0, 0).unwrap());

        // A 2h request cannot fit the 09:00-10:00 slot.
        let slots2 = find_free_slots(&ctx, day, 120, None, None).expect("slots 2h");
        assert_eq!(slots2.len(), 1);
        assert_eq!(slots2[0].0.time(), chrono::NaiveTime::from_hms_opt(11, 0, 0).unwrap());

        let _ = std::fs::remove_dir_all(&temp);
    }
}
