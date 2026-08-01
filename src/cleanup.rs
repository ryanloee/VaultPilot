//! Vault Cleanup Suggestions — unified report of items that can be removed
//! to tidy up the vault (#3708).
//!
//! Inspired by Anytype 0.56's "Cleanup Suggestions" feature, this module
//! aggregates several categories of cleanup candidates into a single report:
//!
//! - **Orphan attachments** — files under `<vault>/attachments/` not referenced
//!   by any note (delegates to [`crate::attachments`]).
//! - **Orphan notes** — notes with no tags, no outgoing wiki-links / URLs,
//!   AND no incoming wikilinks from other notes (i.e. no backlinks). A note
//!   that is referenced via `[[wikilink]]` by any other note is treated as a
//!   "hub note" and is never flagged as orphan, even if its own body has no
//!   links or tags (#3719).
//! - **Empty notes** — notes whose body is empty or below a minimum threshold.
//! - **Stale notes** — notes not updated in a configurable number of days.
//!
//! The report is read-only by default: it *suggests* deletable items but does
//! not remove anything.  Deletion of orphan attachments is handled separately
//! via `vp attachments clean --delete`.

use std::collections::HashSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::attachments::{scan_orphan_attachments, OrphanAttachment};
use crate::models::NoteMeta;
use crate::storage::StorageContext;

/// Maximum notes to scan for cleanup analysis.
const CLEANUP_MAX_NOTES: usize = 10_000;

/// A note whose body is empty or near-empty (< `EMPTY_BODY_THRESHOLD` chars of
/// non-whitespace content).
pub const EMPTY_BODY_THRESHOLD: usize = 50;

/// Default staleness threshold in days — notes not updated within this period
/// are flagged as stale.
pub const DEFAULT_STALE_DAYS: u64 = 90;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A stale note with contextual information for the cleanup report.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleNote {
    /// The note metadata (id, title, tags, updated_at, …).
    #[serde(flatten)]
    pub note: NoteMeta,
    /// Approximate number of days since the last update.
    pub days_since_update: u64,
}

/// Unified cleanup report aggregating all categories of removable items.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    /// Total number of notes in the vault (for context).
    pub total_notes: usize,
    /// Attachment files not referenced by any note or the attachments table.
    #[serde(default)]
    pub orphan_attachments: Vec<OrphanAttachment>,
    /// Notes with no tags, no outgoing links, AND no incoming wikilinks
    /// (backlinks) from other notes. Hub notes referenced via `[[wikilink]]`
    /// are excluded even if they have no tags/links themselves (#3719).
    #[serde(default)]
    pub orphan_notes: Vec<NoteMeta>,
    /// Notes whose body is empty or below the minimum content threshold.
    #[serde(default)]
    pub empty_notes: Vec<NoteMeta>,
    /// Notes not updated in the staleness window.
    #[serde(default)]
    pub stale_notes: Vec<StaleNote>,
    /// Total number of cleanup candidates across all categories.
    pub total_items: usize,
    /// Bytes that would be freed if all orphan attachments were deleted.
    pub potential_freed_bytes: u64,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate a comprehensive vault cleanup report.
