//! Calendar view — render vault notes on an interactive month-grid calendar
//! (#3182).
//!
//! Distinct from [`crate::calendar`], which is *event/meeting* integration
//! (ICS parsing, today agenda). This module is the **view layer** that takes
//! vault notes (via frontmatter dates) and lays them out on a calendar grid,
//! mirroring Obsidian's planned "Calendar view for Bases" feature.
//!
//! # Pipeline
//!
//! 1. Build [`CalendarEntry`]s from vault [`vault_query::Record`]s (or any
//!    iterable of `(path, date, title)` tuples) via [`entries_from_records`].
//! 2. Optionally filter by tag / folder / arbitrary query (the caller runs the
//!    filter; this module is render-only).
//! 3. Render with [`render_month_grid`] for a text/CLI month grid.
//!
//! The CLI subcommand is `vp vault calendar` (see
//! `src/bin/vaultpilot-cli/main.rs`). WinUI/Mobile render the same data via
//! their native calendar widgets — see issue #3182.

use chrono::{Datelike, Local, NaiveDate, Weekday};

use crate::vault_query::{QValue, Record};

/// One note positioned on a calendar date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEntry {
    /// Note path (or id) — used as a stable key for click-through.
    pub note_path: String,
    /// The calendar date this note belongs to.
    pub date: NaiveDate,
    /// Optional title for display (first heading or frontmatter `title`).
    pub title: Option<String>,
}

/// Which day of the week a calendar grid starts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekStart {
    /// Sunday-first (US convention).
    Sunday,
    /// Monday-first (ISO 8601 / most of the world).
    Monday,
}

impl WeekStart {
    /// Column index (0-based, leftmost) for the given weekday under this
    /// week-start convention.
    fn column_for(&self, wd: Weekday) -> usize {
        match self {
            WeekStart::Sunday => wd.num_days_from_sunday() as usize,
            WeekStart::Monday => wd.num_days_from_monday() as usize,
        }
    }
}

/// Default frontmatter fields probed for a note's calendar date, in priority
/// order. Mirrors the field set Obsidian's Daily Notes / Calendar plugins use.
pub const DEFAULT_DATE_FIELDS: &[&str] = &["date", "created", "published", "day"];

/// Build [`CalendarEntry`]s from vault records by extracting a date from the
/// first matching field in `date_fields`. Records with no parseable date are
/// skipped (return value contains only entries that resolved to a date).
///
/// `title_field` is optional — if present and the record has a non-empty
/// `title`/`name`/etc. property, it becomes the entry title; otherwise the
/// entry has no title and the caller may derive one from the note body.
pub fn entries_from_records(
    records: &[Record],
    date_fields: &[&str],
    title_field: Option<&str>,
) -> Vec<CalendarEntry> {
    let mut out = Vec::with_capacity(records.len());
    for rec in records {
        let date = date_fields
            .iter()
            .find_map(|field| rec.props.get(*field))
            .and_then(extract_date);
        let Some(date) = date else { continue };
        let title = title_field
            .and_then(|f| rec.props.get(f))
            .and_then(|v| match v {
                QValue::Text(s) => {
                    let t = s.trim();
                    if t.is_empty() {
                        None
                    } else {
                        Some(t.to_string())
                    }
                }
                _ => None,
            });
        out.push(CalendarEntry {
            note_path: rec.path.clone(),
            date,
            title,
        });
    }
    out
}

/// Try to pull a [`NaiveDate`] out of a [`QValue`]. Accepts:
/// - `QValue::Date(d)`
/// - `QValue::Text("YYYY-MM-DD")` (also `"YYYY/MM/DD"`)
/// - `QValue::Text("YYYY-MM-DDTHH:MM:SSZ"|"YYYY-MM-DD HH:MM:SS")` — datetime
///   strings; the date part is taken.
fn extract_date(v: &QValue) -> Option<NaiveDate> {
    match v {
        QValue::Date(d) => Some(*d),
        QValue::Text(s) => parse_flexible_date(s),
        _ => None,
    }
}

/// Parse a handful of common date/datetime formats. Returns just the date.
fn parse_flexible_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Try pure date first.
    let normalized = s.replace('/', "-");
    // Take the date prefix before 'T' or space (datetime forms).
    let date_part = normalized
        .split_once(['T', ' '])
        .map(|(d, _)| d)
        .unwrap_or(&normalized);
    NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()
}

