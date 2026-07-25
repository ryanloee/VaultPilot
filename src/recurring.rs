//! Recurring tasks / repeating to-dos (#3464).
//!
//! This module implements the backend for Obsidian-Tasks-style recurring
//! checkboxes.  A task line in a Markdown note may carry a recurrence rule
//! using the `🔁` emoji plus an optional due date (`📅`) and done date (`✅`).
//! When the user checks the box (`- [ ]` → `- [x]`), the agent / MCP layer
//! can call [`generate_next_instance`] to produce a fresh `- [ ]` line with
//! the due date advanced by one cycle.
//!
//! ## Supported syntax
//!
//! ```markdown
//! - [ ] 每周复盘 🔁 every week 📅 2026-07-25
//! - [x] 每周复盘 🔁 every week 📅 2026-07-18 ✅ 2026-07-18
//! ```
//!
//! The recurrence rule grammar mirrors the Obsidian Tasks plugin:
//!
//! | Expression | Meaning |
//! |------------|---------|
//! | `🔁 every day` / `every daily` | every 1 day |
//! | `🔁 every week` / `every weekly` | every 1 week |
//! | `🔁 every month` / `every monthly` | every 1 month |
//! | `🔁 every year` / `every yearly` | every 1 year |
//! | `🔁 every 3 days` | every 3 days |
//! | `🔁 every 2 weeks` | every 2 weeks |
//!
//! ## Advance semantics
//!
//! Two Logseq-compatible modes are supported via [`RecurringSemantics`]:
//!
//! * **Push** (`++` in Logseq, default in Obsidian Tasks): the next due date
//!   is computed from the *previous* due date.  This keeps a task on a fixed
//!   cadence regardless of when it was actually completed.
//! * **Plus** (`.+` in Logseq): the next due date is computed from the
//!   *completion* date.  Useful for "every day after I finish".
//!
//! When no explicit due date is present the rule falls back to the
//! completion date (Plus semantics), so a brand-new recurring task still
//! advances predictably.

use chrono::{Duration, Local, Months, NaiveDate};
use serde::{Deserialize, Serialize};

// ─── Emoji tokens ────────────────────────────────────────────────────

/// Recurrence-rule marker (Obsidian Tasks compatible).
const EMOJI_RECURRENCE: char = '🔁';
/// Due-date marker (Obsidian Tasks compatible).
const EMOJI_DUE: char = '📅';
/// Done/completed-date marker (Obsidian Tasks compatible).
const EMOJI_DONE: char = '✅';

// ─── Data types ──────────────────────────────────────────────────────

/// The unit of repetition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecurUnit {
    Day,
    Week,
    Month,
    Year,
}

impl std::fmt::Display for RecurUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RecurUnit::Day => "day",
            RecurUnit::Week => "week",
            RecurUnit::Month => "month",
            RecurUnit::Year => "year",
        };
        write!(f, "{s}")
    }
}

/// How the next due date is derived from the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecurringSemantics {
    /// Advance from the previous due date (fixed cadence).
    /// Maps to Logseq `++` / Obsidian Tasks default behaviour.
    #[default]
    Push,
    /// Advance from the completion date (rolling cadence).
    /// Maps to Logseq `.+` behaviour.
    Plus,
}

/// A parsed recurrence rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurringRule {
    /// Number of units per cycle (e.g. `3` in "every 3 weeks").
    pub interval: u32,
    /// The unit of repetition.
    pub unit: RecurUnit,
    /// How to advance the date.
    pub semantics: RecurringSemantics,
}

impl Default for RecurringRule {
    fn default() -> Self {
        Self {
            interval: 1,
            unit: RecurUnit::Week,
            semantics: RecurringSemantics::Push,
        }
    }
}

impl std::fmt::Display for RecurringRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let unit_word = match self.unit {
            RecurUnit::Day => "day",
            RecurUnit::Week => "week",
            RecurUnit::Month => "month",
            RecurUnit::Year => "year",
        };
        let plural = if self.interval == 1 { "" } else { "s" };
        write!(
            f,
            "{} every {} {}{}",
            EMOJI_RECURRENCE, self.interval, unit_word, plural
        )
    }
}