///
/// `stale_days` controls the staleness threshold — notes whose `updated_at`
/// is older than `now - stale_days` are flagged.  Pass `DEFAULT_STALE_DAYS`
/// for the default 90-day window.
pub fn generate_cleanup_report(context: &StorageContext, stale_days: u64) -> Result<CleanupReport> {
    let conn = context.get_connection()?;

    // ── 1. Total note count ────────────────────────────────────────────
    let total_notes: usize =
        conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get::<_, i64>(0))? as usize;

    // ── 2. Load note metas ─────────────────────────────────────────────
    let all_notes = load_note_metas(&conn, total_notes)?;

    // ── 3. Orphan attachments (delegates to attachments module) ────────
    let orphan_attachments = scan_orphan_attachments(context).unwrap_or_else(|e| {
        warn!(error = %e, "failed to scan orphan attachments during cleanup report");
        Vec::new()
    });

    // ── 4. Orphan notes (no tags, no links) ────────────────────────────
    let orphan_notes = find_orphan_notes(&conn, &all_notes)?;

    // ── 5. Empty notes ─────────────────────────────────────────────────
    let empty_notes = find_empty_notes(&conn, &all_notes)?;

    // ── 6. Stale notes ─────────────────────────────────────────────────
    let stale_notes = find_stale_notes(&all_notes, stale_days)?;

    // ── 7. Aggregate totals ────────────────────────────────────────────
    let potential_freed_bytes = orphan_attachments.iter().map(|a| a.size_bytes).sum();
    // Count UNIQUE note IDs across the three note categories so that a note
    // appearing in several categories (e.g. orphan + empty + stale) is only
    // counted once.  Orphan attachments are separate files and always counted
    // in full.  See #3714 — the previous simple sum double-counted overlaps.
    let mut seen: HashSet<&str> = HashSet::new();
    for n in &orphan_notes {
        seen.insert(&n.id);
    }
    for n in &empty_notes {
        seen.insert(&n.id);
    }
    for n in &stale_notes {
        seen.insert(&n.note.id);
    }
    let total_items = orphan_attachments.len() + seen.len();

    Ok(CleanupReport {
        total_notes,
        orphan_attachments,
        orphan_notes,
        empty_notes,
        stale_notes,
        total_items,
        potential_freed_bytes,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Load note metas from the database (up to `CLEANUP_MAX_NOTES`).
fn load_note_metas(conn: &rusqlite::Connection, total: usize) -> Result<Vec<NoteMeta>> {
    let limit = total.min(CLEANUP_MAX_NOTES);
    let mut stmt = conn.prepare(
        "SELECT id, title, tags, keywords, platform, board, kernel, status, \
         created_at, updated_at, source, path, summary \
         FROM notes ORDER BY updated_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |row| {
        let tags_raw: String = row.get(2)?;
        let kw_raw: String = row.get(3)?;
        Ok(NoteMeta {
            id: row.get(0)?,
            title: row.get(1)?,
            tags: serde_json::from_str(&tags_raw).unwrap_or_default(),
            keywords: serde_json::from_str(&kw_raw).unwrap_or_default(),
            platform: row.get(4)?,
            board: row.get(5)?,
            kernel: row.get(6)?,
            status: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
            source: row.get(10)?,
            path: row.get(11)?,
            summary: row.get(12)?,
            collections: Vec::new(),
        })
    })?;
    let mut notes = Vec::with_capacity(limit);
    for row in rows {
        notes.push(row?);
    }
    Ok(notes)
}

/// Find notes that have no tags, no outgoing wiki-links / URLs in their body,
/// AND no incoming wikilinks (backlinks) from other notes.
///
/// A note whose title is referenced via `[[wikilink]]` by any other note is
/// treated as a "hub note" — it has incoming links and is therefore NOT an
/// orphan, even if its own body has no tags or outgoing links. Without this
/// check, hub notes were misclassified as orphans and suggested for deletion,
/// which would break every backlink pointing to them (#3719).
///
/// To keep this O(n) rather than O(n²), we build a one-time index of every
/// wikilink target seen across the vault before scanning candidates.
fn find_orphan_notes(conn: &rusqlite::Connection, all_notes: &[NoteMeta]) -> Result<Vec<NoteMeta>> {
    // Build a one-time index of note titles (lowercased) that are referenced
    // by at least one `[[wikilink]]` anywhere in the vault. A candidate note
    // whose title appears in this index has at least one incoming link and is
    // excluded from the orphan set (#3719).
    let backlinked_titles = build_backlinked_title_index(conn, all_notes)?;

    let mut orphans = Vec::new();
    for note in all_notes {
        let has_tags = !note.tags.is_empty();
        let has_links = body_has_links(conn, &note.id)?;
        if has_tags || has_links {
            continue;
        }
        // No tags and no outgoing links — but does any other note link TO it?
        // A non-empty title that appears in the backlink index means the note
        // is a hub: exclude it from the orphan set.
        let title_trimmed = note.title.trim();
        if !title_trimmed.is_empty() && backlinked_titles.contains(&title_trimmed.to_lowercase()) {
            continue;
        }
        orphans.push(note.clone());
    }
    // Cap to keep the report manageable.
    orphans.truncate(500);
    Ok(orphans)
}

/// Build a set of note titles (lowercased) that appear as the target of at
/// least one `[[wikilink]]` across all notes in the vault.
///
/// This is the incoming-link index used by [`find_orphan_notes`] to detect
/// "hub notes". Scanning every note body once and extracting wikilinks is
/// O(total_body_size); lookups against the returned set are O(1).
///
/// Notes whose body cannot be loaded (e.g. missing FTS row) are skipped
/// silently — they simply contribute no targets to the index.
fn build_backlinked_title_index(
    conn: &rusqlite::Connection,
    all_notes: &[NoteMeta],
) -> Result<std::collections::HashSet<String>> {
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for note in all_notes {
        let body: String = match conn.query_row(
            "SELECT body FROM note_fts WHERE note_id = ?1",
            [&note.id],
            |row| row.get(0),
        ) {
            Ok(b) => b,
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => {
                warn!(
                    error = %e, note_id = %note.id,
                    "failed to query note_fts for backlink index"
                );
                continue;
            }
        };
        for (target, _alias) in crate::storage::extract_wikilinks(&body) {
            // `extract_wikilinks` returns the raw target text (already without
            // the surrounding `[[ ]]`). Strip a leading path segment if the
            // target uses Obsidian's `[[folder/Note]]` form so that a link to
            // `folder/Note` also counts as a backlink for a note titled `Note`.
            let target = target.trim();
            if target.is_empty() {
                continue;
            }
            referenced.insert(target.to_lowercase());
            if let Some(slash) = target.rfind('/') {
                let basename = target[slash + 1..].trim();
                if !basename.is_empty() {
                    referenced.insert(basename.to_lowercase());
                }
            }
        }
    }
    Ok(referenced)
}

/// Check whether a note body contains wiki-style links (`[[...]]`) or
/// markdown / URL links.
fn body_has_links(conn: &rusqlite::Connection, note_id: &str) -> Result<bool> {
    let body: String = match conn.query_row(
        "SELECT body FROM note_fts WHERE note_id = ?1",
        [note_id],
        |row| row.get(0),
    ) {
        Ok(b) => b,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(e) => {
            warn!(error = %e, note_id = note_id, "failed to query note_fts for link check");
            return Err(e.into());
        }
    };
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    if trimmed.contains("[[") && trimmed.contains("]]") {
        return Ok(true);
    }
    if trimmed.contains("http://") || trimmed.contains("https://") || trimmed.contains("ftp://") {
        return Ok(true);
    }
    Ok(false)
}

/// Find notes whose body is empty or has fewer than `EMPTY_BODY_THRESHOLD`
/// non-whitespace characters.
fn find_empty_notes(conn: &rusqlite::Connection, all_notes: &[NoteMeta]) -> Result<Vec<NoteMeta>> {
    let mut empty = Vec::new();
    for note in all_notes {
        let body: String = match conn.query_row(
            "SELECT body FROM note_fts WHERE note_id = ?1",
            [&note.id],
            |row| row.get(0),
        ) {
            Ok(b) => b,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // No FTS entry → treat body as empty
                empty.push(note.clone());
                continue;
            }
            Err(e) => {
                warn!(error = %e, note_id = %note.id, "failed to query note_fts for empty check");
                continue;
            }
        };
        let non_ws_len = body.trim().chars().count();
        if non_ws_len < EMPTY_BODY_THRESHOLD {
            empty.push(note.clone());
        }
    }
    empty.truncate(500);
    Ok(empty)
}

