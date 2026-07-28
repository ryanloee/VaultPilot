//! AI Chat @-mention context injection (#3548).
//!
//! When a user types `@NoteTitle` or `@[Multi Word Title]` in the chat input,
//! this module resolves those mentions to actual vault notes and injects their
//! content into the prompt as additional context — similar to how OCR text and
//! tweet content are already injected by [`super::chat::build_effective_question`].
//!
//! ## Syntax
//!
//! - `@word` — matches a single word (letters, digits, `-`, `_`, CJK chars)
//!   until whitespace or punctuation.
//! - `@[Everything in brackets]` — matches arbitrary text (including spaces)
//!   between the bracket pair immediately following `@`.
//!
//! ## Resolution
//!
//! Each extracted mention query is fed to [`typeahead_search`] which does a
//! case-insensitive `LIKE` lookup on note titles. The best match (most recently
//! updated) is selected and its full body is loaded and appended to the prompt
//! under a dedicated context header.

use crate::storage::{load_note_async, typeahead_search_async, StorageContext};

/// Header prepended to the injected mention context block, mirroring the style
/// of `OCR_SECTION_HEADER` in `chat.rs`.
pub(crate) const MENTION_CONTEXT_HEADER: &str = "[引用笔记上下文]:";

/// Separator drawn between individual note blocks inside the context section.
const NOTE_SEPARATOR: &str = "\n---\n";

/// Maximum number of notes to inject in a single chat turn. Prevents token
/// explosion when a user sprinkles many `@` mentions in one message.
pub(crate) const MAX_MENTION_NOTES: usize = 5;

/// Maximum character length of a single note's body to inject. Notes longer
/// than this are truncated with an ellipsis marker so the model knows content
/// was cut.
pub(crate) const MAX_NOTE_CONTENT_CHARS: usize = 2000;

/// Parse `@` mentions from the given text.
///
/// Returns a list of raw query strings (without the leading `@` or wrapping
/// brackets) in order of appearance, deduplicated while preserving first-seen
/// order.
///
/// # Syntax
///
/// - `@word` — word characters (alphanumeric, `-`, `_`, and any Unicode
///   letter/digit via `\w`), greedily matched until a boundary.
/// - `@[bracketed text]` — everything between `[` and `]` immediately after
///   `@`.
///
/// # Examples
///
/// ```
/// # use vaultpilot_lib::orchestration::mention::parse_at_mentions;
/// assert_eq!(parse_at_mentions("Hello @world"), vec!["world"]);
/// assert_eq!(parse_at_mentions("@[Meeting Notes] summary"), vec!["Meeting Notes"]);
/// assert_eq!(parse_at_mentions("@a @b @a"), vec!["a", "b"]);
/// assert!(parse_at_mentions("no mentions here").is_empty());
/// assert!(parse_at_mentions("email@example.com").is_empty());
/// ```
pub fn parse_at_mentions(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut mentions = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut i = 0;

    while i < len {
        if chars[i] != '@' {
            i += 1;
            continue;
        }

        // Skip standalone '@' at end of text
        if i + 1 >= len {
            break;
        }

        // Reject email-like patterns: an ASCII alphanumeric or '.' immediately
        // before '@' (e.g. "foo@bar.com"). CJK and other Unicode letters
        // before '@' are fine — they are natural sentence continuations in
        // non-Latin scripts and should still trigger a mention (#3548).
        if i > 0 && is_email_context_char(chars[i - 1]) {
            i += 1;
            continue;
        }

        // Bracketed form: @[...]
        if chars[i + 1] == '[' {
            if let Some(close_rel) = find_bracket_close(&chars, i + 2) {
                let inner: String = chars[i + 2..i + 2 + close_rel].iter().collect();
                let trimmed = inner.trim().to_string();
                if !trimmed.is_empty() && seen.insert(trimmed.clone()) {
                    mentions.push(trimmed);
                }
                // +1 for '@', +1 for '[', close_rel chars, +1 for ']'
                i = i + 2 + close_rel + 1;
                continue;
            }
            // Unclosed '[' — skip the '@' to avoid infinite loop
            i += 1;
            continue;
        }

        // Word form: @word
        let start = i + 1;
        let mut end = start;
        while end < len && is_mention_word_char(chars[end]) {
            end += 1;
        }
        if end > start {
            let word: String = chars[start..end].iter().collect();
            if !word.is_empty() && seen.insert(word.clone()) {
                mentions.push(word);
            }
            i = end;
        } else {
            // '@' not followed by a word char — skip
            i += 1;
        }
    }

    mentions
}

