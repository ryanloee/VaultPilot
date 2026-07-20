//! Web Clipper — minimal pure-Rust HTML → Markdown converter (#3189).
//!
//! This is the backend "collect" half of the Web Clipper feature: turn an
//! arbitrary web page's HTML into clean, readable Markdown so it can be stored
//! as a vault note. The browser-extension UI is a follow-up; the CLI
//! `vaultpilot clip <url>` command consumes this module.
//!
//! The converter depends only on `std` so it is fully unit testable without
//! network access. It covers the common article constructs: headings,
//! paragraphs, emphasis, inline code, fenced code blocks, links, images,
//! (nested) lists, blockquotes, and horizontal rules. `<script>`/`<style>`
//! content is dropped.

use std::fmt::Write as _;

/// Convert an HTML document (or fragment) into Markdown.
///
/// The function is deterministic and side-effect free: the same input always
/// yields the same output, which keeps the regression tests stable.
pub fn html_to_markdown(html: &str) -> String {
    let cleaned = strip_script_style(html);
    let chars: Vec<char> = cleaned.chars().collect();
    let mut pos = 0usize;
    let mut out = String::with_capacity(cleaned.len());
    parse_blocks(&chars, &mut pos, &mut out, 0);
    // Collapse 3+ consecutive newlines into 2, then trim trailing whitespace.
    let collapsed = collapse_blank_lines(&out);
    collapsed.trim_end().to_string() + "\n"
}

/// Recursively parse block-level content, writing Markdown to `out`.
///
/// `list_depth` is unused for indentation here (lists manage their own indent)
/// but is kept so the signature is stable for nested blockquotes if needed.
fn parse_blocks(chars: &[char], pos: &mut usize, out: &mut String, _list_depth: usize) {
    let mut paragraph = String::new();

    while *pos < chars.len() {
        if chars[*pos] == '<' {
            // Read the tag name.
            *pos += 1;
            let mut name = String::new();
            while *pos < chars.len() && chars[*pos] != '>' && !chars[*pos].is_whitespace() {
                name.push(chars[*pos]);
                *pos += 1;
            }
            // Capture attributes (until '>').
            let attr_start = *pos;
            while *pos < chars.len() && chars[*pos] != '>' {
                *pos += 1;
            }
            let attrs: String = if attr_start < *pos {
                chars[attr_start..*pos].iter().collect()
            } else {
                String::new()
            };
            if *pos < chars.len() {
                *pos += 1; // consume '>'
            }
            let lower = name.to_ascii_lowercase();
            let is_close = lower.starts_with('/');
            let tag = if is_close { &lower[1..] } else { &lower[..] };

            match tag {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    flush_paragraph(out, &mut paragraph);
                    if is_close {
                        out.push('\n');
                    } else {
                        let level = tag[1..].parse::<usize>().unwrap_or(1);
                        let inner = collect_inner_html(chars, pos, tag, false);
                        let text = inline(&inner).trim().to_string();
                        if !text.is_empty() {
                            let _ = writeln!(out, "{} {}", "#".repeat(level), text);
                            out.push('\n');
                        }
                    }
                }
                "p" => {
                    flush_paragraph(out, &mut paragraph);
                    if is_close {
                        flush_paragraph(out, &mut paragraph);
                    }
                }
                "br" => {
                    paragraph.push('\n');
                }
                "hr" => {
                    flush_paragraph(out, &mut paragraph);
                    out.push_str("\n---\n\n");
                }
                "ul" | "ol" if !is_close => {
                    flush_paragraph(out, &mut paragraph);
                    let ordered = tag == "ol";
                    render_list(chars, pos, ordered, 0, out);
                    out.push('\n');
                }
                "blockquote" if !is_close => {
                    flush_paragraph(out, &mut paragraph);
                    let inner = collect_inner_html(chars, pos, "blockquote", true);
                    let inner_vec = inner_chars(&inner);
                    let mut quoted = String::new();
                    parse_blocks(&inner_vec, &mut 0, &mut quoted, 0);
                    for line in quoted.lines() {
                        let _ = writeln!(out, "> {}", line);
                    }
                    out.push('\n');
                }
                "a" if !is_close => {
                    let href = extract_attr(&attrs).unwrap_or_default();
                    let inner = collect_inner_html(chars, pos, "a", false);
                    let label = inline(&inner).trim().to_string();
                    if !href.is_empty() && href != "#" {
                        let _ = write!(paragraph, "[{}]({})", label, href);
                    } else {
                        paragraph.push_str(&label);
                    }
                }
                "img" if !is_close => {
                    let src = extract_attr(&attrs).unwrap_or_default();
                    let alt = extract_named_attr(&attrs, "alt").unwrap_or_default();
                    let _ = write!(paragraph, "![{}]({})", alt, src);
                }
                "code" if !is_close => {
                    let inner = collect_inner_html(chars, pos, "code", false);
                    let _ = write!(paragraph, "`{}`", inner.trim());
                }
                "pre" if !is_close => {
                    flush_paragraph(out, &mut paragraph);
                    let inner = collect_inner_html(chars, pos, "pre", false);
                    out.push_str("```\n");
                    out.push_str(inner.trim_end());
                    out.push_str("\n```\n\n");
                }
                "strong" | "b" if !is_close => {
                    let inner = collect_inner_html(chars, pos, tag, false);
                    let _ = write!(paragraph, "**{}**", inline(&inner).trim());
                }
                "em" | "i" if !is_close => {
                    let inner = collect_inner_html(chars, pos, tag, false);
                    let _ = write!(paragraph, "*{}*", inline(&inner).trim());
                }
                "script" | "style" => {
                    // Already stripped up front; guard nested occurrences.
                    collect_inner_html(chars, pos, tag, false);
                }
                _ => {
                    // Unknown tag: ignore (its text content still flows through).
                }
            }
        } else {
            // Plain text until the next '<'.
            let mut text = String::new();
            while *pos < chars.len() && chars[*pos] != '<' {
                text.push(chars[*pos]);
                *pos += 1;
            }
            paragraph.push_str(&decode_entities(&text));
        }
    }

    flush_paragraph(out, &mut paragraph);
}