// ─── Parsing ─────────────────────────────────────────────────────────

/// Parse a recurrence rule from the `🔁 every …` portion of a task line.
///
/// Returns `None` when no recurrence marker is present or the grammar is
/// unrecognised.  Unit aliases (`daily`, `weekly`, `monthly`, `yearly`) are
/// accepted and normalised to `interval = 1`.
///
/// ```
/// use vaultpilot::recurring::{parse_recurring_rule, RecurUnit};
///
/// let r = parse_recurring_rule("- [ ] 喝水 🔁 every day").unwrap();
/// assert_eq!(r.interval, 1);
/// assert_eq!(r.unit, RecurUnit::Day);
///
/// let r = parse_recurring_rule("- [ ] 复盘 🔁 every 2 weeks").unwrap();
/// assert_eq!(r.interval, 2);
/// assert_eq!(r.unit, RecurUnit::Week);
/// ```
pub fn parse_recurring_rule(line: &str) -> Option<RecurringRule> {
    let recur_idx = line.find(EMOJI_RECURRENCE)?;
    let after_emoji = &line[recur_idx + EMOJI_RECURRENCE.len_utf8()..];
    // Trim leading spaces.
    let rest = after_emoji.trim_start();
    // Expect "every …"
    let rest = rest.strip_prefix("every")?.trim_start();
    if rest.is_empty() {
        return Some(RecurringRule::default());
    }
    // Take up to the next emoji or end-of-line.
    let segment = rest
        .find([EMOJI_DUE, EMOJI_DONE, '🔁'])
        .map(|i| &rest[..i])
        .unwrap_or(rest);
    let segment = segment.trim();

    // Try to parse "<interval> <unit>" or just "<unit-word>".
    let (interval_str, unit_word) = match segment.split_once(char::is_whitespace) {
        Some((a, b)) => (a, b),
        None => ("1", segment), // single token → interval defaults to 1
    };

    let interval: u32 = interval_str
        .parse()
        .ok()
        // If the first token wasn't a number, it was probably the unit
        // word alone (e.g. "every week") — retry with interval = 1.
        .or_else(|| {
            if unit_word.is_empty() {
                None
            } else {
                RecurUnit::from_word(interval_str).map(|_| 1_u32)
            }
        })?;

    // Resolve unit word.  When the first token was the unit word, use it.
    let resolved_word = if interval == 1 && RecurUnit::from_word(interval_str).is_some() {
        interval_str
    } else {
        unit_word
    };

    let unit = RecurUnit::from_word(resolved_word)?;
    Some(RecurringRule {
        interval: interval.max(1),
        unit,
        semantics: RecurringSemantics::Push,
    })
}

impl RecurUnit {
    /// Resolve a human-friendly unit word to a [`RecurUnit`].
    fn from_word(word: &str) -> Option<RecurUnit> {
        match word.to_ascii_lowercase().as_str() {
            "day" | "days" | "daily" => Some(RecurUnit::Day),
            "week" | "weeks" | "weekly" => Some(RecurUnit::Week),
            "month" | "months" | "monthly" => Some(RecurUnit::Month),
            "year" | "years" | "yearly" | "annually" => Some(RecurUnit::Year),
            _ => None,
        }
    }
}

/// Extract a `📅 YYYY-MM-DD` due date from a task line.
pub fn parse_due_date(line: &str) -> Option<NaiveDate> {
    parse_emoji_date(line, EMOJI_DUE)
}

/// Extract a `✅ YYYY-MM-DD` done date from a task line.
pub fn parse_done_date(line: &str) -> Option<NaiveDate> {
    parse_emoji_date(line, EMOJI_DONE)
}

fn parse_emoji_date(line: &str, emoji: char) -> Option<NaiveDate> {
    let idx = line.find(emoji)?;
    let after = &line[idx + emoji.len_utf8()..];
    let trimmed = after.trim_start();
    // Take the first whitespace-delimited token after the emoji.
    let token = trimmed.split_whitespace().next()?;
    NaiveDate::parse_from_str(token, "%Y-%m-%d").ok()
}

