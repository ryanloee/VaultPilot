use vaultpilot_lib::models::*;

pub(super) const MARKDOWN_OPEN_TAG: &str = "<vp-markdown>";
pub(super) const MARKDOWN_CLOSE_TAG: &str = "</vp-markdown>";

pub(super) fn strip_cli_markdown_from_chat_result(
    mut result: ChatExchangeResult,
) -> ChatExchangeResult {
    result.answer = strip_cli_markdown_from_grounded_answer(result.answer);
    result.state = strip_cli_markdown_from_chat_state(result.state);
    result
}

pub(super) fn strip_cli_markdown_from_grounded_answer(
    mut answer: GroundedAnswer,
) -> GroundedAnswer {
    answer.answer = simplify_cli_text(&answer.answer);
    answer.thinking_trace = None;
    answer
}

pub(super) fn strip_cli_markdown_from_chat_state(mut state: ChatState) -> ChatState {
    for session in &mut state.sessions {
        for turn in &mut session.turns {
            if turn.role.eq_ignore_ascii_case("assistant") {
                turn.text = simplify_cli_text(&turn.text);
                turn.thinking_trace = None;
            }
        }
    }
    state
}

pub(super) fn strip_markdown_wrapper_tags(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with(MARKDOWN_OPEN_TAG) && trimmed.ends_with(MARKDOWN_CLOSE_TAG) {
        return trimmed[MARKDOWN_OPEN_TAG.len()..trimmed.len() - MARKDOWN_CLOSE_TAG.len()]
            .trim()
            .to_string();
    }

    text.to_string()
}

pub(super) fn simplify_cli_text(text: &str) -> String {
    let text = strip_markdown_wrapper_tags(text);
    let mut simplified = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        let line = if in_code_block {
            trimmed.to_string()
        } else {
            simplify_markdown_line(trimmed)
        };

        if line.is_empty() {
            if simplified
                .last()
                .is_some_and(|item: &String| !item.is_empty())
            {
                simplified.push(String::new());
            }
        } else {
            simplified.push(line);
        }
    }

    while simplified.last().is_some_and(|item| item.is_empty()) {
        simplified.pop();
    }

    simplified.join("\n")
}

fn simplify_markdown_line(line: &str) -> String {
    let without_heading = line.trim_start_matches('#').trim();
    let without_bullet = strip_markdown_list_marker(without_heading);
    strip_inline_markdown(without_bullet)
}

fn strip_markdown_list_marker(line: &str) -> &str {
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return rest.trim();
    }

    let bytes = line.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index > 0 && index + 1 < bytes.len() && bytes[index] == b'.' && bytes[index + 1] == b' ' {
        return line[index + 2..].trim();
    }

    line
}