/// Check if a character is valid inside an unbracketed `@word` mention.
///
/// Matches ASCII alphanumeric, `_`, `-`, `.`, and any Unicode letter or digit
/// (so CJK note titles like `@笔记` work). Does **not** match spaces, which
/// is why `@[...]` exists for multi-word titles.
fn is_mention_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

/// Check if the character immediately before `@` makes it look like part of
/// an email address rather than a mention. Only ASCII alphanumerics and `.`
/// trigger this — e.g. `foo@bar.com`, `user.name@example.org`. Unicode
/// letters (CJK, Cyrillic, etc.) do **not** trigger it, since `关于@笔记`
/// is a valid mention in Chinese text.
fn is_email_context_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.'
}

/// Find the index (relative to `start`) of the closing `]` bracket, handling
/// nested brackets by returning the first unmatched `]`.
///
/// Returns `None` if no closing bracket is found.
fn find_bracket_close(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut j = start;
    while j < chars.len() {
        match chars[j] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j - start);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Resolve `@` mentions in the prompt to note content and append the results
/// as a context block.
///
/// This is the main entry point called by the chat orchestration layer after
/// [`super::chat::build_effective_question`]. It:
///
/// 1. Parses `@` mentions from the prompt text.
/// 2. For each mention, searches the vault for a matching note (best match by
///    recency).
/// 3. Loads the full note body.
/// 4. Appends a formatted context section to the prompt.
///
/// If no mentions are found, or no notes match, the prompt is returned
/// unchanged. Errors during note lookup are logged and silently skipped
/// (best-effort: a missing note should not abort the chat).
pub async fn inject_mention_context(context: &StorageContext, mut prompt: String) -> String {
    let mentions = parse_at_mentions(&prompt);
    if mentions.is_empty() {
        return prompt;
    }

    let mut note_blocks = Vec::new();
    let mut resolved = 0usize;

    for query in &mentions {
        if resolved >= MAX_MENTION_NOTES {
            break;
        }

        // Search for matching notes — take the single best (most recent) hit.
        let results = match typeahead_search_async(context, query, 1).await {
            Ok(metas) if !metas.is_empty() => metas,
            Ok(_) => {
                tracing::debug!(
                    query = %query,
                    "mention resolution: no matching note found, skipping"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    query = %query,
                    error = %e,
                    "mention resolution: search failed, skipping"
                );
                continue;
            }
        };

        let meta = &results[0];
        let note = match load_note_async(context, &meta.id).await {
            Ok(doc) => doc,
            Err(e) => {
                tracing::warn!(
                    note_id = %meta.id,
                    title = %meta.title,
                    error = %e,
                    "mention resolution: failed to load note body, skipping"
                );
                continue;
            }
        };

        let body = truncate_note_content(&note.body);
        note_blocks.push(format_note_block(&meta.title, &body));
        resolved += 1;
    }

    if note_blocks.is_empty() {
        return prompt;
    }

    prompt.push_str("\n\n");
    prompt.push_str(MENTION_CONTEXT_HEADER);
    prompt.push('\n');
    prompt.push_str(&note_blocks.join(NOTE_SEPARATOR));
    prompt
}

/// Truncate note content to [`MAX_NOTE_CONTENT_CHARS`] characters (counting
/// Unicode scalar values, not bytes), appending an ellipsis if truncation
/// occurred.
pub(crate) fn truncate_note_content(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    if chars.len() <= MAX_NOTE_CONTENT_CHARS {
        return body.to_string();
    }
    let truncated: String = chars[..MAX_NOTE_CONTENT_CHARS].iter().collect();
    format!("{truncated}…[截断: 原文 {total} 字符]", total = chars.len())
}

/// Format a single note's content block for injection into the prompt.
fn format_note_block(title: &str, body: &str) -> String {
    format!("📄 **{title}**\n{body}")
}

// ───────────────────────────── Tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_at_mentions ──

    #[test]
    fn test_no_mentions() {
        assert!(parse_at_mentions("Hello world").is_empty());
        assert!(parse_at_mentions("").is_empty());
        assert!(parse_at_mentions("Just some text without at-signs").is_empty());
    }

    #[test]
    fn test_simple_word_mention() {
        assert_eq!(parse_at_mentions("Hello @world"), vec!["world"]);
        assert_eq!(parse_at_mentions("@world hello"), vec!["world"]);
        assert_eq!(
            parse_at_mentions("Tell me about @rust and @python"),
            vec!["rust", "python"]
        );
    }

    #[test]
    fn test_bracketed_mention() {
        assert_eq!(
            parse_at_mentions("@[Meeting Notes] summary"),
            vec!["Meeting Notes"]
        );
        assert_eq!(
            parse_at_mentions("See @[Project Alpha Plan] for details"),
            vec!["Project Alpha Plan"]
        );
    }

    #[test]
    fn test_multiple_brackets() {
        let text = "@[Note One] and @[Note Two]";
        assert_eq!(parse_at_mentions(text), vec!["Note One", "Note Two"]);
    }

    #[test]
    fn test_deduplication() {
        assert_eq!(parse_at_mentions("@a @b @a"), vec!["a", "b"]);
        assert_eq!(parse_at_mentions("@[Same] @[Same]"), vec!["Same"]);
    }

    #[test]
    fn test_email_not_mention() {
        assert!(parse_at_mentions("user@example.com").is_empty());
        assert!(parse_at_mentions("Contact me at foo@bar.com").is_empty());
    }

    #[test]
    fn test_standalone_at() {
        assert!(parse_at_mentions("end@").is_empty());
        assert!(parse_at_mentions("@ @ @").is_empty());
    }

    #[test]
    fn test_unclosed_bracket() {
        // Unclosed bracket should not produce a mention and should not loop
        assert!(parse_at_mentions("@[unclosed").is_empty());
        // Text after unclosed bracket should still be scanned
        assert_eq!(parse_at_mentions("@[unclosed and @valid"), vec!["valid"]);
    }

    #[test]
    fn test_cjk_mention() {
        // CJK characters should work in word mentions
        assert_eq!(parse_at_mentions("关于@笔记的讨论"), vec!["笔记的讨论"]);
    }

    #[test]
    fn test_cjk_bracketed() {
        assert_eq!(
            parse_at_mentions("@[会议记录 2026] 的总结"),
            vec!["会议记录 2026"]
        );
    }

    #[test]
    fn test_hyphen_dot_underscore() {
        assert_eq!(
            parse_at_mentions("See @my-note and @config.json and @test_case"),
            vec!["my-note", "config.json", "test_case"]
        );
    }

    #[test]
    fn test_mixed_word_and_bracket() {
        let text = "@rust is great, but see @[The Rust Book] for details";
        assert_eq!(parse_at_mentions(text), vec!["rust", "The Rust Book"]);
    }

    #[test]
    fn test_mention_at_end_of_text() {
        assert_eq!(parse_at_mentions("see @end"), vec!["end"]);
    }

    #[test]
    fn test_mention_with_trailing_punctuation() {
        // Punctuation (other than word chars) terminates the mention
        assert_eq!(parse_at_mentions("@note!"), vec!["note"]);
        assert_eq!(parse_at_mentions("(@note)"), vec!["note"]);
        assert_eq!(parse_at_mentions("@note?"), vec!["note"]);
    }

    // ── truncate_note_content ──

    #[test]
    fn test_truncate_short_content() {
        let body = "Short content";
        assert_eq!(truncate_note_content(body), body);
    }

    #[test]
    fn test_truncate_long_content() {
        let body = "a".repeat(3000);
        let result = truncate_note_content(&body);
        assert!(result.starts_with(&"a".repeat(MAX_NOTE_CONTENT_CHARS)));
        assert!(result.contains("截断"));
        assert!(result.contains("3000"));
    }

    #[test]
    fn test_truncate_exact_boundary() {
        let body = "a".repeat(MAX_NOTE_CONTENT_CHARS);
        // Exactly at the limit — no truncation
        assert_eq!(truncate_note_content(&body), body);
    }

    #[test]
    fn test_truncate_unicode_content() {
        // CJK chars: each is one Unicode scalar value
        let body = "你".repeat(MAX_NOTE_CONTENT_CHARS + 10);
        let result = truncate_note_content(&body);
        assert!(result.contains("截断"));
    }

    // ── format_note_block ──

    #[test]
    fn test_format_note_block() {
        let block = format_note_block("My Note", "Some body text");
        assert!(block.contains("My Note"));
        assert!(block.contains("Some body text"));
    }

    // ── inject_mention_context (no vault) ──

    #[test]
    fn test_inject_no_mentions_returns_unchanged() {
        // This doesn't need a real vault — the function returns early when
        // parse_at_mentions returns empty. We still need a StorageContext to
        // satisfy the signature, but it's never accessed.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "vp-mention-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let context = StorageContext::for_test(&dir);
        let prompt = "Hello world, no mentions here".to_string();
        let result = rt.block_on(async { inject_mention_context(&context, prompt).await });
        assert_eq!(result, "Hello world, no mentions here");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