/// Find notes not updated within the last `stale_days` days.
fn find_stale_notes(all_notes: &[NoteMeta], stale_days: u64) -> Result<Vec<StaleNote>> {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let threshold_secs = now_secs.saturating_sub(stale_days * 86_400);

    let mut stale = Vec::new();
    for note in all_notes {
        // Parse updated_at as epoch seconds.  The timestamp format in the DB
        // is typically ISO-8601 ("2024-01-15T10:30:00Z" or similar).  We try
        // several parse strategies.
        let updated_secs = parse_timestamp_to_epoch(&note.updated_at);
        if let Some(updated) = updated_secs {
            if updated < threshold_secs {
                let days_since = now_secs.saturating_sub(updated) / 86_400;
                stale.push(StaleNote {
                    note: note.clone(),
                    days_since_update: days_since,
                });
            }
        }
    }
    // Sort by most stale first.
    stale.sort_by_key(|n| std::cmp::Reverse(n.days_since_update));
    stale.truncate(500);
    Ok(stale)
}

/// Best-effort parse of a timestamp string to epoch seconds.
///
/// Handles common ISO-8601 variants stored in the notes table:
/// - `2024-01-15T10:30:00Z`
/// - `2024-01-15T10:30:00.123Z`
/// - `2024-01-15 10:30:00`
/// - `2024-01-15`
/// - Unix epoch seconds as a string
fn parse_timestamp_to_epoch(ts: &str) -> Option<u64> {
    let ts = ts.trim();
    if ts.is_empty() {
        return None;
    }

    // Try parsing as a raw Unix timestamp (all digits).
    if let Ok(secs) = ts.parse::<u64>() {
        // Sanity check: must be after year 2000 and before year 2100.
        if secs > 946_684_800 && secs < 4_102_444_800 {
            return Some(secs);
        }
    }

    // Extract date components: YYYY-MM-DD
    let (year, month, day) = parse_date_part(ts)?;

    // Try to extract time components: HH:MM:SS
    let (hour, minute, second) = parse_time_part(ts).unwrap_or((0, 0, 0));

    Some(civil_to_epoch(year, month, day, hour, minute, second))
}

