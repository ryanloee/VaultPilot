/// Issue #1995: 实时上下文面板 — AI proactively surfaces notes related to the
/// text the user is currently editing (or a live meeting transcript), without
/// requiring a saved note. Phase 1: free-text related-notes ranking + a
/// debounced live session that only refreshes when text changed AND the
/// min-interval has elapsed.
///
/// Feature: Real-time Context Surface — proactive "relevant notes" panel (#1995).
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use chrono::Utc;

    use crate::context_surface::{surface_for_text, LiveContextConfig, LiveContextSession};
    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{
        find_related_notes_for_text_with_context, save_note_with_context, StorageContext,
    };

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-context-surface-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    fn make_note(id: &str, title: &str, tags: Vec<&str>, body: &str) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                id: id.to_string(),
                title: title.to_string(),
                tags: tags.into_iter().map(String::from).collect(),
                keywords: vec![],
                platform: String::new(),
                board: String::new(),
                kernel: String::new(),
                status: String::new(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                source: "test".to_string(),
                path: String::new(),
                summary: String::new(),
                collections: Vec::new(),
            },
            body: body.to_string(),
            search_snippet: None,
            search_score: None,
        }
    }

    #[test]
    fn regression_1995_surface_for_text_finds_related_without_saved_note() {
        let (temp, ctx) = setup_temp_context();

        let rust_note = make_note(
            "rust-ownership",
            "Rust ownership model",
            vec!["rust", "memory"],
            "Ownership is Rust's key feature for memory safety",
        );
        let web_note = make_note(
            "python-web",
            "Python web framework",
            vec!["python"],
            "Django is a popular web framework for web apps",
        );
        save_note_with_context(&ctx, rust_note).expect("save rust note");
        save_note_with_context(&ctx, web_note).expect("save web note");

        // Free-form text about Rust — no saved source note, just what the user
        // happens to be typing. The Rust note should surface, the Python note
        // should not outrank it.
        let results = find_related_notes_for_text_with_context(
            &ctx,
            "notes about Rust ownership and memory",
            5,
        )
        .expect("surface for text");
        assert!(!results.is_empty(), "should surface at least one note");

        let top = &results[0];
        assert_eq!(
            top.meta.id, "rust-ownership",
            "Rust ownership note should be the top surfacing"
        );
        assert!(
            !results.iter().any(|r| r.score < 0),
            "scores must be non-negative"
        );

        // One-shot helper must agree with the storage-level function.
        let helper = surface_for_text(&ctx, "notes about Rust ownership and memory", 5).unwrap();
        assert_eq!(helper.len(), results.len());

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_1995_empty_or_stopword_text_returns_nothing() {
        let (temp, ctx) = setup_temp_context();
        let note = make_note("n1", "Some note", vec!["tag"], "body content here");
        save_note_with_context(&ctx, note).unwrap();

        // Pure stopword / short text yields an empty query → no surfacing, no panic.
        let r = find_related_notes_for_text_with_context(&ctx, "the and for with", 5).unwrap();
        assert!(r.is_empty(), "stopword-only text should not surface notes");

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_1995_hashtag_in_text_boosts_matching_tag() {
        let (temp, ctx) = setup_temp_context();
        let tagged = make_note("tagged-note", "Deep note", vec!["meeting"], "deep content");
        let other = make_note("other-note", "Unrelated", vec!["cooking"], "recipe");
        save_note_with_context(&ctx, tagged).unwrap();
        save_note_with_context(&ctx, other).unwrap();

        // Text containing #meeting should surface the tagged note first thanks to
        // the tag-overlap bonus.
        let results =
            find_related_notes_for_text_with_context(&ctx, "standup today #meeting notes", 5)
                .unwrap();
        assert!(
            results.iter().any(|r| r.meta.id == "tagged-note"),
            "should surface the #meeting-tagged note"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_1995_live_session_debounces_and_refreshes_on_change() {
        let (temp, ctx) = setup_temp_context();
        let note = make_note(
            "rust-note",
            "Rust ownership",
            vec!["rust"],
            "ownership borrowing lifetimes",
        );
        save_note_with_context(&ctx, note).unwrap();

        let cfg = LiveContextConfig {
            min_interval: Duration::from_millis(60),
            window_chars: 200,
            limit: 5,
        };
        let mut session = LiveContextSession::new(cfg);

        let t0 = Instant::now();
        // First consider → no prior refresh, should compute.
        let first = session
            .consider(&ctx, "typing about Rust ownership", t0)
            .unwrap();
        assert!(first.is_some(), "first call should refresh");

        // Immediately again with the same text → within interval & unchanged → None.
        let dup = session
            .consider(&ctx, "typing about Rust ownership", t0)
            .unwrap();
        assert!(
            dup.is_none(),
            "unchanged text within interval must not refresh"
        );

        // Same text, but after the interval elapsed → still unchanged query → None.
        let after = t0 + Duration::from_millis(80);
        let same = session
            .consider(&ctx, "typing about Rust ownership", after)
            .unwrap();
        assert!(
            same.is_none(),
            "unchanged query must not refresh even after interval"
        );

        // Different text after the interval → should refresh.
        let changed = session
            .consider(&ctx, "now writing about Python web framework", after)
            .unwrap();
        assert!(
            changed.is_some(),
            "changed text after interval should refresh"
        );

        // Different text but within the interval → debounced to None.
        let quick = session
            .consider(&ctx, "yet another different sentence", after)
            .unwrap();
        assert!(
            quick.is_none(),
            "changed text within interval must be debounced"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_1995_windowed_respects_char_boundary() {
        let cfg = LiveContextConfig {
            window_chars: 10,
            ..Default::default()
        };
        let session = LiveContextSession::new(cfg);

        // ASCII: window keeps last 10 bytes.
        assert_eq!(session.windowed("abcdefghijklm"), "defghijklm");
        // Multibyte: must not split a char — result must be valid UTF-8 and a suffix.
        let s = "你好世界你好世界你好世界"; // each char is 3 bytes
        let w = session.windowed(s);
        assert!(
            s.ends_with(w),
            "windowed text must be a suffix of the input"
        );
        // sanity: the window is valid utf-8 by construction (it's a &str slice).
        let _ = w.chars().count();

        // Short text shorter than the window is returned whole.
        assert_eq!(session.windowed("short"), "short");
    }

    #[test]
    fn regression_2368_windowed_uses_char_count_not_byte_count() {
        let cfg = LiveContextConfig {
            window_chars: 5,
            ..Default::default()
        };
        let session = LiveContextSession::new(cfg);

        // CJK text: each char is 3 bytes. Old code treated window_chars as
        // byte offset, so 5 chars = 15 bytes — only ~5 bytes (= ~1 char) would
        // be returned. With the fix, exactly 5 chars are returned.
        let s = "你好世界你好世界"; // 8 chars, 24 bytes
        let w = session.windowed(s);
        assert_eq!(
            w.chars().count(),
            5,
            "window should contain exactly window_chars characters"
        );
        assert_eq!(w, "界你好世界");
    }

    #[test]
    fn regression_1995_reset_clears_session_state() {
        let (temp, ctx) = setup_temp_context();
        let note = make_note("n", "Rust note", vec!["rust"], "ownership");
        save_note_with_context(&ctx, note).unwrap();

        let cfg = LiveContextConfig {
            min_interval: Duration::from_secs(60),
            ..Default::default()
        };
        let mut session = LiveContextSession::new(cfg);
        let now = Instant::now();
        let _ = session.consider(&ctx, "Rust ownership", now).unwrap();

        // After reset, the same text counts as "changed" (last_query cleared).
        session.reset();
        let again = session.consider(&ctx, "Rust ownership", now).unwrap();
        assert!(again.is_some(), "after reset the same text should refresh");

        let _ = fs::remove_dir_all(&temp);
    }
}
