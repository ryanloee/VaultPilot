//! Tweet/X post import (#1864).
//!
//! Detects `twitter.com`/`x.com`/`mobile.twitter.com` status URLs in chat
//! input and fetches the tweet content via Twitter's public oEmbed API
//! (`https://publish.twitter.com/oembed`).  The oEmbed endpoint requires
//! **no API key** and returns structured metadata (author, HTML embed).
//!
//! The implementation gracefully degrades: if the network is unavailable,
//! the API is blocked, or the URL is invalid, the functions return empty
//! strings / `None` so the conversation flow is never interrupted.

use anyhow::{Context, Result};

// ── Public API ───────────────────────────────────────────────────────────

/// Detect a tweet/X status URL in `text` and return the URL (including
/// `https://` prefix) if found.
///
/// Supports:
/// - `https://twitter.com/username/status/12345`
/// - `https://x.com/username/status/12345`
/// - `https://mobile.twitter.com/username/status/12345`
/// - Bare `x.com/username/status/12345` (no protocol)
/// - URLs with query params, fragments, trailing slashes
///
/// Returns `None` for profile URLs (no `/status/` segment), non-tweet
/// domains, or input with no tweet URL.
pub fn detect_tweet_url(text: &str) -> Option<String> {
    // Find the first occurrence of /status/ marker.
    let (start_idx, marker) = text
        .match_indices("/status/")
        .find_map(|(idx, m)| {
            // Verify the URL domain (twitter.com / x.com / mobile.twitter.com)
            let before = &text[..idx];
            // Check if any of the allowed domains appear right before the marker
            let url_start = before
                .rfind("https://twitter.com")
                .or(before.rfind("https://x.com"))
                .or(before.rfind("https://mobile.twitter.com"))
                .or(before.rfind("twitter.com"))
                .or(before.rfind("x.com"))
                .or(before.rfind("mobile.twitter.com"));
            let has_known_domain = url_start.is_some()
                // Also check if the URL starts right at the beginning or after whitespace
                || before.is_empty()
                || before.ends_with(char::is_whitespace)
                || before.ends_with('(');
            if has_known_domain {
                Some((idx, m))
            } else {
                None
            }
        })?;

    // Walk backwards from the marker to find the start of the URL.
    let text_before = &text[..start_idx];
    let url_start = text_before
        .rfind("https://")
        .or(text_before.rfind("http://"))
        .or_else(|| {
            // No protocol — the URL may be bare (e.g. "x.com/...").  Use the
            // domain start as the URL start.  Find "twitter.com", "x.com", or
            // "mobile.twitter.com" nearest to the marker.
            text_before.rfind("twitter.com")
                .or(text_before.rfind("x.com"))
                .or(text_before.rfind("mobile.twitter.com"))
        });

    // Determine the actual URL start position.
    let (actual_start, needs_scheme) = match url_start {
        Some(pos) if text[pos..].starts_with("https://") || text[pos..].starts_with("http://") => {
            (pos, false)
        }
        Some(pos) => (pos, true), // found domain but no protocol
        None => (start_idx, true), // marker itself is the start
    };

    // Find the end of the URL: stop at whitespace, closing paren, angle bracket, or end.
    let end_idx = text[marker.len() + start_idx..]
        .find(|c: char| c.is_whitespace() || c == ')' || c == '>' || c == ']')
        .map(|rel| marker.len() + start_idx + rel)
        .unwrap_or(text.len());

    let raw_url = &text[actual_start..end_idx];

    // Build the canonical URL with scheme.
    if needs_scheme {
        Some(format!("https://{}", raw_url))
    } else {
        Some(raw_url.to_string())
    }
}

/// Fetch tweet content from the Twitter oEmbed API and return a formatted
/// context string (e.g. `"[引用推文 - 作者: @NASA]: ..."`).
///
/// Returns an empty string on any error (network timeout, blocked domain,
/// parse failure) so callers can safely append the result without special
/// error handling.
pub async fn fetch_tweet_context(url: &str) -> String {
    fetch_tweet_context_inner(url).await.unwrap_or_default()
}

