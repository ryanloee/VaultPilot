//! Reader Mode utilities (#3150).
//!
//! Backend foundation for the Reader Mode feature: reading-time estimation and
//! web-clipper note detection.  The UI layer (WinUI / Mobile) can call these
//! functions to decide whether to show a "Reader Mode" button and to display
//! estimated reading time and source metadata.

use crate::models::NoteDocument;
use crate::storage::notes::split_frontmatter_yaml;

/// Average adult reading speed in words per minute.
/// 200 WPM is the widely-used industry standard (Medium, dev.to, etc.).
const WORDS_PER_MINUTE: usize = 200;

/// Minimum reading time returned (never show "0 min read").
const MIN_READING_MINUTES: usize = 1;

/// Estimated reading metadata for a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingEstimate {
    /// Estimated reading time in whole minutes (minimum 1).
    pub minutes: usize,
    /// Approximate word count of the body content.
    pub word_count: usize,
}

/// Estimate reading time from a note's body content.
///
/// Strips markdown syntax (code fences, inline code, links, images, headings)
/// before counting words, giving a more accurate estimate for technical
/// content that is heavy on code blocks.
///
/// # Example
/// ```
/// use vaultpilot_lib::reader::estimate_reading_time;
///
/// let est = estimate_reading_time("Hello world. This is a test.");
/// assert!(est.minutes >= 1);
/// assert!(est.word_count > 0);
/// ```
pub fn estimate_reading_time(body: &str) -> ReadingEstimate {
    let clean = strip_markdown(body);
    let word_count = count_words(&clean);
    let minutes = if word_count == 0 {
        MIN_READING_MINUTES
    } else {
        ((word_count as f64) / WORDS_PER_MINUTE as f64).ceil() as usize
    };
    ReadingEstimate {
        minutes: minutes.max(MIN_READING_MINUTES),
        word_count,
    }
}

/// Estimate reading time from a [`NoteDocument`].
///
/// Convenience wrapper that passes the note body to [`estimate_reading_time`].
pub fn estimate_reading_time_for_note(note: &NoteDocument) -> ReadingEstimate {
    estimate_reading_time(&note.body)
}

/// Check whether a note was created by the Web Clipper.
///
/// Web-clipper notes have frontmatter keys `sourceUrl` and/or `type: web-clip`
/// (see `src/bin/vaultpilot-cli/main.rs` clip command).  This function parses
/// the raw note body's frontmatter YAML to detect those keys, because the
/// typed [`crate::storage::Frontmatter`] struct does not capture them.
///
/// Falls back to checking the `> Source:` body marker used by the HTTP bridge
/// clipper endpoint.
pub fn is_web_clipper_note(note: &NoteDocument) -> bool {
    // Strategy 1: Check raw frontmatter YAML for sourceUrl / type: web-clip.
    // We need to check the *raw* content because NoteDocument may have already
    // lost the sourceUrl during frontmatter parsing.  The body alone is not
    // enough — but we can check the note's source field and body markers.
    if note.meta.source == "web-clip" || note.meta.source == "web_clipper" {
        return true;
    }

    // Strategy 2: Body-level source marker used by the HTTP bridge.
    if note.body.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("> Source:") || trimmed.starts_with("> Source：")
    }) {
        return true;
    }

    // Strategy 3: Check if the raw body starts with frontmatter containing
    // sourceUrl.  We attempt to split the frontmatter from the body text.
    // NoteDocument.body may or may not contain the original frontmatter —
    // when loaded from storage it typically doesn't.  But when checking a
    // raw note (e.g. from import), it might.
    if let Ok((yaml, _)) = split_frontmatter_yaml(&note.body) {
        if !yaml.is_empty() {
            if yaml
                .get("sourceUrl")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
            {
                return true;
            }
            if yaml
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s == "web-clip")
                .unwrap_or(false)
            {
                return true;
            }
        }
    }

    false
}

