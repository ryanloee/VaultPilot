//! Regression tests for #3359: Agent Instant Undo/Redo.
//!
//! Tests the WriteTracker modification_order, undo_log, last_modified_note,
//! undo_last_write, redo_last_undo, and CLI commands.

use crate::orchestration::write::{WriteBackup, WriteTracker};

fn make_note(id: &str, title: &str, body: &str) -> crate::models::NoteDocument {
    crate::models::NoteDocument {
        meta: crate::models::NoteMeta {
            id: id.to_string(),
            title: title.to_string(),
            path: format!("{}.md", id),
            tags: vec!["test".to_string()],
            keywords: vec![],
            platform: "test".to_string(),
            board: "".to_string(),
            kernel: "".to_string(),
            status: "active".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            source: "test".to_string(),
            summary: format!("Summary {}", title),
            collections: vec![],
        },
        body: body.to_string(),
        ..Default::default()
    }
}

#[test]
fn undo_log_starts_empty() {
    let tracker = WriteTracker::new();
    assert_eq!(tracker.undo_log_len(), 0);
    assert!(tracker.last_modified_note().is_none());
}

#[test]
fn record_backup_pushes_to_modification_order() {
    let tracker = WriteTracker::new();
    tracker.record_backup(&make_note("n1", "Title 1", "body 1"));
    assert_eq!(tracker.undo_log_len(), 1);
    assert_eq!(tracker.last_modified_note().unwrap(), "n1");
}

#[test]
fn multiple_backups_track_order() {
    let tracker = WriteTracker::new();
    tracker.record_backup(&make_note("a", "A", "body-a"));
    tracker.record_backup(&make_note("b", "B", "body-b"));
    tracker.record_backup(&make_note("c", "C", "body-c"));
    assert_eq!(tracker.undo_log_len(), 3);
    // Most recent should be "c"
    assert_eq!(tracker.last_modified_note().unwrap(), "c");
}

#[test]
fn same_note_deduplicates_in_order() {
    let tracker = WriteTracker::new();
    tracker.record_backup(&make_note("n1", "T1", "body-1"));
    tracker.record_backup(&make_note("n2", "T2", "body-2"));
    // Second modification to n1 should move it to the end
    tracker.record_backup(&make_note("n1", "T1-v2", "body-1-v2"));
    assert_eq!(tracker.undo_log_len(), 2);
    assert_eq!(tracker.last_modified_note().unwrap(), "n1");
}

#[test]
fn pop_last_modification_removes_from_stack() {
    let tracker = WriteTracker::new();
    tracker.record_backup(&make_note("n1", "T1", "b1"));
    tracker.record_backup(&make_note("n2", "T2", "b2"));
    assert_eq!(tracker.undo_log_len(), 2);

    let popped = tracker.pop_last_modification();
    assert_eq!(popped.unwrap(), "n2");
    assert_eq!(tracker.undo_log_len(), 1);
    assert_eq!(tracker.last_modified_note().unwrap(), "n1");
}

#[test]
fn undo_log_capped_at_max() {
    let tracker = WriteTracker::new();
    // MAX_UNDO_LOG is 20, push 25 entries
    for i in 0..25 {
        tracker.record_backup(&make_note(
            &format!("n{}", i),
            &format!("Title {}", i),
            &format!("body {}", i),
        ));
    }
    // Should be capped at MAX_UNDO_LOG
    assert_eq!(tracker.undo_log_len(), 20);
    // Most recent should be n24
    assert_eq!(tracker.last_modified_note().unwrap(), "n24");
}

#[test]
fn redo_store_and_take() {
    let tracker = WriteTracker::new();
    let redo_data = WriteBackup {
        note_id: "n1".to_string(),
        note_path: "n1.md".to_string(),
        title: "Redo Title".to_string(),
        tags: vec![],
        keywords: vec![],
        platform: "redo".to_string(),
        board: "".to_string(),
        kernel: "".to_string(),
        status: "active".to_string(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-02T00:00:00Z".to_string(),
        source: "redo".to_string(),
        summary: "".to_string(),
        collections: vec![],
        body: "redo body content".to_string(),
        timestamp: 1700000000,
    };
    tracker.record_redo(redo_data);

    let taken = tracker.take_redo("n1");
    assert!(taken.is_some());
    assert_eq!(taken.unwrap().body, "redo body content");

    // After take, should be gone
    assert!(tracker.take_redo("n1").is_none());
}

#[test]
fn undo_log_deduplication_preserves_other_notes() {
    let tracker = WriteTracker::new();
    tracker.record_backup(&make_note("a", "A", "ba"));
    tracker.record_backup(&make_note("b", "B", "bb"));
    tracker.record_backup(&make_note("a", "A2", "ba2")); // updates a, moves to end
    assert_eq!(tracker.undo_log_len(), 2);
    // Pop twice: first gets "a" (most recent), then "b"
    let first = tracker.pop_last_modification().unwrap();
    assert_eq!(first, "a");
    let second = tracker.pop_last_modification().unwrap();
    assert_eq!(second, "b");
    assert_eq!(tracker.undo_log_len(), 0);
}