// ── Internal implementation ──────────────────────────────────────────────

async fn fetch_tweet_context_inner(url: &str) -> Result<String> {
    let encoded = urlencoding(url);
    let api_url = format!("https://publish.twitter.com/oembed?url={}", encoded);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(
            "Mozilla/5.0 (compatible; VaultPilot/1.0; +https://github.com/ryanloee/VaultPilot)",
        )
        .build()
        .context("failed to build HTTP client")?;

    let resp = client
        .get(&api_url)
        .send()
        .await
        .context("oEmbed request failed")?;

    let status = resp.status();
    if !status.is_success() {
        // Non-200 — likely a bad URL or rate limit.  Quietly return empty.
        return Ok(String::new());
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .context("failed to parse oEmbed JSON")?;

    let author_name = body["author_name"].as_str().unwrap_or("unknown");
    let author_url = body["author_url"].as_str().unwrap_or("");
    let html = body["html"].as_str().unwrap_or("");

    let handle = extract_handle(author_url, author_name);
    let text_content = strip_html_oembed(html);

    Ok(format!(
        "\n\n[引用推文 - 作者: {}]: {}",
        handle, text_content
    ))
}

/// Strip HTML tags from the oEmbed `html` field to produce plain text.
/// Uses a simple state machine (same pattern as `deep_research.rs`).
fn strip_html_oembed(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {} // inside tag, skip
        }
    }
    // Decode common HTML entities for readability
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}

/// Extract the Twitter handle from `author_url` (e.g.
/// `https://twitter.com/NASA` → `@NASA`).  Falls back to `display_name`
/// if the URL is not a twitter/x.com domain.
fn extract_handle(author_url: &str, display_name: &str) -> String {
    for prefix in &[
        "https://twitter.com/",
        "https://x.com/",
        "https://mobile.twitter.com/",
        "http://twitter.com/",
        "http://x.com/",
    ] {
        if let Some(handle) = author_url.strip_prefix(prefix) {
            let handle = handle.trim_end_matches('/');
            if !handle.is_empty() {
                return format!("@{}", handle);
            }
        }
    }
    display_name.to_string()
}

