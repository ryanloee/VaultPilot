//! Regression test for issue #2643/#2642: `extract_source_card` cannot
//! parse quoted datetime values from `build_source_card_yaml` round-trip.
//!
//! The `build_source_card_yaml` writes datetime lines like
//! `meeting_start: "2026-07-01T10:00:00Z"` (with explicit quotes), but
//! the `val()` closure in `extract_source_card` didn't strip the quotes
//! before passing to `chrono::DateTime::parse_from_rfc3339`.
//!
//! Fix: the `val()` closure now strips surrounding double quotes.

#![cfg(test)]

use crate::calendar::{build_source_card_yaml, extract_source_card, MeetingSourceCard};
use chrono::{TimeZone, Utc};

fn make_card() -> MeetingSourceCard {
    MeetingSourceCard {
        event_id: "test-1@vp".to_string(),
        title: "Sprint Planning".to_string(),
        organizer: Some("alice@example.com".to_string()),
        attendees: vec!["Bob".to_string(), "Carol".to_string()],
        calendar_source: "work".to_string(),
        start: Utc.with_ymd_and_hms(2026, 7, 1, 10, 0, 0).unwrap(),
        end: Utc.with_ymd_and_hms(2026, 7, 1, 11, 0, 0).unwrap(),
        location: Some("Room A".to_string()),
        meeting_url: Some("https://meet.example.com/sprint".to_string()),
    }
}

#[test]
fn regression_2643_build_source_card_yaml_includes_quoted_datetimes() {
    let card = make_card();
    let lines = build_source_card_yaml(&card);

    // The datetime lines should contain quoted RFC3339 values
    let start_line = lines
        .iter()
        .find(|l| l.starts_with("meeting_start:"))
        .unwrap();
    assert!(
        start_line.contains("\"2026-07-01T10:00:00+00:00\""),
        "expected quoted datetime in build_source_card_yaml output, got: {start_line}"
    );

    let end_line = lines
        .iter()
        .find(|l| l.starts_with("meeting_end:"))
        .unwrap();
    assert!(
        end_line.contains("\"2026-07-01T11:00:00+00:00\""),
        "expected quoted datetime in build_source_card_yaml output, got: {end_line}"
    );
}

#[test]
fn regression_2643_roundtrip_full_yaml() {
    let card = make_card();
    let yaml_lines = build_source_card_yaml(&card);

    // Reconstruct the YAML frontmatter block as extract_source_card expects it
    let mut body = String::from("---\n");
    for line in &yaml_lines {
        body.push_str(line);
        body.push('\n');
    }
    body.push_str("---\n\nSome note content here.\n");

    let parsed = extract_source_card(&body).expect("should parse round-tripped YAML");

    assert_eq!(parsed.event_id, "test-1@vp");
    assert_eq!(parsed.title, "Sprint Planning");
    assert_eq!(parsed.organizer, Some("alice@example.com".to_string()));
    assert_eq!(parsed.attendees, vec!["Bob", "Carol"]);
    assert_eq!(
        parsed.start,
        Utc.with_ymd_and_hms(2026, 7, 1, 10, 0, 0).unwrap()
    );
    assert_eq!(
        parsed.end,
        Utc.with_ymd_and_hms(2026, 7, 1, 11, 0, 0).unwrap()
    );
    assert_eq!(parsed.location, Some("Room A".to_string()));
    assert_eq!(
        parsed.meeting_url,
        Some("https://meet.example.com/sprint".to_string())
    );
}

#[test]
fn regression_2643_roundtrip_empty_attendees() {
    let mut card = make_card();
    card.attendees = vec![];
    let yaml_lines = build_source_card_yaml(&card);

    let mut body = String::from("---\n");
    for line in &yaml_lines {
        body.push_str(line);
        body.push('\n');
    }
    body.push_str("---\n\nEmpty attendees test.\n");

    let parsed = extract_source_card(&body).expect("should parse with empty attendees");
    assert!(parsed.attendees.is_empty(), "expected empty attendees");
    assert_eq!(
        parsed.start,
        Utc.with_ymd_and_hms(2026, 7, 1, 10, 0, 0).unwrap()
    );
}

#[test]
fn regression_2643_parse_unquoted_datetime() {
    // Verify that extract_source_card also accepts unquoted datetimes
    // (e.g., if someone manually edits the YAML and removes quotes)
    let body = "---\nmeeting_event_id: evt-1\nmeeting_title: Test\nmeeting_start: 2026-07-01T10:00:00Z\nmeeting_end: 2026-07-01T11:00:00Z\nmeeting_attendees: []\n---\n\nBody\n";

    let parsed = extract_source_card(body).expect("should parse unquoted datetime");
    assert_eq!(parsed.event_id, "evt-1");
    assert_eq!(
        parsed.start,
        Utc.with_ymd_and_hms(2026, 7, 1, 10, 0, 0).unwrap()
    );
    assert_eq!(
        parsed.end,
        Utc.with_ymd_and_hms(2026, 7, 1, 11, 0, 0).unwrap()
    );
}