/// Render a month-grid calendar for the given year/month, marking days that
/// have entries with the count (and title of the first entry, truncated).
///
/// The grid is plain ASCII so it works in any terminal. Example:
///
/// ```text
///     July 2026
/// Su Mo Tu We Th Fr Sa
///          1  2  3  4
///  5[3] 6  7  8[1] 9 10
/// 11 12 13 14 15 16 17
/// 18 19 20 21[2]22 23 24
/// 25 26 27 28 29 30 31
/// ```
///
/// Days with entries are suffixed `[N]` where N is the entry count. If
/// `--with-titles` is requested, the first entry's title is rendered on the
/// line below the date.
pub fn render_month_grid(
    year: i32,
    month: u32,
    entries: &[CalendarEntry],
    week_start: WeekStart,
    with_titles: bool,
) -> String {
    // Validate inputs.
    let first_of_month = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_else(|| {
        Local::now()
            .date_naive()
            .with_day(1)
            .unwrap_or(Local::now().date_naive())
    });
    let month_len = days_in_month(year, month);
    // Bucket entries by day-of-month.
    let mut by_day: std::collections::HashMap<u32, Vec<&CalendarEntry>> =
        std::collections::HashMap::new();
    for e in entries {
        if e.date.year() == year && e.date.month() == month {
            by_day.entry(e.date.day()).or_default().push(e);
        }
    }

    let mut out = String::with_capacity(512);
    // Title line.
    let month_name = month_name(month);
    out.push_str(&format!("    {} {}\n", month_name, year));

    // Weekday header.
    let header = match week_start {
        WeekStart::Sunday => "Su Mo Tu We Th Fr Sa\n",
        WeekStart::Monday => "Mo Tu We Th Fr Sa Su\n",
    };
    out.push_str(header);

    // Leading blanks.
    let first_col = week_start.column_for(first_of_month.weekday());
    let mut col = first_col;
    for _ in 0..first_col {
        out.push_str("   ");
    }

    for day in 1..=month_len {
        let count = by_day.get(&day).map(|v| v.len()).unwrap_or(0);
        let cell = if count > 0 {
            format!("{:>2}[{}]", day, min_digits(count))
        } else {
            format!("{:>2}  ", day)
        };
        out.push_str(&cell);
        col += 1;
        if col == 7 {
            out.push('\n');
            if with_titles {
                // Render up to 3 titles per row, aligned under each day column.
                for slot in 0..7 {
                    // Map the column slot back to the day number. When col hit
                    // 7 we just emitted a full row ending at `day`, so the row
                    // spans days [day-6 .. day]. Guard the subtraction to avoid
                    // underflow on the first row of the month.
                    let d = day.saturating_sub(6 - slot as u32);
                    if d >= 1 && d <= day {
                        if let Some(es) = by_day.get(&d) {
                            if let Some(first) = es.first() {
                                let title = first.title.as_deref().unwrap_or("");
                                let trunc = truncate_str(title, 12);
                                out.push_str(&format!("{:<14}", trunc));
                                continue;
                            }
                        }
                        // Day exists but has no matching entry: full-width blank.
                        out.push_str("              ");
                    } else {
                        // No day in this column (leading blank of first row
                        // when month starts mid-week). Use narrow padding that
                        // matches the date row's blank column width so the
                        // first title isn't massively shifted to the right.
                        out.push_str("   ");
                    }
                }
                out.push('\n');
            }
            col = 0;
        }
    }
    // Trailing padding to end-of-row.
    if col != 0 {
        for _ in col..7 {
            out.push_str("   ");
        }
        out.push('\n');
        if with_titles {
            // Render an aligned title row for the final (incomplete) week,
            // mirroring the full-row title rendering above. The trailing row
            // spans days [month_len - col + 1 ..= month_len] across columns
            // [0 .. col). Leading slots within this row always hold a day
            // because the date loop filled them contiguously from day 1.
            for c in 0..7 {
                if c < col {
                    let d = (month_len as usize - col + 1 + c) as u32;
                    if let Some(es) = by_day.get(&d) {
                        if let Some(first) = es.first() {
                            let title = first.title.as_deref().unwrap_or("");
                            let trunc = truncate_str(title, 12);
                            out.push_str(&format!("{:<14}", trunc));
                            continue;
                        }
                    }
                    // Existing day with no entry: full-width blank.
                    out.push_str("              ");
                } else {
                    // Trailing padding column: narrow blank to match the date row.
                    out.push_str("   ");
                }
            }
            out.push('\n');
        }
    }

    // Summary: total entries this month + a per-day list.
    let total: usize = by_day.values().map(|v| v.len()).sum();
    if total > 0 {
        out.push_str(&format!("\n{} note(s) in {} {}\n", total, month_name, year));
        let mut days: Vec<u32> = by_day.keys().copied().collect();
        days.sort_unstable();
        for d in days {
            let es = by_day.get(&d).unwrap();
            out.push_str(&format!("  {:>2}: ", d));
            for (i, e) in es.iter().enumerate() {
                if i > 0 {
                    out.push_str("; ");
                }
                match &e.title {
                    Some(t) => out.push_str(&format!("{} ({})", t, e.note_path)),
                    None => out.push_str(&e.note_path),
                }
            }
            out.push('\n');
        }
    } else {
        out.push_str(&format!("\nNo notes in {} {}.\n", month_name, year));
    }

    out
}