/// Simple percent-encoding for query parameters.
fn urlencoding(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    out
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── URL detection ────────────────────────────────────────────────

    #[test]
    fn test_detect_standard_twitter_url() {
        let url = detect_tweet_url("https://twitter.com/user/status/123456789");
        assert_eq!(url.as_deref(), Some("https://twitter.com/user/status/123456789"));
    }

    #[test]
    fn test_detect_x_url() {
        let url = detect_tweet_url("https://x.com/user/status/123456789");
        assert_eq!(url.as_deref(), Some("https://x.com/user/status/123456789"));
    }

    #[test]
    fn test_detect_mobile_twitter_url() {
        let url = detect_tweet_url("https://mobile.twitter.com/user/status/123456789");
        assert_eq!(url.as_deref(), Some("https://mobile.twitter.com/user/status/123456789"));
    }

    #[test]
    fn test_detect_bare_url_without_protocol() {
        let url = detect_tweet_url("x.com/user/status/123456789");
        assert_eq!(url.as_deref(), Some("https://x.com/user/status/123456789"));
    }

    #[test]
    fn test_detect_url_with_query_params() {
        let text = "Check this: https://twitter.com/user/status/123?lang=en";
        let url = detect_tweet_url(text);
        assert_eq!(url.as_deref(), Some("https://twitter.com/user/status/123?lang=en"));
    }

    #[test]
    fn test_detect_url_with_trailing_text() {
        let text = "see https://x.com/a/status/1 and more";
        let url = detect_tweet_url(text);
        assert_eq!(url.as_deref(), Some("https://x.com/a/status/1"));
    }

    #[test]
    fn test_detect_url_with_fragment() {
        let text = "https://twitter.com/u/status/1#my-fragment";
        let url = detect_tweet_url(text);
        assert_eq!(url.as_deref(), Some("https://twitter.com/u/status/1#my-fragment"));
    }

    #[test]
    fn test_url_with_trailing_slash() {
        let url = detect_tweet_url("https://twitter.com/u/status/1/");
        assert_eq!(url.as_deref(), Some("https://twitter.com/u/status/1/"));
    }

    #[test]
    fn test_detect_url_in_text() {
        let text = "I just saw this tweet https://twitter.com/NASA/status/1672518474145062912 what do you think?";
        let url = detect_tweet_url(text);
        assert_eq!(
            url.as_deref(),
            Some("https://twitter.com/NASA/status/1672518474145062912")
        );
    }

    #[test]
    fn test_detect_url_in_parentheses() {
        let text = "(https://twitter.com/u/status/1)";
        let url = detect_tweet_url(text);
        assert_eq!(url.as_deref(), Some("https://twitter.com/u/status/1"));
    }

    #[test]
    fn test_non_tweet_url_not_detected() {
        assert!(detect_tweet_url("https://example.com/page").is_none());
        assert!(detect_tweet_url("https://github.com/user/repo").is_none());
        assert!(detect_tweet_url("no url here").is_none());
        assert!(detect_tweet_url("").is_none());
    }

    #[test]
    fn test_twitter_profile_not_detected() {
        // Profile URLs don't contain /status/
        assert!(detect_tweet_url("https://twitter.com/NASA").is_none());
        assert!(detect_tweet_url("https://x.com/user").is_none());
    }

    #[test]
    fn test_detect_first_url_when_multiple() {
        let text = "tweet1: https://x.com/a/status/1 and tweet2: https://x.com/b/status/2";
        let url = detect_tweet_url(text);
        // Should return the first match
        assert_eq!(url.as_deref(), Some("https://x.com/a/status/1"));
    }

    // ── HTML stripping ───────────────────────────────────────────────

    #[test]
    fn test_strip_html_simple() {
        assert_eq!(strip_html_oembed("<p>Hello</p>"), "Hello");
    }

    #[test]
    fn test_strip_html_nested() {
        let html = "<blockquote><p>Hello <b>world</b></p></blockquote>";
        assert_eq!(strip_html_oembed(html), "Hello world");
    }

    #[test]
    fn test_strip_html_no_html() {
        assert_eq!(strip_html_oembed("plain text"), "plain text");
    }

    #[test]
    fn test_strip_html_empty() {
        assert_eq!(strip_html_oembed(""), "");
    }

    #[test]
    fn test_strip_html_entities() {
        let html = "<p>AT&amp;T &lt;good&gt;</p>";
        assert_eq!(strip_html_oembed(html), "AT&T <good>");
    }

    // ── Handle extraction ────────────────────────────────────────────

    #[test]
    fn test_extract_handle_from_url() {
        assert_eq!(
            extract_handle("https://twitter.com/NASA", "NASA"),
            "@NASA"
        );
    }

    #[test]
    fn test_extract_handle_from_x_url() {
        assert_eq!(
            extract_handle("https://x.com/elonmusk", "Elon Musk"),
            "@elonmusk"
        );
    }

    #[test]
    fn test_extract_handle_with_trailing_slash() {
        assert_eq!(
            extract_handle("https://twitter.com/user/", "User"),
            "@user"
        );
    }

    #[test]
    fn test_extract_handle_fallback_to_name() {
        assert_eq!(
            extract_handle("https://example.com/profile", "Display Name"),
            "Display Name"
        );
    }

    #[test]
    fn test_extract_handle_empty_fallback() {
        assert_eq!(extract_handle("", ""), "");
    }

    // ── URL encoding ─────────────────────────────────────────────────

    #[test]
    fn test_urlencoding_basic() {
        assert_eq!(urlencoding("hello"), "hello");
    }

    #[test]
    fn test_urlencoding_special_chars() {
        assert_eq!(urlencoding("a b"), "a%20b");
        assert_eq!(urlencoding("https://x.com/user/status/1"), "https%3A%2F%2Fx.com%2Fuser%2Fstatus%2F1");
    }
}