/// Extract the source URL from a web-clipper note, if present.
///
/// Checks:
/// 1. The `sourceUrl` key in frontmatter YAML (if body still contains frontmatter)
/// 2. The `> Source: {url}` body marker
pub fn extract_source_url(note: &NoteDocument) -> Option<String> {
    // Strategy 1: frontmatter YAML.
    if let Ok((yaml, _)) = split_frontmatter_yaml(&note.body) {
        if let Some(url) = yaml
            .get("sourceUrl")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
        {
            return Some(url);
        }
    }

    // Strategy 2: body-level "> Source: {url}" marker.
    for line in note.body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("> Source:")
            .or_else(|| trimmed.strip_prefix("> Source："))
        {
            let url = rest.trim();
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }
    }

    None
}

/// Comprehensive reading metadata for a note — combines reading time estimate
/// with web-clipper source information.
#[derive(Debug, Clone)]
pub struct NoteReadingInfo {
    pub estimate: ReadingEstimate,
    pub is_web_clip: bool,
    pub source_url: Option<String>,
}

/// Get all reading-related metadata for a note in one call.
pub fn reading_info(note: &NoteDocument) -> NoteReadingInfo {
    NoteReadingInfo {
        estimate: estimate_reading_time_for_note(note),
        is_web_clip: is_web_clipper_note(note),
        source_url: extract_source_url(note),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Remove markdown formatting to get plain text for word counting.
fn strip_markdown(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_code_fence = false;

    for line in text.lines() {
        let trimmed = line.trim();

        // Toggle code fence state.
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            continue;
        }

        // Skip content inside code fences (code is hard to "read" and inflates
        // the estimate — Medium and similar tools skip code blocks).
        if in_code_fence {
            continue;
        }

        // Skip image-only lines.
        if trimmed.starts_with("![") && trimmed.ends_with(')') {
            continue;
        }

        // Process the line: strip inline markdown syntax.
        let cleaned = strip_inline_markdown(trimmed);
        if !cleaned.is_empty() {
            result.push_str(&cleaned);
            result.push(' ');
        }
    }

    result
}

/// Strip inline markdown syntax: headings, bold, italic, inline code, links.
fn strip_inline_markdown(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut in_inline_code = false;

    while i < chars.len() {
        let c = chars[i];

        // Toggle inline code.
        if c == '`' {
            in_inline_code = !in_inline_code;
            i += 1;
            continue;
        }

        // Inside inline code: keep text but skip the backtick markers.
        if in_inline_code {
            out.push(c);
            i += 1;
            continue;
        }

        // Heading markers at line start.
        if c == '#' && out.is_empty() {
            i += 1;
            continue;
        }

        // Bold/italic markers.
        if c == '*' || c == '_' {
            // Skip ** and * and ___ etc.
            i += 1;
            continue;
        }

        // Image syntax: ![alt](url) → skip entirely (images aren't read).
        if c == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some(close) = find_matching_bracket(&chars, i + 1) {
                if close + 1 < chars.len() && chars[close + 1] == '(' {
                    if let Some(close_paren) = find_matching_paren(&chars, close + 1) {
                        i = close_paren + 1;
                        continue;
                    }
                }
                // `![alt]` without `()` — skip the bracket.
                i = close + 1;
                continue;
            }
            // Just a `!` — keep it.
        }

        // Markdown links: [text](url) → keep text, skip url.
        if c == '[' {
            if let Some(close) = find_matching_bracket(&chars, i) {
                // Check for `](url)` pattern.
                if close + 1 < chars.len() && chars[close + 1] == '(' {
                    if let Some(close_paren) = find_matching_paren(&chars, close + 1) {
                        // Extract link text.
                        let text: String = chars[i + 1..close].iter().collect();
                        out.push_str(&text);
                        i = close_paren + 1;
                        continue;
                    }
                }
                // Just a bracket without link — keep inner text.
                let text: String = chars[i + 1..close].iter().collect();
                out.push_str(&text);
                i = close + 1;
                continue;
            }
        }

        // Skip blockquote markers at start.
        if c == '>' && out.is_empty() {
            i += 1;
            continue;
        }

        out.push(c);
        i += 1;
    }

    out.trim().to_string()
}