/// Render a `<ul>`/`<ol>` list (and any nested lists) starting at `*pos`.
fn render_list(chars: &[char], pos: &mut usize, ordered: bool, indent: usize, out: &mut String) {
    let mut index = 0usize;
    skip_ws(chars, pos);
    // If we're pointing at the list container's own opening tag (<ul>/<ol>),
    // skip past it so the first <li> is what we process.
    if peek_tag_is(chars, *pos, "ul") || peek_tag_is(chars, *pos, "ol") {
        *pos += 1;
        while *pos < chars.len() && chars[*pos] != '>' {
            *pos += 1;
        }
        if *pos < chars.len() {
            *pos += 1; // '>'
        }
        skip_ws(chars, pos);
    }
    while *pos < chars.len() {
        // Expect an <li> tag.
        if !peek_tag_is(chars, *pos, "li") {
            break;
        }
        // Consume the <li> opening tag.
        *pos += 1; // '<'
        while *pos < chars.len() && chars[*pos] != '>' {
            *pos += 1;
        }
        if *pos < chars.len() {
            *pos += 1; // '>'
        }
        // Collect the inner HTML of this <li> (balanced).
        let inner = collect_inner_html(chars, pos, "li", true);
        // Split into the leading text and any nested list.
        let (text_part, nested) = split_nested_list(&inner);
        let label = inline(&text_part).trim().to_string();
        let marker = if ordered {
            index += 1;
            format!("{}. ", index)
        } else {
            "- ".to_string()
        };
        let _ = writeln!(out, "{}{}{}", "  ".repeat(indent), marker, label);
        if let Some(nested_html) = nested {
            let nested_ordered = nested_html.to_ascii_lowercase().starts_with("<ol");
            let nested_vec = inner_chars(&nested_html);
            render_list(&nested_vec, &mut 0, nested_ordered, indent + 1, out);
        }
        skip_ws(chars, pos);
    }
}

/// Split inner-`<li>` HTML into the leading text and an optional nested list
/// (`<ul>...</ul>` or `<ol>...</ol>`) that begins after the text.
fn split_nested_list(inner: &str) -> (String, Option<String>) {
    let bytes = inner.as_bytes();
    let mut i = 0;
    let mut depth = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &inner[i..];
            let low = rest.to_ascii_lowercase();
            if low.starts_with("<ul") || low.starts_with("<ol") {
                if depth == 0 {
                    // Everything before is the text part.
                    let text_part = inner[..i].to_string();
                    let nested = inner[i..].to_string();
                    return (text_part, Some(nested));
                }
                depth += 1;
            } else if low.starts_with("</ul>") || low.starts_with("</ol>") {
                depth -= 1;
            }
        }
        i += 1;
    }
    (inner.to_string(), None)
}

