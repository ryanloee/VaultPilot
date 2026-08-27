//! Regression tests for #3150: Reader Mode backend utilities.
//!
//! Tests reading-time estimation, web-clipper note detection, and source URL
//! extraction from the `reader` module.

#[cfg(test)]
mod tests {
    use crate::models::{NoteDocument, NoteMeta};
    use crate::reader::{
        estimate_reading_time, estimate_reading_time_for_note, extract_source_url,
        is_web_clipper_note, reading_info, ReadingEstimate,
    };

    #[test]
    fn reading_estimate_for_typical_article() {
        // ~600 words → 3 minutes at 200 WPM.
        let words: Vec<&str> = (0..600).map(|_| "lorem").collect();
        let body = words.join(" ");
        let est = estimate_reading_time(&body);
        assert_eq!(est.word_count, 600);
        assert_eq!(est.minutes, 3);
    }

    #[test]
    fn reading_estimate_strips_markdown_tables() {
        let body = "| Col1 | Col2 |\n|------|------|\n| A | B |\n\nSome text here.";
        let est = estimate_reading_time(body);
        // Table content is treated as plain text words.
        assert!(est.word_count > 0);
        assert!(est.minutes >= 1);
    }

    #[test]
    fn reading_estimate_for_note_convenience() {
        let note = NoteDocument {
            meta: NoteMeta::default(),
            body: "This is a note body with enough words.".to_string(),
            ..Default::default()
        };
        let est = estimate_reading_time_for_note(&note);
        assert!(est.word_count > 0);
    }

    #[test]
    fn web_clipper_detected_from_http_bridge_source_marker() {
        let note = NoteDocument {
            body: "> Source: https://blog.example.com/post\n\nArticle content.".to_string(),
            ..Default::default()
        };
        assert!(is_web_clipper_note(&note));
        assert_eq!(
            extract_source_url(&note).as_deref(),
            Some("https://blog.example.com/post")
        );
    }

    #[test]
    fn web_clipper_detected_from_cli_frontmatter() {
        let raw = "---\ntitle: Test Article\nsourceUrl: https://example.com/art\ntype: web-clip\nclipped: 2026-07-30T12:00:00Z\n---\n\nArticle body.";
        let note = NoteDocument {
            body: raw.to_string(),
            ..Default::default()
        };
        assert!(is_web_clipper_note(&note));
        assert_eq!(
            extract_source_url(&note).as_deref(),
            Some("https://example.com/art")
        );
    }

    #[test]
    fn regular_note_not_flagged_as_clipper() {
        let note = NoteDocument {
            body: "My thoughts on software architecture.".to_string(),
            ..Default::default()
        };
        assert!(!is_web_clipper_note(&note));
        assert!(extract_source_url(&note).is_none());
    }

    #[test]
    fn reading_info_provides_complete_metadata() {
        let note = NoteDocument {
            body: "> Source: https://example.com\n\nFour word body text.".to_string(),
            ..Default::default()
        };
        let info = reading_info(&note);
        assert!(info.is_web_clip);
        assert!(info.source_url.is_some());
        assert!(info.estimate.word_count >= 3); // "Four word body text" minus "Source: https://example.com"
    }

    #[test]
    fn reading_estimate_struct_fields() {
        let est = ReadingEstimate {
            minutes: 5,
            word_count: 1000,
        };
        assert_eq!(est.minutes, 5);
        assert_eq!(est.word_count, 1000);
    }
}