fn find_matching_bracket(chars: &[char], start: usize) -> Option<usize> {
    for (idx, &c) in chars.iter().enumerate().skip(start + 1) {
        if c == ']' {
            return Some(idx);
        }
    }
    None
}

fn find_matching_paren(chars: &[char], start: usize) -> Option<usize> {
    for (idx, &c) in chars.iter().enumerate().skip(start + 1) {
        if c == ')' {
            return Some(idx);
        }
    }
    None
}

/// Count words in plain text (Unicode-aware).
fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_returns_min_reading_time() {
        let est = estimate_reading_time("");
        assert_eq!(est.minutes, MIN_READING_MINUTES);
        assert_eq!(est.word_count, 0);
    }

    #[test]
    fn short_text_returns_one_minute() {
        let est = estimate_reading_time("Hello world. This is a short text.");
        assert_eq!(est.minutes, 1);
        assert!(est.word_count > 0);
    }

    #[test]
    fn long_text_estimates_correctly() {
        // 400 words → should be 2 minutes at 200 WPM.
        let words: Vec<&str> = (0..400).map(|_| "word").collect();
        let text = words.join(" ");
        let est = estimate_reading_time(&text);
        assert_eq!(est.word_count, 400);
        assert_eq!(est.minutes, 2);
    }

    #[test]
    fn code_fences_are_skipped() {
        let with_code = "Some text here.\n\n```rust\nlet x = 42;\n```\n\nMore text.";
        let without_code = "Some text here. More text.";
        assert_eq!(
            estimate_reading_time(with_code).word_count,
            estimate_reading_time(without_code).word_count
        );
    }

    #[test]
    fn images_are_skipped() {
        let with_img = "Text ![alt](image.png) more text.";
        let without_img = "Text more text.";
        assert_eq!(
            estimate_reading_time(with_img).word_count,
            estimate_reading_time(without_img).word_count
        );
    }

    #[test]
    fn headings_are_stripped() {
        let est = estimate_reading_time("## Heading\n\nSome body text.");
        // "Heading" and "Some body text" = 4 words
        assert_eq!(est.word_count, 4);
    }

    #[test]
    fn links_keep_text() {
        let est = estimate_reading_time("See [this link](https://example.com) for info.");
        // "See this link for info" = 5 words
        assert_eq!(est.word_count, 5);
    }

    #[test]
    fn web_clipper_detection_via_source_marker() {
        let note = NoteDocument {
            body: "> Source: https://example.com/article\n\nArticle body.".to_string(),
            ..Default::default()
        };
        assert!(is_web_clipper_note(&note));
        assert_eq!(
            extract_source_url(&note),
            Some("https://example.com/article".to_string())
        );
    }

    #[test]
    fn web_clipper_detection_via_frontmatter() {
        let note = NoteDocument {
            body: "---\ntitle: Test\nsourceUrl: https://example.com\ntype: web-clip\n---\n\nBody."
                .to_string(),
            ..Default::default()
        };
        assert!(is_web_clipper_note(&note));
        assert_eq!(
            extract_source_url(&note),
            Some("https://example.com".to_string())
        );
    }

    #[test]
    fn non_clipper_note_not_detected() {
        let note = NoteDocument {
            body: "Just a regular note with some text.".to_string(),
            ..Default::default()
        };
        assert!(!is_web_clipper_note(&note));
        assert_eq!(extract_source_url(&note), None);
    }

    #[test]
    fn reading_info_combines_all() {
        let note = NoteDocument {
            body: "> Source: https://example.com\n\nThis is a clipped article body.".to_string(),
            ..Default::default()
        };
        let info = reading_info(&note);
        assert!(info.is_web_clip);
        assert!(info.source_url.is_some());
        assert!(info.estimate.word_count > 0);
    }

    #[test]
    fn chinese_text_word_count() {
        // CJK text doesn't use spaces between characters.
        // split_whitespace counts by whitespace, so CJK text without spaces
        // counts as 1 "word" — this is a known limitation.  The reading time
        // will be conservative (overestimate) for CJK content.
        let est = estimate_reading_time("这是一段中文文本内容");
        assert!(est.word_count >= 1);
    }
}