fn flush_paragraph(out: &mut String, paragraph: &mut String) {
    let trimmed = paragraph.trim();
    if !trimmed.is_empty() {
        out.push_str(trimmed);
        out.push_str("\n\n");
    }
    paragraph.clear();
}

/// Collect the raw inner HTML between an opening tag and its matching closing
/// tag (balancing nested tags of the same name). Consumes the closing tag.
///
/// When `preserve_closes` is true, closing tags are kept in the returned buffer
/// so the inner HTML remains well-formed for recursive re-parsing (e.g. nested
/// lists / blockquotes). When false (inline elements), closing tags are
/// dropped so the text can be passed straight to `inline()`.
fn collect_inner_html(
    chars: &[char],
    pos: &mut usize,
    name: &str,
    preserve_closes: bool,
) -> String {
    let close = format!("/{}", name.to_ascii_lowercase());
    let open = name.to_ascii_lowercase();
    let mut depth = 1;
    let mut buf = String::new();
    while *pos < chars.len() {
        if chars[*pos] == '<' {
            *pos += 1;
            let mut tag = String::new();
            while *pos < chars.len() && chars[*pos] != '>' && !chars[*pos].is_whitespace() {
                tag.push(chars[*pos]);
                *pos += 1;
            }
            while *pos < chars.len() && chars[*pos] != '>' {
                *pos += 1;
            }
            if *pos < chars.len() {
                *pos += 1;
            }
            let lower = tag.to_ascii_lowercase();
            if lower == close {
                depth -= 1;
                if preserve_closes {
                    buf.push('<');
                    buf.push_str(&tag);
                    buf.push('>');
                }
                if depth == 0 {
                    break;
                }
            } else if lower == open {
                depth += 1;
                buf.push('<');
                buf.push_str(&tag);
                buf.push('>');
            } else {
                // Preserve the tag verbatim so nested structures survive.
                buf.push('<');
                buf.push_str(&tag);
                buf.push('>');
            }
        } else {
            buf.push(chars[*pos]);
            *pos += 1;
        }
    }
    buf
}

fn inner_chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn skip_ws(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos].is_whitespace() {
        *pos += 1;
    }
}

fn peek_tag_is(chars: &[char], pos: usize, name: &str) -> bool {
    if pos >= chars.len() || chars[pos] != '<' {
        return false;
    }
    let rest: String = chars[pos + 1..].iter().take(name.len()).collect();
    rest.eq_ignore_ascii_case(name)
}

/// Inline-level conversions: decode entities and collapse whitespace.
fn inline(text: &str) -> String {
    let decoded = decode_entities(text);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn strip_script_style(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &html[i..];
            let lower = rest.to_ascii_lowercase();
            if lower.starts_with("<script") || lower.starts_with("<style") {
                let close_name = if lower.starts_with("<script") {
                    "</script"
                } else {
                    "</style"
                };
                if let Some(pos) = lower.find(close_name) {
                    let end = i + pos + close_name.len();
                    let mut j = end;
                    while j < bytes.len() && bytes[j] != b'>' {
                        j += 1;
                    }
                    i = (j + 1).min(bytes.len());
                    continue;
                }
            }
        }
        result.push(html[i..].chars().next().unwrap());
        i += 1;
    }
    result
}

fn extract_attr(attrs: &str) -> Option<String> {
    extract_named_attr(attrs, "href").or_else(|| extract_named_attr(attrs, "src"))
}

fn extract_named_attr(attrs: &str, key: &str) -> Option<String> {
    let chars: Vec<char> = attrs.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Find the key.
        let window: String = chars[i..].iter().take(key.len()).collect();
        if window.eq_ignore_ascii_case(key)
            && (i + key.len() >= chars.len()
                || chars[i + key.len()].is_whitespace()
                || chars[i + key.len()] == '='
                || chars[i + key.len()] == '>')
        {
            i += key.len();
            // Skip whitespace.
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '=' {
                i += 1;
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                let quote = if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                    Some(chars[i])
                } else {
                    None
                };
                if let Some(q) = quote {
                    i += 1;
                    let mut val = String::new();
                    while i < chars.len() && chars[i] != q {
                        val.push(chars[i]);
                        i += 1;
                    }
                    return Some(val);
                } else {
                    let mut val = String::new();
                    while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '>' {
                        val.push(chars[i]);
                        i += 1;
                    }
                    return Some(val);
                }
            }
        }
        i += 1;
    }
    None
}

fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for c in s.chars() {
        if c == '\n' {
            blank += 1;
            if blank <= 2 {
                out.push(c);
            }
        } else {
            blank = 0;
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heading() {
        let md = html_to_markdown("<h1>Title</h1>");
        assert!(md.contains("# Title"), "got: {md}");
    }

    #[test]
    fn test_paragraph() {
        let md = html_to_markdown("<p>Hello world</p>");
        assert!(md.contains("Hello world"), "got: {md}");
    }

    #[test]
    fn test_bold_italic() {
        let md = html_to_markdown("<p><strong>bold</strong> and <em>italic</em></p>");
        assert!(md.contains("**bold**"), "got: {md}");
        assert!(md.contains("*italic*"), "got: {md}");
    }

    #[test]
    fn test_link() {
        let md = html_to_markdown(r#"<p>see <a href="https://example.com">site</a></p>"#);
        assert!(md.contains("[site](https://example.com)"), "got: {md}");
    }

    #[test]
    fn test_inline_code() {
        let md = html_to_markdown("<p>use <code>cargo test</code> here</p>");
        assert!(md.contains("`cargo test`"), "got: {md}");
    }

    #[test]
    fn test_unordered_list() {
        let md = html_to_markdown("<ul><li>one</li><li>two</li></ul>");
        assert!(md.contains("- one"), "got: {md}");
        assert!(md.contains("- two"), "got: {md}");
    }

    #[test]
    fn test_ordered_list() {
        let md = html_to_markdown("<ol><li>first</li><li>second</li></ol>");
        assert!(md.contains("1. first"), "got: {md}");
        assert!(md.contains("2. second"), "got: {md}");
    }

    #[test]
    fn test_blockquote() {
        let md = html_to_markdown("<blockquote><p>quoted text</p></blockquote>");
        assert!(md.contains("> quoted text"), "got: {md}");
    }

    #[test]
    fn test_code_block() {
        let md = html_to_markdown("<pre><code>fn main() {}</code></pre>");
        assert!(md.contains("```"), "got: {md}");
        assert!(md.contains("fn main() {}"), "got: {md}");
    }

    #[test]
    fn test_script_style_stripped() {
        let md =
            html_to_markdown("<style>.x{color:red}</style><p>visible</p><script>alert(1)</script>");
        assert!(md.contains("visible"), "got: {md}");
        assert!(!md.contains("alert"), "got: {md}");
        assert!(!md.contains("color:red"), "got: {md}");
    }

    #[test]
    fn test_html_entities_decoded() {
        let md = html_to_markdown("<p>Tom &amp; Jerry &lt;3</p>");
        assert!(md.contains("Tom & Jerry"), "got: {md}");
        assert!(md.contains("<3"), "got: {md}");
    }

    #[test]
    fn test_image() {
        let md = html_to_markdown(r#"<p><img src="pic.png" alt="cat"></p>"#);
        assert!(md.contains("![cat](pic.png)"), "got: {md}");
    }

    #[test]
    fn test_nested_list() {
        let md = html_to_markdown("<ul><li>top<ul><li>child</li></ul></li></ul>");
        assert!(md.contains("- top"), "got: {md}");
        assert!(md.contains("  - child"), "got: {md}");
    }

    #[test]
    fn test_full_article() {
        let html = r#"
        <html><head><title>My Article</title></head>
        <body>
          <h1>My Article</h1>
          <p>This is a <strong>great</strong> article about <a href="https://rust-lang.org">Rust</a>.</p>
          <h2>Section</h2>
          <ul><li>point one</li><li>point two</li></ul>
          <blockquote><p>wise words</p></blockquote>
        </body></html>
        "#;
        let md = html_to_markdown(html);
        assert!(md.contains("# My Article"), "got: {md}");
        assert!(md.contains("**great**"), "got: {md}");
        assert!(md.contains("[Rust](https://rust-lang.org)"), "got: {md}");
        assert!(md.contains("## Section"), "got: {md}");
        assert!(md.contains("- point one"), "got: {md}");
        assert!(md.contains("> wise words"), "got: {md}");
    }
}