// ─── Date arithmetic ──────────────────────────────────────────────────

/// Compute the next due date for `rule`, given the previous due date
/// (optional) and the completion date.
///
/// * **Push** semantics use `prev_due` when available, falling back to
///   `completed` otherwise.
/// * **Plus** semantics always use `completed`.
pub fn calculate_next_due(
    rule: &RecurringRule,
    prev_due: Option<NaiveDate>,
    completed: NaiveDate,
) -> NaiveDate {
    let base = match rule.semantics {
        RecurringSemantics::Push => prev_due.unwrap_or(completed),
        RecurringSemantics::Plus => completed,
    };
    advance_date(base, rule.interval, rule.unit)
}

/// Add `n × interval` units to `date`.
///
/// For month/year additions chrono's `checked_add_months` clamps the day to
/// the last valid day of the target month (e.g. Jan 31 + 1 month → Feb 28),
/// matching Obsidian Tasks behaviour.  It only returns `None` on year-range
/// overflow (~year 262143), so we fall back to the original date in that
/// (practically impossible) case.
fn advance_date(date: NaiveDate, interval: u32, unit: RecurUnit) -> NaiveDate {
    let n = i64::from(interval);
    match unit {
        RecurUnit::Day => date + Duration::days(n),
        RecurUnit::Week => date + Duration::weeks(n),
        RecurUnit::Month => date
            .checked_add_months(Months::new(interval))
            .unwrap_or(date),
        RecurUnit::Year => date
            .checked_add_months(Months::new(interval * 12))
            .unwrap_or(date),
    }
}

// ─── Line generation ─────────────────────────────────────────────────

/// Options for generating the next recurring-task instance.
#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    /// Override the completion date (defaults to today in local time).
    pub completed_date: Option<NaiveDate>,
    /// Override the semantics parsed from the line.
    pub semantics: Option<RecurringSemantics>,
}

/// Given a completed task line (`- [x] … 🔁 every … 📅 … ✅ …`), produce the
/// next uncompleted instance (`- [ ] … 🔁 every … 📅 <next-due>`).
///
/// The completion marker (`✅`) and check state (`[x]` → `[ ]`) are
/// rewritten; all other text is preserved verbatim.
///
/// Returns `None` when the line is not a recurring task.
pub fn generate_next_instance(line: &str) -> Option<String> {
    generate_next_instance_with(line, GenerateOptions::default())
}

/// Like [`generate_next_instance`] but with explicit options (useful for
/// tests and scheduled back-fills).
pub fn generate_next_instance_with(line: &str, opts: GenerateOptions) -> Option<String> {
    let mut rule = parse_recurring_rule(line)?;
    if let Some(s) = opts.semantics {
        rule.semantics = s;
    }

    let prev_due = parse_due_date(line);
    let completed = opts
        .completed_date
        .or_else(|| parse_done_date(line))
        .unwrap_or_else(|| Local::now().date_naive());

    let next_due = calculate_next_due(&rule, prev_due, completed);
    let next_due_str = next_due.format("%Y-%m-%d").to_string();

    // Rewrite the line.
    let mut result = line.to_string();

    // 1. `[x]` → `[ ]`
    result = result.replacen("- [x]", "- [ ]", 1);
    result = result.replacen("- [X]", "- [ ]", 1);

    // 2. Remove the ✅ done-date marker (it belongs to the completed instance).
    remove_emoji_segment(&mut result, EMOJI_DONE);

    // 3. Update or insert the 📅 due date.
    if result.contains(EMOJI_DUE) {
        replace_emoji_date(&mut result, EMOJI_DUE, &next_due_str);
    } else {
        // No due date existed — append 📅 at end of line (after recurrence).
        result.push_str(&format!(" {EMOJI_DUE} {next_due_str}"));
    }

    Some(result)
}