fn strip_inline_markdown(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Inline code span: preserve everything inside backticks verbatim
            '`' => {
                result.push('`');
                for inner in chars.by_ref() {
                    result.push(inner);
                    if inner == '`' {
                        break;
                    }
                }
            }
            // Wikilink / Block reference [[note#^blockid|display]]
            '[' if chars.peek() == Some(&'[') => {
                chars.next(); // consume second [
                let mut inner = String::new();
                let mut expect_close = false;
                for ch in chars.by_ref() {
                    if expect_close {
                        if ch == ']' {
                            break; // ]] found
                        } else {
                            inner.push(']');
                            inner.push(ch);
                            expect_close = false;
                        }
                    } else if ch == ']' {
                        expect_close = true; // could be start of ]]
                    } else {
                        inner.push(ch);
                    }
                }
                // Extract display text: [[note|display]] → display; [[note]] → note
                let display = if let Some(pipe) = inner.rfind('|') {
                    inner[pipe + 1..].to_string()
                } else {
                    inner.trim().to_string()
                };
                result.push_str(&display);
            }
            // Bold marker **: skip the ** but keep the content
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); // consume second *
                              // Copy until closing **
                while let Some(inner) = chars.next() {
                    if inner == '*' && chars.peek() == Some(&'*') {
                        chars.next(); // consume closing *
                        break;
                    }
                    result.push(inner);
                }
            }
            // Strikethrough marker ~~: skip the ~~ but keep the content
            '~' if chars.peek() == Some(&'~') => {
                chars.next(); // consume second ~
                while let Some(inner) = chars.next() {
                    if inner == '~' && chars.peek() == Some(&'~') {
                        chars.next(); // consume closing ~
                        break;
                    }
                    result.push(inner);
                }
            }
            // Italic *: skip the * but keep the content
            '*' => {
                for inner in chars.by_ref() {
                    if inner == '*' {
                        break;
                    }
                    result.push(inner);
                }
            }
            // Italic/bold __: skip the __ but keep the content
            '_' if chars.peek() == Some(&'_') => {
                chars.next(); // consume second _
                while let Some(inner) = chars.next() {
                    if inner == '_' && chars.peek() == Some(&'_') {
                        chars.next(); // consume closing _
                        break;
                    }
                    result.push(inner);
                }
            }
            // Italic _: skip the _ but keep the content
            '_' => {
                for inner in chars.by_ref() {
                    if inner == '_' {
                        break;
                    }
                    result.push(inner);
                }
            }
            // Regular character: pass through
            _ => {
                result.push(c);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_markdown_wrapper_tags ────────────────────────────────

    #[test]
    fn strip_wrapper_tags_basic() {
        let input = "<vp-markdown>hello</vp-markdown>";
        assert_eq!(strip_markdown_wrapper_tags(input), "hello");
    }

    #[test]
    fn strip_wrapper_tags_with_whitespace() {
        let input = "  <vp-markdown>content</vp-markdown>  ";
        assert_eq!(strip_markdown_wrapper_tags(input), "content");
    }

    #[test]
    fn strip_wrapper_tags_plain_text_unchanged() {
        assert_eq!(strip_markdown_wrapper_tags("no tags here"), "no tags here");
    }

    #[test]
    fn strip_wrapper_tags_only_open() {
        assert_eq!(
            strip_markdown_wrapper_tags("<vp-markdown>incomplete"),
            "<vp-markdown>incomplete"
        );
    }

    // ── strip_inline_markdown ─────────────────────────────────────

    #[test]
    fn strip_bold() {
        assert_eq!(strip_inline_markdown("**bold**"), "bold");
    }

    #[test]
    fn strip_italic_star() {
        assert_eq!(strip_inline_markdown("*italic*"), "italic");
    }

    #[test]
    fn strip_italic_underscore() {
        assert_eq!(strip_inline_markdown("_italic_"), "italic");
    }

    #[test]
    fn strip_bold_underscore() {
        assert_eq!(strip_inline_markdown("__bold__"), "bold");
    }

    #[test]
    fn strip_strikethrough() {
        assert_eq!(strip_inline_markdown("~~struck~~"), "struck");
    }

    #[test]
    fn preserve_code_span() {
        assert_eq!(strip_inline_markdown("`code`"), "`code`");
    }

    #[test]
    fn plain_text_unchanged() {
        assert_eq!(strip_inline_markdown("hello world"), "hello world");
    }

    #[test]
    fn strip_bold_with_surrounding_text() {
        assert_eq!(
            strip_inline_markdown("before **bold** after"),
            "before bold after"
        );
    }

    #[test]
    fn unclosed_bold_passthrough() {
        // Unclosed ** should pass through the * as regular chars
        let result = strip_inline_markdown("**unclosed");
        // The ** gets consumed as opening, then no closing found
        assert_eq!(result, "unclosed");
    }

    // ── strip_markdown_list_marker ────────────────────────────────

    #[test]
    fn strip_dash_bullet() {
        assert_eq!(strip_markdown_list_marker("- item"), "item");
    }

    #[test]
    fn strip_star_bullet() {
        assert_eq!(strip_markdown_list_marker("* item"), "item");
    }

    #[test]
    fn strip_plus_bullet() {
        assert_eq!(strip_markdown_list_marker("+ item"), "item");
    }

    #[test]
    fn strip_numbered_list() {
        assert_eq!(strip_markdown_list_marker("1. first"), "first");
    }

    #[test]
    fn strip_multi_digit_number() {
        assert_eq!(strip_markdown_list_marker("12. twelfth"), "twelfth");
    }

    #[test]
    fn no_marker_unchanged() {
        assert_eq!(strip_markdown_list_marker("plain text"), "plain text");
    }

    // ── simplify_cli_text ─────────────────────────────────────────

    #[test]
    fn simplify_removes_headings() {
        assert_eq!(simplify_cli_text("# Heading"), "Heading");
    }

    #[test]
    fn simplify_removes_code_blocks() {
        let input = "before\n```\ncode here\n```\nafter";
        assert_eq!(simplify_cli_text(input), "before\ncode here\nafter");
    }

    #[test]
    fn simplify_collapses_empty_lines() {
        let input = "line1\n\n\nline2";
        assert_eq!(simplify_cli_text(input), "line1\n\nline2");
    }

    #[test]
    fn simplify_strips_wrapper_and_content() {
        let input = "<vp-markdown>**bold** text</vp-markdown>";
        assert_eq!(simplify_cli_text(input), "bold text");
    }

    #[test]
    fn simplify_trims_trailing_whitespace() {
        let input = "content\n\n";
        assert_eq!(simplify_cli_text(input), "content");
    }

    // ── wikilink / block reference stripping ──────────────────────

    #[test]
    fn strip_wikilink_simple() {
        assert_eq!(strip_inline_markdown("[[My Note]]"), "My Note");
    }

    #[test]
    fn strip_wikilink_with_display() {
        assert_eq!(
            strip_inline_markdown("[[My Note|click here]]"),
            "click here",
        );
    }

    #[test]
    fn strip_block_reference() {
        assert_eq!(
            strip_inline_markdown("[[My Note#^abc123]]"),
            "My Note#^abc123",
        );
    }

    #[test]
    fn strip_wikilink_with_surrounding_text() {
        assert_eq!(
            strip_inline_markdown("See [[note]] for details"),
            "See note for details",
        );
    }
}