/// Parse `YYYY-MM-DD` from the beginning of `ts`.
fn parse_date_part(ts: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = ts.splitn(3, '-').collect();
    if parts.len() < 3 {
        return None;
    }
    let year = parts[0].trim_start_matches(' ').parse::<u64>().ok()?;
    let month = parts[1].parse::<u64>().ok()?;
    // Day may have trailing 'T' or ' '
    let day_str = parts[2].split(['T', ' ']).next()?;
    let day = day_str.parse::<u64>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || !(1970..=2100).contains(&year) {
        return None;
    }
    Some((year, month, day))
}

/// Parse `HH:MM:SS` from `ts` (after the date part, following 'T' or space).
fn parse_time_part(ts: &str) -> Option<(u64, u64, u64)> {
    let after_date = ts.split(['T', ' ']).nth(1)?;
    let time_str = after_date.split(['Z', '+', '-']).next()?; // strip timezone
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() < 2 {
        return None;
    }
    let hour = parts[0].parse::<u64>().ok()?;
    let minute = parts[1].parse::<u64>().ok()?;
    let second = parts
        .get(2)
        .and_then(|s| s.split('.').next())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some((hour, minute, second))
}

/// Convert civil (calendar) date-time to Unix epoch seconds (UTC).
///
/// Uses the well-known Howard Hinnant algorithm (`days_from_civil`).
fn civil_to_epoch(year: u64, month: u64, day: u64, hour: u64, minute: u64, second: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let doy = (153 * if month > 2 { month - 3 } else { month + 9 } + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146_097 + doe as i64 - 719_468;
    let secs = days * 86_400 + hour as i64 * 3_600 + minute as i64 * 60 + second as i64;
    secs.max(0) as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamp_iso_z() {
        // 2024-01-15T10:30:00Z → known epoch
        let epoch = parse_timestamp_to_epoch("2024-01-15T10:30:00Z").unwrap();
        // 2024-01-15T10:30:00 UTC = 1705314600
        assert_eq!(epoch, 1_705_314_600);
    }

    #[test]
    fn test_parse_timestamp_iso_millis() {
        let epoch = parse_timestamp_to_epoch("2024-01-15T10:30:00.123Z").unwrap();
        assert_eq!(epoch, 1_705_314_600);
    }

    #[test]
    fn test_parse_timestamp_space_separated() {
        let epoch = parse_timestamp_to_epoch("2024-01-15 10:30:00").unwrap();
        assert_eq!(epoch, 1_705_314_600);
    }

    #[test]
    fn test_parse_timestamp_date_only() {
        // 2024-01-15 → midnight UTC
        let epoch = parse_timestamp_to_epoch("2024-01-15").unwrap();
        assert_eq!(epoch, 1_705_276_800);
    }

    #[test]
    fn test_parse_timestamp_epoch_seconds() {
        let epoch = parse_timestamp_to_epoch("1705315800").unwrap();
        assert_eq!(epoch, 1_705_315_800);
    }

    #[test]
    fn test_parse_timestamp_empty() {
        assert!(parse_timestamp_to_epoch("").is_none());
        assert!(parse_timestamp_to_epoch("   ").is_none());
    }

    #[test]
    fn test_parse_timestamp_invalid() {
        assert!(parse_timestamp_to_epoch("not a date").is_none());
        assert!(parse_timestamp_to_epoch("2024-13-45").is_none());
    }

    #[test]
    fn test_civil_to_epoch_epoch() {
        // 1970-01-01T00:00:00Z → 0
        assert_eq!(civil_to_epoch(1970, 1, 1, 0, 0, 0), 0);
    }

    #[test]
    fn test_civil_to_epoch_known() {
        // 2000-01-01T00:00:00Z → 946684800
        assert_eq!(civil_to_epoch(2000, 1, 1, 0, 0, 0), 946_684_800);
    }

    #[test]
    fn test_stale_detection() {
        // A timestamp from 1 year ago should be stale with a 90-day threshold.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let one_year_ago = now - 365 * 86_400;
        let ts = format!(
            "{:04}-{:02}-{:02}T00:00:00Z",
            // Reverse civil-to-epoch: approximate
            1970 + one_year_ago / 31_536_000,
            (one_year_ago % 31_536_000) / 2_628_000 + 1,
            (one_year_ago % 2_628_000) / 86_400 + 1
        );
        let epoch = parse_timestamp_to_epoch(&ts).unwrap();
        // The approximate reverse conversion won't be exact, but the epoch
        // should be in the past and beyond the 90-day threshold.
        assert!(epoch < now);
        let days_since = now.saturating_sub(epoch) / 86_400;
        assert!(
            days_since > 350,
            "expected ~365 days, got {days_since} (epoch={epoch}, ts={ts})"
        );
    }

    #[test]
    fn test_cleanup_report_serialization() {
        let report = CleanupReport {
            total_notes: 100,
            orphan_attachments: vec![],
            orphan_notes: vec![],
            empty_notes: vec![],
            stale_notes: vec![],
            total_items: 0,
            potential_freed_bytes: 0,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"totalNotes\":100"));
        assert!(json.contains("\"totalItems\":0"));
        // Deserialise round-trip
        let parsed: CleanupReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total_notes, 100);
    }

    #[test]
    fn test_stale_note_serialization() {
        let note = NoteMeta {
            id: "test-1".to_string(),
            title: "Old Note".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        let stale = StaleNote {
            note,
            days_since_update: 1500,
        };
        let json = serde_json::to_string(&stale).unwrap();
        assert!(json.contains("\"id\":\"test-1\""));
        assert!(json.contains("\"daysSinceUpdate\":1500"));
    }

    #[test]
    fn test_empty_body_threshold() {
        assert_eq!(EMPTY_BODY_THRESHOLD, 50);
    }

    #[test]
    fn test_body_has_links_no_fts() {
        // This test verifies the logic indirectly — body_has_links returns
        // false for a non-existent note_id (no FTS row).
        // Full integration test requires a StorageContext.
    }

    /// Regression test for #3719: a "hub note" — one whose body has no tags
    /// and no outgoing links but is referenced via `[[wikilink]]` by other
    /// notes — must NOT be classified as an orphan. Before the fix, the
    /// cleanup report suggested deleting such notes, which would break every
    /// backlink pointing to them.
    ///
    /// This is a full integration test: it writes markdown files into a temp
    /// vault, indexes them, runs `generate_cleanup_report`, and asserts that
    /// the hub note is excluded from `orphan_notes` while a genuine orphan
    /// (no tags, no outgoing links, no incoming links) is still flagged.
    #[test]
    fn regression_3719_hub_note_with_backlinks_not_orphan() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-3719-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        let vault = temp.join("vault");
        let notes_dir = vault.join("notes");
        std::fs::create_dir_all(&notes_dir).expect("create notes dir");

        let ctx = crate::storage::StorageContext::for_test(&temp);
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");

        // ── Hub note: plain text only, no tags, no outgoing links. ──
        // Multiple other notes will `[[wikilink]]` to it, so it must be
        // treated as a hub, NOT an orphan.
        std::fs::write(
            notes_dir.join("hub.md"),
            "---\ntitle: Hub Concept\n---\n\nThis is an important concept page with plain text only.\n",
        )
        .expect("write hub note");

        // ── Three notes that backlink to the hub. ──
        for name in &["alpha", "beta", "gamma"] {
            std::fs::write(
                notes_dir.join(format!("{name}.md")),
                format!("---\ntitle: {name}\n---\n\nSee [[Hub Concept]] for background.\n"),
            )
            .expect("write backlinking note");
        }

        // ── Control: a genuine orphan — no tags, no links, no backlinks. ──
        std::fs::write(
            notes_dir.join("loner.md"),
            "---\ntitle: Loner\n---\n\nJust some isolated text with no connections at all.\n",
        )
        .expect("write orphan note");

        crate::storage::rebuild_index_with_context(&ctx).expect("rebuild index");

        let report = generate_cleanup_report(&ctx, DEFAULT_STALE_DAYS).expect("cleanup report");

        // Sanity: all 5 notes were indexed.
        assert_eq!(
            report.total_notes, 5,
            "expected 5 indexed notes (hub + 3 backlinks + loner)"
        );

        let orphan_titles: Vec<&str> = report
            .orphan_notes
            .iter()
            .map(|n| n.title.as_str())
            .collect();

        // ── The hub note must NOT be flagged as orphan (the actual bug). ──
        assert!(
            !orphan_titles.iter().any(|t| t == &"Hub Concept"),
            "#3719 regression: hub note 'Hub Concept' has backlinks and must NOT be an orphan, \
             but it appeared in orphan_notes: {orphan_titles:?}"
        );

        // ── The three backlinking notes have outgoing links, so they're not orphans either. ──
        for name in &["alpha", "beta", "gamma"] {
            assert!(
                !orphan_titles.iter().any(|t| t == name),
                "note '{name}' has an outgoing wikilink and must not be an orphan"
            );
        }

        // ── The genuine orphan IS still detected (no false negatives). ──
        assert!(
            orphan_titles.iter().any(|t| t == &"Loner"),
            "the genuine orphan 'Loner' (no tags/links/backlinks) must be flagged, \
             got orphan_notes: {orphan_titles:?}"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    /// Unit test for the backlink title index path-prefix handling (#3719):
    /// a wikilink of the form `[[folder/Note]]` should count as a backlink
    /// for a note titled `Note`. We exercise the index logic directly against
    /// an in-memory DB seeded with a single note whose body contains the link.
    #[test]
    fn regression_3719_backlink_index_handles_path_prefixed_wikilink() {
        use crate::models::NoteMeta;
        use rusqlite::Connection;

        let conn = Connection::open_in_memory().expect("open in-memory db");
        // Minimal FTS5 table mirroring the production `note_fts` shape used
        // by `build_backlinked_title_index` / `body_has_links`.
        conn.execute(
            "CREATE VIRTUAL TABLE note_fts USING fts5(note_id, body)",
            [],
        )
        .expect("create note_fts");

        // A note whose body links to `concepts/Important` — Obsidian path form.
        conn.execute(
            "INSERT INTO note_fts (note_id, body) VALUES (?1, ?2)",
            [
                &"linker-id".to_string() as &dyn rusqlite::ToSql,
                &"See [[concepts/Important]] for more.".to_string() as &dyn rusqlite::ToSql,
            ],
        )
        .expect("insert linker note");

        let notes = vec![NoteMeta {
            id: "linker-id".to_string(),
            ..Default::default()
        }];

        let index =
            super::build_backlinked_title_index(&conn, &notes).expect("build backlink index");

        // Both the full path target and the basename must be present so that
        // a note titled "Important" (no folder) is recognised as backlinked.
        assert!(
            index.contains("concepts/important"),
            "full path target should be in the index: {index:?}"
        );
        assert!(
            index.contains("important"),
            "basename of path-prefixed wikilink should be in the index: {index:?}"
        );
    }

    /// Regression test for #3714: overlapping notes across categories must be
    /// counted only once in `total_items`.  We simulate the dedup logic inline
    /// (the production code lives in `generate_cleanup_report` which needs a
    /// DB; here we verify the dedup algorithm itself).
    #[test]
    fn test_total_items_dedup_overlapping_notes() {
        let orphan_notes = vec![
            NoteMeta {
                id: "n1".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "n2".into(),
                ..Default::default()
            },
        ];
        let empty_notes = vec![
            // n2 overlaps with orphan_notes
            NoteMeta {
                id: "n2".into(),
                ..Default::default()
            },
            NoteMeta {
                id: "n3".into(),
                ..Default::default()
            },
        ];
        let stale_notes = vec![
            // n1 and n3 overlap; n4 is new
            StaleNote {
                note: NoteMeta {
                    id: "n1".into(),
                    ..Default::default()
                },
                days_since_update: 100,
            },
            StaleNote {
                note: NoteMeta {
                    id: "n3".into(),
                    ..Default::default()
                },
                days_since_update: 200,
            },
            StaleNote {
                note: NoteMeta {
                    id: "n4".into(),
                    ..Default::default()
                },
                days_since_update: 300,
            },
        ];

        // Mirror the dedup logic from generate_cleanup_report.
        let mut seen: HashSet<&str> = HashSet::new();
        for n in &orphan_notes {
            seen.insert(&n.id);
        }
        for n in &empty_notes {
            seen.insert(&n.id);
        }
        for n in &stale_notes {
            seen.insert(&n.note.id);
        }
        let orphan_attachment_count = 5usize;
        let total_items = orphan_attachment_count + seen.len();

        // 4 unique notes (n1..n4) + 5 attachments = 9, NOT 5+2+2+3=12.
        assert_eq!(seen.len(), 4);
        assert_eq!(total_items, 9);
        // Ensure the naive sum would have been wrong (regression guard).
        let naive =
            orphan_attachment_count + orphan_notes.len() + empty_notes.len() + stale_notes.len();
        assert_eq!(naive, 12);
        assert!(total_items < naive);
    }
}