/// Remove the `<emoji> <token>` segment (trailing spaces trimmed) from `s`.
fn remove_emoji_segment(s: &mut String, emoji: char) {
    while let Some(idx) = s.find(emoji) {
        let after = idx + emoji.len_utf8();
        let rest = &s[after..];
        // Skip the single token following the emoji.
        let skip = rest
            .trim_start()
            .find(char::is_whitespace)
            .map(|w| rest.trim_start().len() - w)
            .unwrap_or(rest.trim_start().len());
        let end = after + rest.len() - rest.trim_start().len() + skip;
        // Also consume one trailing space for cleanliness.
        let end = if end < s.len() && s.as_bytes().get(end) == Some(&b' ') {
            end + 1
        } else {
            end
        };
        s.replace_range(idx..end.min(s.len()), "");
    }
}

/// Replace the date token following `emoji` in `s` with `new_date`.
fn replace_emoji_date(s: &mut String, emoji: char, new_date: &str) {
    if let Some(idx) = s.find(emoji) {
        let after = idx + emoji.len_utf8();
        let rest = &s[after..];
        let leading = rest.len() - rest.trim_start().len();
        let trimmed = &rest[leading..];
        let token_len = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        let replace_start = after + leading;
        let replace_end = replace_start + token_len;
        s.replace_range(replace_start..replace_end, new_date);
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Rule parsing ────────────────────────────────────────────────

    #[test]
    fn parse_every_day() {
        let r = parse_recurring_rule("- [ ] 喝水 🔁 every day").unwrap();
        assert_eq!(r.interval, 1);
        assert_eq!(r.unit, RecurUnit::Day);
    }

    #[test]
    fn parse_every_daily_alias() {
        let r = parse_recurring_rule("- [ ] 喝水 🔁 every daily").unwrap();
        assert_eq!(r.interval, 1);
        assert_eq!(r.unit, RecurUnit::Day);
    }

    #[test]
    fn parse_every_week() {
        let r = parse_recurring_rule("- [ ] 复盘 🔁 every week 📅 2026-07-25").unwrap();
        assert_eq!(r.interval, 1);
        assert_eq!(r.unit, RecurUnit::Week);
    }

    #[test]
    fn parse_every_3_days() {
        let r = parse_recurring_rule("- [ ] 跑步 🔁 every 3 days").unwrap();
        assert_eq!(r.interval, 3);
        assert_eq!(r.unit, RecurUnit::Day);
    }

    #[test]
    fn parse_every_2_weeks() {
        let r = parse_recurring_rule("- [x] 双周会 🔁 every 2 weeks 📅 2026-07-20 ✅ 2026-07-20")
            .unwrap();
        assert_eq!(r.interval, 2);
        assert_eq!(r.unit, RecurUnit::Week);
    }

    #[test]
    fn parse_every_month() {
        let r = parse_recurring_rule("- [ ] 月度账单 🔁 every month").unwrap();
        assert_eq!(r.interval, 1);
        assert_eq!(r.unit, RecurUnit::Month);
    }

    #[test]
    fn parse_every_year() {
        let r = parse_recurring_rule("- [ ] 生日 🔁 every year").unwrap();
        assert_eq!(r.interval, 1);
        assert_eq!(r.unit, RecurUnit::Year);
    }

    #[test]
    fn parse_every_yearly_alias() {
        let r = parse_recurring_rule("- [ ] 续费 🔁 every yearly").unwrap();
        assert_eq!(r.interval, 1);
        assert_eq!(r.unit, RecurUnit::Year);
    }

    #[test]
    fn parse_no_marker_returns_none() {
        assert!(parse_recurring_rule("- [ ] 普通任务 📅 2026-07-25").is_none());
        assert!(parse_recurring_rule("- [ ] 没有任何标记").is_none());
        assert!(parse_recurring_rule("").is_none());
    }

    #[test]
    fn parse_invalid_unit_returns_none() {
        assert!(parse_recurring_rule("- [ ] 错误 🔁 every fortnight").is_none());
        assert!(parse_recurring_rule("- [ ] 错误 🔁 every abc def").is_none());
    }

    // ── Date extraction ─────────────────────────────────────────────

    #[test]
    fn extract_due_date() {
        let d = parse_due_date("- [ ] 任务 🔁 every week 📅 2026-07-25").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 7, 25).unwrap());
    }

    #[test]
    fn extract_done_date() {
        let d = parse_done_date("- [x] 任务 🔁 every week 📅 2026-07-18 ✅ 2026-07-18").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 7, 18).unwrap());
    }

    #[test]
    fn no_date_returns_none() {
        assert!(parse_due_date("- [ ] 任务 🔁 every week").is_none());
        assert!(parse_done_date("- [x] 任务").is_none());
    }

    // ── Date arithmetic ─────────────────────────────────────────────

    #[test]
    fn advance_one_week() {
        let rule = RecurringRule {
            interval: 1,
            unit: RecurUnit::Week,
            semantics: RecurringSemantics::Push,
        };
        let due = NaiveDate::from_ymd_opt(2026, 7, 25).unwrap();
        let next = calculate_next_due(&rule, Some(due), due);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
    }

    #[test]
    fn advance_three_days() {
        let rule = RecurringRule {
            interval: 3,
            unit: RecurUnit::Day,
            semantics: RecurringSemantics::Push,
        };
        let due = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let next = calculate_next_due(&rule, Some(due), due);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 1, 13).unwrap());
    }

    #[test]
    fn advance_one_month() {
        let rule = RecurringRule {
            interval: 1,
            unit: RecurUnit::Month,
            semantics: RecurringSemantics::Push,
        };
        let due = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let next = calculate_next_due(&rule, Some(due), due);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 2, 15).unwrap());
    }

    #[test]
    fn advance_one_year() {
        let rule = RecurringRule {
            interval: 1,
            unit: RecurUnit::Year,
            semantics: RecurringSemantics::Push,
        };
        let due = NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();
        let next = calculate_next_due(&rule, Some(due), due);
        assert_eq!(next, NaiveDate::from_ymd_opt(2027, 7, 26).unwrap());
    }

    #[test]
    fn advance_month_clamps_jan31_to_feb28() {
        let rule = RecurringRule {
            interval: 1,
            unit: RecurUnit::Month,
            semantics: RecurringSemantics::Push,
        };
        let due = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let next = calculate_next_due(&rule, Some(due), due);
        // Jan 31 + 1 month → Feb 28 (2026 is not a leap year)
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());
    }

    // ── Push vs Plus semantics ───────────────────────────────────────

    #[test]
    fn push_uses_prev_due() {
        let rule = RecurringRule {
            interval: 1,
            unit: RecurUnit::Week,
            semantics: RecurringSemantics::Push,
        };
        let prev = NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        let completed = NaiveDate::from_ymd_opt(2026, 7, 23).unwrap(); // late completion
        let next = calculate_next_due(&rule, Some(prev), completed);
        // Push: 07-18 + 7 = 07-25 (ignores late completion)
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 7, 25).unwrap());
    }

    #[test]
    fn plus_uses_completion() {
        let rule = RecurringRule {
            interval: 1,
            unit: RecurUnit::Week,
            semantics: RecurringSemantics::Plus,
        };
        let prev = NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        let completed = NaiveDate::from_ymd_opt(2026, 7, 23).unwrap();
        let next = calculate_next_due(&rule, Some(prev), completed);
        // Plus: 07-23 + 7 = 07-30 (rolls forward from completion)
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 7, 30).unwrap());
    }

    #[test]
    fn push_falls_back_to_completed_when_no_due() {
        let rule = RecurringRule {
            interval: 1,
            unit: RecurUnit::Day,
            semantics: RecurringSemantics::Push,
        };
        let completed = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let next = calculate_next_due(&rule, None, completed);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 7, 21).unwrap());
    }

    // ── generate_next_instance ───────────────────────────────────────

    #[test]
    fn generate_weekly_with_explicit_date() {
        let line = "- [x] 每周复盘 🔁 every week 📅 2026-07-18 ✅ 2026-07-18";
        let opts = GenerateOptions {
            completed_date: Some(NaiveDate::from_ymd_opt(2026, 7, 18).unwrap()),
            semantics: None,
        };
        let next = generate_next_instance_with(line, opts).unwrap();
        assert!(
            next.starts_with("- [ ] 每周复盘 🔁 every week 📅 2026-07-25"),
            "got: {next}"
        );
        // ✅ marker must be removed from the new instance
        assert!(
            !next.contains('✅'),
            "done marker should be removed: {next}"
        );
    }

    #[test]
    fn generate_daily_no_existing_due() {
        let line = "- [x] 喝水 🔁 every day";
        let opts = GenerateOptions {
            completed_date: Some(NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()),
            semantics: None,
        };
        let next = generate_next_instance_with(line, opts).unwrap();
        assert!(
            next.starts_with("- [ ] 喝水 🔁 every day 📅 2026-07-21"),
            "got: {next}"
        );
    }

    #[test]
    fn generate_preserves_leading_text_and_tags() {
        let line = "- [x] 周报 #work [[ProjectA]] 🔁 every week 📅 2026-07-18 ✅ 2026-07-20";
        let opts = GenerateOptions {
            completed_date: Some(NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()),
            semantics: Some(RecurringSemantics::Push),
        };
        let next = generate_next_instance_with(line, opts).unwrap();
        assert!(next.contains("#work"), "tags preserved: {next}");
        assert!(next.contains("[[ProjectA]]"), "links preserved: {next}");
        assert!(next.contains("📅 2026-07-25"), "due advanced: {next}");
        assert!(!next.contains('✅'), "done removed: {next}");
    }

    #[test]
    fn generate_non_recurring_returns_none() {
        assert!(generate_next_instance("- [x] 普通任务 ✅ 2026-07-20").is_none());
        assert!(generate_next_instance("- [ ] 普通任务").is_none());
    }

    #[test]
    fn generate_uppercase_x_checkbox() {
        let line = "- [X] 任务 🔁 every day";
        let opts = GenerateOptions {
            completed_date: Some(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
            semantics: None,
        };
        let next = generate_next_instance_with(line, opts).unwrap();
        assert!(next.starts_with("- [ ] 任务 🔁 every day"), "got: {next}");
    }

    // ── Display round-trip ───────────────────────────────────────────

    #[test]
    fn display_format_singular() {
        let rule = RecurringRule {
            interval: 1,
            unit: RecurUnit::Week,
            semantics: RecurringSemantics::Push,
        };
        assert_eq!(rule.to_string(), "🔁 every 1 week");
    }

    #[test]
    fn display_format_plural() {
        let rule = RecurringRule {
            interval: 3,
            unit: RecurUnit::Day,
            semantics: RecurringSemantics::Push,
        };
        assert_eq!(rule.to_string(), "🔁 every 3 days");
    }

    #[test]
    fn serde_round_trip() {
        let rule = RecurringRule {
            interval: 2,
            unit: RecurUnit::Month,
            semantics: RecurringSemantics::Plus,
        };
        let json = serde_json::to_string(&rule).unwrap();
        let back: RecurringRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, back);
    }

    // ── Edge cases ───────────────────────────────────────────────────

    #[test]
    fn month_clamp_handles_february() {
        // chrono checked_add_months clamps Jan 31 → Feb 28/29
        let rule = RecurringRule {
            interval: 1,
            unit: RecurUnit::Month,
            semantics: RecurringSemantics::Push,
        };
        let jan31 = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let next = calculate_next_due(&rule, Some(jan31), jan31);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());

        let jan31_2028 = NaiveDate::from_ymd_opt(2028, 1, 31).unwrap();
        let next_leap = calculate_next_due(&rule, Some(jan31_2028), jan31_2028);
        assert_eq!(next_leap, NaiveDate::from_ymd_opt(2028, 2, 29).unwrap());
    }

    #[test]
    fn parse_every_with_no_unit_defaults() {
        // "🔁 every" with nothing after — should default to weekly
        let r = parse_recurring_rule("- [ ] 任务 🔁 every").unwrap();
        assert_eq!(r.interval, 1);
        assert_eq!(r.unit, RecurUnit::Week);
    }
}
