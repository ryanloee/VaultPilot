//! Regression test for issue #1994: Calendar integration foundation.
//!
//! Exercises the ICS parser (RFC 5545 line folding, date formats, all-day
//! events, attendee extraction, CJK content), the meeting metadata
//! attachment (with/without existing frontmatter, key replacement), and
//! the SQLite cache round-trip (sync → cached agenda).

#![cfg(test)]

use crate::calendar::{
    self, attach_meeting_metadata, sync_events, today_agenda, today_agenda_cached, CalendarEvent,
    IcsCalendarProvider, LocalCalendarProvider,
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

/// Multi-event ICS fixture: timed UTC event with attendees + CJK,
/// an all-day event, and a line-folded summary.
const SAMPLE_ICS: &str = "BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//VaultPilot//Test//EN\r
BEGIN:VEVENT\r
UID:event-1@vaultpilot\r
DTSTART:20260701T100000Z\r
DTEND:20260701T110000Z\r
SUMMARY:Sprint Planning\r
LOCATION:Room A\r
DESCRIPTION:Weekly sprint sync\r
ATTENDEE;CN=张三:mailto:zhangsan@example.com\r
ATTENDEE;CN=John Doe:mailto:john@example.com\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:event-2@vaultpilot\r
DTSTART;VALUE=DATE:20260701\r
SUMMARY:All Day Event\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:event-3@vaultpilot\r
DTSTART:20260701T140000Z\r
DTEND:20260701T150000Z\r
SUMMARY:This is a very long summary that spans multiple lines because \r
 of RFC 5545 line folding rules and should be joined into one line\r
END:VEVENT\r
END:VCALENDAR\r
";

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

// ─── ICS Parser ───────────────────────────────────────────────────

#[test]
fn issue_1994_parse_multi_event_ics() {
    let provider = IcsCalendarProvider::from_content(SAMPLE_ICS, "test-cal");
    let events = provider.parse_events();
    assert_eq!(events.len(), 3, "should parse exactly 3 events");

    // Event 1 — timed UTC with attendees and CJK
    let e1 = &events[0];
    assert_eq!(e1.provider_event_id, "event-1@vaultpilot");
    assert_eq!(e1.title, "Sprint Planning");
    assert_eq!(e1.location.as_deref(), Some("Room A"));
    assert_eq!(e1.description.as_deref(), Some("Weekly sprint sync"));
    assert_eq!(e1.attendees, vec!["张三", "John Doe"]);
    assert!(!e1.all_day);
    let expected_start = NaiveDate::from_ymd_opt(2026, 7, 1)
        .unwrap()
        .and_hms_opt(10, 0, 0)
        .unwrap()
        .and_utc();
    assert_eq!(e1.start, expected_start);
    let expected_end = NaiveDate::from_ymd_opt(2026, 7, 1)
        .unwrap()
        .and_hms_opt(11, 0, 0)
        .unwrap()
        .and_utc();
    assert_eq!(e1.end, expected_end);

    // Event 2 — all-day
    let e2 = &events[1];
    assert_eq!(e2.title, "All Day Event");
    assert!(e2.all_day, "should be flagged as all-day");

    // Event 3 — folded summary line
    let e3 = &events[2];
    assert_eq!(
        e3.title,
        "This is a very long summary that spans multiple lines because of RFC 5545 line folding rules and should be joined into one line"
    );
}

#[test]
fn issue_1994_parse_rfc3339_timezone() {
    let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:tz-1@vp\nDTSTART:2026-07-01T10:00:00+08:00\nSUMMARY:TZ Event\nEND:VEVENT\nEND:VCALENDAR\n";
    let provider = IcsCalendarProvider::from_content(ics, "tz-cal");
    let events = provider.parse_events();
    assert_eq!(events.len(), 1);
    // +08:00 offset → 02:00 UTC
    let expected = NaiveDate::from_ymd_opt(2026, 7, 1)
        .unwrap()
        .and_hms_opt(2, 0, 0)
        .unwrap()
        .and_utc();
    assert_eq!(events[0].start, expected);
}

// ─── Meeting Metadata ─────────────────────────────────────────────

#[test]
fn issue_1994_attach_metadata_no_frontmatter() {
    let event = make_event("mtg-1", "Sprint Planning");
    let note = "# My Note\nSome content here";
    let result = attach_meeting_metadata(note, &event);

    assert!(result.starts_with("---\n"), "should prepend frontmatter");
    assert!(result.contains("meeting_event_id: mtg-1"));
    assert!(result.contains("meeting_title: Sprint Planning"));
    assert!(result.contains("meeting_attendees: []"));
    // Body preserved
    assert!(result.contains("# My Note"));
    assert!(result.contains("Some content here"));
}

#[test]
fn issue_1994_attach_metadata_existing_frontmatter() {
    let event = make_event("mtg-2", "Review");
    let note = "---\ntitle: My Note\ntags: [work]\n---\n# Content\nBody text";
    let result = attach_meeting_metadata(note, &event);

    assert!(result.contains("title: My Note"), "existing key preserved");
    assert!(result.contains("tags: [work]"), "existing key preserved");
    assert!(result.contains("meeting_event_id: mtg-2"));
    assert!(result.contains("meeting_title: Review"));
    assert!(result.contains("# Content"), "body preserved");
    assert!(result.contains("Body text"), "body preserved");
}

#[test]
fn issue_1994_attach_metadata_cjk_preserved() {
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
fn issue_1994_attach_metadata_replaces_old_keys() {
    let event = make_event("mtg-new", "New Title");
    let note = "---\nmeeting_event_id: mtg-old\nmeeting_title: Old Title\ntitle: Keep\n---\nBody";
    let result = attach_meeting_metadata(note, &event);

    assert!(!result.contains("mtg-old"), "old meeting id removed");
    assert!(!result.contains("Old Title"), "old title removed");
    assert!(result.contains("meeting_event_id: mtg-new"));
    assert!(result.contains("meeting_title: New Title"));
    assert!(result.contains("title: Keep"), "non-meeting key preserved");
}

// ─── Cache round-trip ─────────────────────────────────────────────

#[test]
fn issue_1994_sync_and_cached_agenda() {
    let temp = std::env::temp_dir().join(format!(
        "vp-cal-regression-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&temp).expect("temp dir");
    let ctx = crate::storage::StorageContext::for_test(&temp);
    crate::storage::initialize_storage_with_context(&ctx).expect("init storage");

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

// ─── Async today_agenda with LocalCalendarProvider ─────────────────

#[tokio::test]
async fn issue_1994_today_agenda_filters_by_date() {
    let day = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
    let events = vec![
        CalendarEvent {
            id: "a".into(),
            provider_event_id: "a".into(),
            title: "Morning".into(),
            start: day.and_hms_opt(9, 0, 0).unwrap().and_utc(),
            end: day.and_hms_opt(10, 0, 0).unwrap().and_utc(),
            location: None,
            description: None,
            attendees: vec![],
            source: "test".into(),
            all_day: false,
        },
        CalendarEvent {
            id: "b".into(),
            provider_event_id: "b".into(),
            title: "Next Day".into(),
            start: NaiveDate::from_ymd_opt(2026, 7, 2)
                .unwrap()
                .and_hms_opt(9, 0, 0)
                .unwrap()
                .and_utc(),
            end: NaiveDate::from_ymd_opt(2026, 7, 2)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap()
                .and_utc(),
            location: None,
            description: None,
            attendees: vec![],
            source: "test".into(),
            all_day: false,
        },
    ];
    let provider = LocalCalendarProvider::new(events);
    let now = day.and_hms_opt(12, 0, 0).unwrap().and_utc();
    let agenda = today_agenda(&provider, now).await.expect("today agenda");
    assert_eq!(agenda.len(), 1);
    assert_eq!(agenda[0].title, "Morning");
}

// ─── Ensure calendar module compiles ──────────────────────────────

#[test]
fn issue_1994_module_compiles() {
    // Touch the module to ensure it links.
    let _ = std::any::type_name::<calendar::CalendarEvent>();
}