/// Return the number of days in a given (year, month). Month must be 1..=12.
fn days_in_month(year: i32, month: u32) -> u32 {
    // chrono: next month's first day minus one day.
    let (y, m) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next_first = NaiveDate::from_ymd_opt(y, m, 1).unwrap_or_else(|| {
        NaiveDate::from_ymd_opt(year, month, 28)
            .unwrap_or(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
    });
    let this_first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(next_first);
    (next_first - this_first).num_days() as u32
}

/// Render a single-digit count compactly. Counts ≥ 10 are clamped to "+".
fn min_digits(n: usize) -> String {
    if n >= 10 {
        "+".to_string()
    } else {
        n.to_string()
    }
}

/// Truncate a string to at most `max_chars` chars (char boundary safe),
/// appending "…" if truncated.
fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// English month name (1..=12 → January..December).
fn month_name(m: u32) -> &'static str {
    match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault_query::{QValue, Record};

    fn rec(path: &str, props: &[(&str, QValue)]) -> Record {
        let mut r = Record::new(path);
        for (k, v) in props {
            r = r.with_prop(*k, v.clone());
        }
        r
    }

    #[test]
    fn extract_date_from_qvalue_date() {
        let r = rec(
            "a.md",
            &[(
                "date",
                QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()),
            )],
        );
        let entries = entries_from_records(&[r], DEFAULT_DATE_FIELDS, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].date,
            NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()
        );
    }

    #[test]
    fn extract_date_from_iso_text_field() {
        // Frontmatter often has `date: 2026-07-20` parsed as text.
        let r = rec("a.md", &[("date", QValue::Text("2026-07-20".into()))]);
        let entries = entries_from_records(&[r], DEFAULT_DATE_FIELDS, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].date,
            NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()
        );
    }

    #[test]
    fn extract_date_from_datetime_text() {
        let r = rec(
            "a.md",
            &[("created", QValue::Text("2026-07-19T08:30:00Z".into()))],
        );
        let entries = entries_from_records(&[r], DEFAULT_DATE_FIELDS, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].date,
            NaiveDate::from_ymd_opt(2026, 7, 19).unwrap()
        );
    }

    #[test]
    fn extract_date_slash_format() {
        let r = rec("a.md", &[("published", QValue::Text("2026/07/18".into()))]);
        let entries = entries_from_records(&[r], DEFAULT_DATE_FIELDS, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].date,
            NaiveDate::from_ymd_opt(2026, 7, 18).unwrap()
        );
    }

    #[test]
    fn extract_date_priority_date_before_created() {
        // `date` field wins over `created` even if both present.
        let r = rec(
            "a.md",
            &[
                ("date", QValue::Text("2026-07-20".into())),
                ("created", QValue::Text("2026-07-01".into())),
            ],
        );
        let entries = entries_from_records(&[r], DEFAULT_DATE_FIELDS, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].date,
            NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()
        );
    }

    #[test]
    fn skip_records_with_no_date() {
        let r1 = rec("a.md", &[("date", QValue::Text("2026-07-20".into()))]);
        let r2 = rec("b.md", &[("title", QValue::Text("No date".into()))]);
        let r3 = rec("c.md", &[("date", QValue::Text("not-a-date".into()))]);
        let entries = entries_from_records(&[r1, r2, r3], DEFAULT_DATE_FIELDS, None);
        // Only r1 has a valid date.
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].note_path, "a.md");
    }

    #[test]
    fn title_extracted_when_field_present() {
        let r = rec(
            "a.md",
            &[
                (
                    "date",
                    QValue::Date(NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()),
                ),
                ("title", QValue::Text("Design Doc".into())),
            ],
        );
        let entries = entries_from_records(&[r], DEFAULT_DATE_FIELDS, Some("title"));
        assert_eq!(entries[0].title.as_deref(), Some("Design Doc"));
    }

    #[test]
    fn render_empty_month_has_header_and_no_notes_message() {
        let out = render_month_grid(2026, 7, &[], WeekStart::Sunday, false);
        assert!(out.contains("July 2026"), "missing title: {out}");
        assert!(out.contains("Su Mo Tu We Th Fr Sa"));
        assert!(out.contains("No notes in July 2026"), "got:\n{out}");
    }

    #[test]
    fn render_month_marks_days_with_entries() {
        let e1 = CalendarEntry {
            note_path: "a.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            title: Some("First".into()),
        };
        let e2 = CalendarEntry {
            note_path: "b.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
            title: None,
        };
        let e3 = CalendarEntry {
            note_path: "c.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
            title: Some("Third".into()),
        };
        let out = render_month_grid(2026, 7, &[e1, e2, e3], WeekStart::Sunday, false);
        // Day 1 has 1 entry → "1[1]"
        assert!(out.contains("1[1]"), "got:\n{out}");
        // Day 15 has 2 entries → "15[2]"
        assert!(out.contains("15[2]"), "got:\n{out}");
        // Summary section: total = 3 notes.
        assert!(out.contains("3 note(s) in July 2026"), "got:\n{out}");
        // Day 15 listing should mention both b.md and c.md.
        assert!(out.contains("b.md"), "got:\n{out}");
        assert!(out.contains("c.md"), "got:\n{out}");
    }

    #[test]
    fn render_excludes_entries_outside_month() {
        let e_in = CalendarEntry {
            note_path: "in.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 7, 10).unwrap(),
            title: None,
        };
        let e_out_prev = CalendarEntry {
            note_path: "prev.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
            title: None,
        };
        let e_out_next = CalendarEntry {
            note_path: "next.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
            title: None,
        };
        let out = render_month_grid(
            2026,
            7,
            &[e_in, e_out_prev, e_out_next],
            WeekStart::Sunday,
            false,
        );
        assert!(out.contains("in.md"), "got:\n{out}");
        assert!(!out.contains("prev.md"), "prev month leaked: {out}");
        assert!(!out.contains("next.md"), "next month leaked: {out}");
    }

    #[test]
    fn render_monday_first_header() {
        let out = render_month_grid(2026, 7, &[], WeekStart::Monday, false);
        assert!(out.contains("Mo Tu We Th Fr Sa Su"), "got: {out}");
        assert!(!out.contains("Su Mo Tu We Th Fr Sa\n"));
    }

    #[test]
    fn render_with_titles_emits_titles_below_grid() {
        let e = CalendarEntry {
            note_path: "a.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            title: Some("My Design Doc".into()),
        };
        let out = render_month_grid(2026, 7, &[e], WeekStart::Sunday, true);
        // Title appears somewhere in the rendered output.
        assert!(
            out.contains("My Design Doc") || out.contains("My Design D…"),
            "got:\n{out}"
        );
    }

    #[test]
    fn render_with_titles_first_week_midweek_start() {
        // Regression test for issue #3187: when a month starts mid-week
        // (first_col > 0), the title for day 1 in the first row must
        // appear under the correct column, not massively shifted right
        // by empty 14-char slots for the leading blank columns.
        //
        // July 2026 starts on Wednesday (Sunday-first → first_col = 3).
        // The first row has leading blanks for Sun/Mon/Tue, then day 1
        // (Wed) through day 4 (Sat). The title for day 1 should align
        // near the start of the title line, not at position 40+.
        let e = CalendarEntry {
            note_path: "a.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            title: Some("TITLEABC".into()),
        };
        let out = render_month_grid(2026, 7, &[e], WeekStart::Sunday, true);
        // The title line is 2 lines below the header (line 0 = title,
        // line 1 = weekday headers, line 2 = first date row,
        // line 3 = first title row).
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.len() > 3, "not enough lines:\n{out}");
        let title_line = lines[3];
        let pos = title_line.find("TITLEABC");
        assert!(pos.is_some(), "TITLEABC not found in title line:\n{out}");
        let pos = pos.unwrap();
        // Before fix: pos would be 42 (3 blank slots × 14 chars each).
        // After fix: pos should be ≤ 12 (3 blank slots × 3 chars + 3
        // for the narrow title indent, or less if no indent needed).
        // We assert < 20 to leave margin for future width tweaks.
        assert!(
            pos < 20,
            "TITLEABC at position {}, expected < 20 (first_col=3 should use narrow indent instead of 14-char slots):\n{}",
            pos,
            out
        );
    }

    #[test]
    fn render_large_count_uses_plus() {
        // 10+ entries on day 1 → "1[+]" not a multi-digit count.
        let entries: Vec<CalendarEntry> = (0..12)
            .map(|i| CalendarEntry {
                note_path: format!("n{i}.md"),
                date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
                title: None,
            })
            .collect();
        let out = render_month_grid(2026, 7, &entries, WeekStart::Sunday, false);
        assert!(out.contains("1[+]"), "expected compact count, got:\n{out}");
    }

    #[test]
    fn days_in_month_handles_february_leap_year() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 1), 31);
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(days_in_month(2026, 12), 31);
    }

    #[test]
    fn render_with_titles_last_incomplete_week_has_titles() {
        // Regression test for issue #3194: the trailing (incomplete) week
        // of a month that does NOT end on a Saturday/Sunday row was missing
        // its title line entirely — only full rows [col == 7] rendered
        // titles. July 2026 ends on Friday (Sunday-first), so the last row
        // is incomplete and must still receive an aligned title line.
        let e1 = CalendarEntry {
            note_path: "d30.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
            title: Some("LastWeekA".into()),
        };
        let e2 = CalendarEntry {
            note_path: "d31.md".into(),
            date: NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
            title: Some("LastWeekB".into()),
        };
        let out = render_month_grid(2026, 7, &[e1, e2], WeekStart::Sunday, true);
        // Both titles must appear somewhere in the output.
        assert!(out.contains("LastWeekA"), "got:\n{out}");
        assert!(out.contains("LastWeekB"), "got:\n{out}");
        // The trailing title line is the one immediately after the final
        // date row. Count title lines (rows containing a title) must be at
        // least as many as the number of weeks that hold entries. We assert
        // the total number of lines containing either title is > 0 and that
        // the LAST date row is immediately followed by a title-bearing line.
        let lines: Vec<&str> = out.lines().collect();
        // The trailing title line must immediately follow the final date row.
        // The grid row for day 31 renders as "31[1]", which only ever appears
        // in the date grid (never in the summary section), so it uniquely
        // identifies the last date row of July 2026.
        let last_date_idx = lines
            .iter()
            .rposition(|l| l.contains("31[1]"))
            .expect("expected a date row containing day 31");
        let trailing_title = lines
            .get(last_date_idx + 1)
            .expect("expected a title line after the last date row");
        assert!(
            trailing_title.contains("LastWeekA") || trailing_title.contains("LastWeekB"),
            "trailing week has no title line: last date row followed by {trailing_title:?}\nfull:\n{out}"
        );
    }

    #[test]
    fn week_start_column_indices() {
        // Sunday-first: Sunday is column 0, Monday is 1, …, Saturday is 6.
        assert_eq!(WeekStart::Sunday.column_for(Weekday::Sun), 0);
        assert_eq!(WeekStart::Sunday.column_for(Weekday::Mon), 1);
        assert_eq!(WeekStart::Sunday.column_for(Weekday::Sat), 6);
        // Monday-first: Monday is column 0, …, Sunday is column 6.
        assert_eq!(WeekStart::Monday.column_for(Weekday::Mon), 0);
        assert_eq!(WeekStart::Monday.column_for(Weekday::Sun), 6);
        assert_eq!(WeekStart::Monday.column_for(Weekday::Sat), 5);
    }

    #[test]
    fn truncate_str_handles_unicode() {
        // "héllo" has 5 chars but "é" is 2 bytes.
        assert_eq!(truncate_str("héllo", 10), "héllo");
        let t = truncate_str("héllo world", 5);
        assert_eq!(t.chars().count(), 5);
        assert!(t.ends_with('…'));
    }
}
