//! Regression test for issue #3708: Vault Cleanup Suggestions — unified report
//! of orphan attachments, orphan notes, empty notes, and stale notes.
//!
//! Bug:        No unified cleanup view existed. Users had no way to discover
//!             notes or attachments that could be safely removed to tidy up
//!             their vault, unlike Anytype 0.56's "Cleanup Suggestions" panel.
//! Root cause: Missing module — `health.rs` covered orphan notes and
//!             `attachments.rs` covered orphan file scanning, but nothing
//!             combined them or added empty/stale note detection.
//! Fix:        Added `src/cleanup.rs` with `CleanupReport`, `StaleNote`, and
//!             `generate_cleanup_report()`, plus a `vp cleanup` CLI command.
//!
//! These tests verify the cleanup report generation with real database state,
//! complementing the unit tests in `cleanup.rs` (which cover timestamp parsing
//! and serialization without a DB).

#[cfg(test)]
mod tests {
    use crate::cleanup::{generate_cleanup_report, CleanupReport, DEFAULT_STALE_DAYS};
    use crate::storage::initialize_storage_with_context;
    use crate::storage::pool::StorageContext;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(label: &str) -> (StorageContext, std::path::PathBuf) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-reg3708-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        // Initialize the database schema.
        initialize_storage_with_context(&ctx).expect("init storage");
        let vault_dir = temp.join("vault");
        fs::create_dir_all(&vault_dir).expect("vault dir");
        (ctx, vault_dir)
    }

    /// Insert a note into the test database.
    fn insert_note(
        ctx: &StorageContext,
        id: &str,
        title: &str,
        body: &str,
        tags: &[&str],
        updated_at: &str,
    ) {
        let conn = ctx.get_connection().expect("connection");
        let tags_json = serde_json::to_string(tags).expect("tags json");
        conn.execute(
            "INSERT INTO notes (id, title, tags, keywords, platform, board, kernel, \
             status, created_at, updated_at, source, path, summary, body_hash) \
             VALUES (?1, ?2, ?3, '[]', '', '', '', '', ?4, ?5, 'manual', ?6, '', '')",
            rusqlite::params![
                id,
                title,
                tags_json,
                updated_at,
                updated_at,
                format!("notes/{}.md", id),
            ],
        )
        .expect("insert note");
        // Insert into FTS for body content.
        conn.execute(
            "INSERT INTO note_fts (note_id, title, keywords, body) VALUES (?1, ?2, '[]', ?3)",
            rusqlite::params![id, title, body],
        )
        .expect("insert fts");
    }

    /// The cleanup report should run on an empty vault without errors.
    #[test]
    fn regression_3708_empty_vault() {
        let (ctx, _vault_dir) = temp_context("empty");
        let report = generate_cleanup_report(&ctx, DEFAULT_STALE_DAYS).expect("report");
        assert_eq!(report.total_notes, 0);
        assert_eq!(report.total_items, 0);
        assert!(report.orphan_attachments.is_empty());
        assert!(report.orphan_notes.is_empty());
        assert!(report.empty_notes.is_empty());
        assert!(report.stale_notes.is_empty());
    }

    /// Orphan notes (no tags, no links) should be detected.
    #[test]
    fn regression_3708_detects_orphan_notes() {
        let (ctx, _vault_dir) = temp_context("orphan");
        // Orphan note: no tags, no links
        insert_note(
            &ctx,
            "orphan-1",
            "Lonely Note",
            "This is a note with enough content to not be empty but has no tags or links whatsoever.",
            &[],
            "2025-06-01T12:00:00Z",
        );
        // Non-orphan note: has tags
        insert_note(
            &ctx,
            "tagged-1",
            "Tagged Note",
            "This note has a tag so it's not an orphan.",
            &["important"],
            "2025-06-01T12:00:00Z",
        );

        let report = generate_cleanup_report(&ctx, DEFAULT_STALE_DAYS).expect("report");
        assert_eq!(report.total_notes, 2);
        assert_eq!(report.orphan_notes.len(), 1);
        assert_eq!(report.orphan_notes[0].id, "orphan-1");
    }

    /// Empty notes (body < threshold) should be detected.
    #[test]
    fn regression_3708_detects_empty_notes() {
        let (ctx, _vault_dir) = temp_context("empty");
        // Empty note: very short body
        insert_note(
            &ctx,
            "empty-1",
            "Empty Note",
            "Hi",
            &["tagged"],
            "2025-06-01T12:00:00Z",
        );
        // Non-empty note: long enough body
        insert_note(
            &ctx,
            "full-1",
            "Full Note",
            "This note has plenty of content that exceeds the minimum threshold for sure.",
            &["tagged"],
            "2025-06-01T12:00:00Z",
        );

        let report = generate_cleanup_report(&ctx, DEFAULT_STALE_DAYS).expect("report");
        assert_eq!(report.empty_notes.len(), 1);
        assert_eq!(report.empty_notes[0].id, "empty-1");
    }

    /// Stale notes (old updated_at) should be detected.
    #[test]
    fn regression_3708_detects_stale_notes() {
        let (ctx, _vault_dir) = temp_context("stale");
        // Stale note: updated 2 years ago
        insert_note(
            &ctx,
            "stale-1",
            "Ancient Note",
            "This is an old note that nobody has touched in a very long time.",
            &["old"],
            "2020-01-01T00:00:00Z",
        );
        // Recent note: updated recently
        let now_iso = {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                1970 + secs / 31_536_000,
                (secs % 31_536_000) / 2_628_000 + 1,
                (secs % 2_628_000) / 86_400 + 1,
                (secs % 86_400) / 3_600,
                (secs % 3_600) / 60,
                secs % 60
            )
        };
        insert_note(
            &ctx,
            "recent-1",
            "Fresh Note",
            "This note was just updated and is not stale at all.",
            &["new"],
            &now_iso,
        );

        let report = generate_cleanup_report(&ctx, DEFAULT_STALE_DAYS).expect("report");
        assert_eq!(report.stale_notes.len(), 1);
        assert_eq!(report.stale_notes[0].note.id, "stale-1");
        // Should be ~2000+ days old
        assert!(report.stale_notes[0].days_since_update > 1000);
    }

    /// The cleanup report should be serializable to JSON and back.
    #[test]
    fn regression_3708_report_json_roundtrip() {
        let (ctx, _vault_dir) = temp_context("json");
        let report = generate_cleanup_report(&ctx, DEFAULT_STALE_DAYS).expect("report");
        let json = serde_json::to_string(&report).expect("serialize");
        assert!(json.contains("\"totalNotes\""));
        let parsed: CleanupReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.total_notes, report.total_notes);
    }

    /// Custom stale_days threshold should change which notes are flagged.
    #[test]
    fn regression_3708_custom_stale_threshold() {
        let (ctx, _vault_dir) = temp_context("threshold");
        // Note updated 30 days ago
        let thirty_days_ago = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 30 * 86_400;
        let ts = format!(
            "{:04}-{:02}-{:02}T00:00:00Z",
            1970 + thirty_days_ago / 31_536_000,
            (thirty_days_ago % 31_536_000) / 2_628_000 + 1,
            (thirty_days_ago % 2_628_000) / 86_400 + 1
        );
        insert_note(
            &ctx,
            "medium-1",
            "Medium Note",
            "This note is 30 days old, somewhere between fresh and stale.",
            &["medium"],
            &ts,
        );

        // With 90-day threshold → not stale
        let report_90 = generate_cleanup_report(&ctx, 90).expect("report 90");
        assert_eq!(report_90.stale_notes.len(), 0);

        // With 7-day threshold → stale
        let report_7 = generate_cleanup_report(&ctx, 7).expect("report 7");
        assert_eq!(report_7.stale_notes.len(), 1);
    }
}
