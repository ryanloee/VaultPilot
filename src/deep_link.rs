//! `vaultpilot://` deep-link route parser (#3728).
//!
//! VaultPilot's three front-ends (WinUI, Mobile, CLI) all need to interpret
//! `vaultpilot://` URIs that arrive from external automation sources
//! (browser widgets, Quick Settings tiles, Alfred / Raycast, iOS Shortcuts,
//! x-callback-url flows).  Previously the only routing logic lived in the
//! mobile app's React Navigation config (`mobile/App.tsx`), which caused
//! inconsistency bugs like #3156 ("one platform has a route, another
//! doesn't").
//!
//! This module provides the **single source of truth** for URI parsing in
//! Rust core.  Every front-end can hand a raw URI to [`parse_deep_link`] and
//! receive a structured [`DeepLinkAction`], guaranteeing identical behaviour
//! across platforms.
//!
//! ## Supported routes
//!
//! | Route | Action |
//! |-------|--------|
//! | `note/new[?params]` | Create a new note (+ optional content / clipboard) |
//! | `note/<id>` / `note/open/<id>` | Open an existing note |
//! | `daily` | Create or open today's daily note |
//! | `chat/new` | Start a new chat session |
//! | `search[?query=...]` | Open global search, optionally prefilled |
//! | `settings` | Open settings |
//!
//! ## Rich action parameters (Obsidian `new` parity)
//!
//! `vaultpilot://note/new` accepts query parameters:
//! - `name` — note title
//! - `content` — initial body text
//! - `clipboard=1` — paste the clipboard as content (overrides `content`)
//! - `append=1` / `prepend=1` — append/prepend to an existing note
//! - `silent=1` — do not navigate to the note after creation
//! - `overwrite=1` — replace an existing note with the same name
//!
//! ## x-callback-url
//!
//! Every route accepts the standard x-callback-url parameters, captured in
//! [`XCallback`]:
//! - `x-success` — URL to open on success
//! - `x-error` — URL to open on failure
//! - `x-source` — human-readable name of the calling app
//!
//! These enable integration with Hook, Alfred, Raycast, and iOS Shortcuts.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A parsed `vaultpilot://` deep-link action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum DeepLinkAction {
    /// `vaultpilot://note/new[?params]` — create a new note.
    NewNote {
        #[serde(flatten)]
        params: NewNoteParams,
        #[serde(flatten)]
        xcallback: XCallback,
    },
    /// `vaultpilot://note/<id>` — open an existing note.
    OpenNote {
        note_id: String,
        #[serde(flatten)]
        xcallback: XCallback,
    },
    /// `vaultpilot://daily` — create or open today's daily note.
    Daily {
        #[serde(flatten)]
        xcallback: XCallback,
    },
    /// `vaultpilot://chat/new` — start a new chat session.
    NewChat {
        #[serde(flatten)]
        xcallback: XCallback,
    },
    /// `vaultpilot://search[?query=...]` — open global search.
    Search {
        query: Option<String>,
        #[serde(flatten)]
        xcallback: XCallback,
    },
    /// `vaultpilot://settings` — open settings.
    Settings {
        #[serde(flatten)]
        xcallback: XCallback,
    },
    /// An unrecognised route — the raw path is preserved so the caller can
    /// decide whether to ignore it or show a diagnostic.
    Unknown {
        raw: String,
        #[serde(flatten)]
        xcallback: XCallback,
    },
}

/// Parameters for the `note/new` action (parity with Obsidian's `new` verb).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewNoteParams {
    /// Note title / file name.
    pub name: Option<String>,
    /// Initial body text.
    pub content: Option<String>,
    /// Paste the clipboard as content (overrides `content`).
    pub clipboard: bool,
    /// Append the content to an existing note instead of creating a new one.
    pub append: bool,
    /// Prepend the content to an existing note.
    pub prepend: bool,
    /// Do not navigate to the note after creation.
    pub silent: bool,
    /// Replace an existing note that shares the same name/path.
    pub overwrite: bool,
}

/// x-callback-url parameters (any route may carry these).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XCallback {
    /// URL to open on success.
    pub x_success: Option<String>,
    /// URL to open on failure.
    pub x_error: Option<String>,
    /// Human-readable name of the calling app.
    pub x_source: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// The URI scheme VaultPilot registers.
pub const SCHEME: &str = "vaultpilot";

/// Parse a `vaultpilot://...` URI into a structured [`DeepLinkAction`].
///
/// Returns [`DeepLinkAction::Unknown`] for URIs that do not match a known
/// route (rather than an error), so callers can simply ignore unsupported
/// routes.  Query parameters are always parsed, even for unknown routes, so
/// that x-callback-url flows can still fire an `x-error` callback.
///
/// # Examples
/// ```
/// use vaultpilot_lib::deep_link::{parse_deep_link, DeepLinkAction};
///
/// let action = parse_deep_link("vaultpilot://note/abc-123");
/// assert_eq!(action, DeepLinkAction::OpenNote {
///     note_id: "abc-123".into(),
///     xcallback: Default::default(),
/// });
///
/// let action = parse_deep_link("vaultpilot://search?query=rust");
/// match action {
///     DeepLinkAction::Search { query, .. } => assert_eq!(query.as_deref(), Some("rust")),
///     _ => panic!("expected Search"),
/// }
/// ```
pub fn parse_deep_link(uri: &str) -> DeepLinkAction {
    // Strip the scheme + authority: `vaultpilot://note/new` → `note/new`.
    let path_and_query = match strip_scheme(uri) {
        Some(rest) => rest,
        None => {
            // Not a vaultpilot:// URI at all.
            return DeepLinkAction::Unknown {
                raw: uri.to_string(),
                xcallback: XCallback::default(),
            };
        }
    };

    // Split path from query string.
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, q),
        None => (path_and_query.as_str(), ""),
    };

    let xcallback = parse_xcallback(query);
    let params = parse_new_note_params(query);

    // Normalise the path: match route keywords case-insensitively (#3734) but
    // extract the note id from the original segments (ids can be mixed-case),
    // then percent-decode the id so encoded characters resolve correctly (#3735).
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let lower: Vec<String> = segments.iter().map(|s| s.to_ascii_lowercase()).collect();
    let lower_refs: Vec<&str> = lower.iter().map(|s| s.as_str()).collect();

    match lower_refs.as_slice() {
        ["note", "new"] => DeepLinkAction::NewNote { params, xcallback },
        ["note", _] => DeepLinkAction::OpenNote {
            note_id: url_decode(segments[1]),
            xcallback,
        },
        ["note", "open", _] => DeepLinkAction::OpenNote {
            note_id: url_decode(segments[2]),
            xcallback,
        },
        ["daily"] => DeepLinkAction::Daily { xcallback },
        ["chat", "new"] => DeepLinkAction::NewChat { xcallback },
        ["search"] => DeepLinkAction::Search {
            query: parse_query_value(query, "query"),
            xcallback,
        },
        ["settings"] => DeepLinkAction::Settings { xcallback },
        _ => DeepLinkAction::Unknown {
            raw: path.to_string(),
            xcallback,
        },
    }
}

/// Check whether a URI string uses the `vaultpilot://` scheme.
pub fn is_vaultpilot_uri(uri: &str) -> bool {
    strip_scheme(uri).is_some()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Strip the `vaultpilot://` prefix (case-insensitive on the scheme),
/// returning the remainder (`note/new?x=1`).
fn strip_scheme(uri: &str) -> Option<String> {
    let lower_prefix = format!("{SCHEME}://");
    if uri.to_ascii_lowercase().starts_with(&lower_prefix) {
        Some(uri[lower_prefix.len()..].to_string())
    } else {
        None
    }
}

/// URL-decode a percent-encoded value (the subset used in query strings).
fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = hex_pair(bytes[i + 1], bytes[i + 2]) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_pair(hi: u8, lo: u8) -> Option<u8> {
    Some((hex_digit(hi)? << 4) | hex_digit(lo)?)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parse a query string (`a=1&b=2`) into a list of (key, decoded value) pairs.
fn parse_query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((url_decode(k), url_decode(v)))
        })
        .collect()
}

/// Return the decoded value of a single query parameter (first occurrence).
fn parse_query_value(query: &str, key: &str) -> Option<String> {
    parse_query_pairs(query)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

/// Parse x-callback-url parameters from the query string.
fn parse_xcallback(query: &str) -> XCallback {
    let pairs = parse_query_pairs(query);
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
    };
    XCallback {
        x_success: get("x-success"),
        x_error: get("x-error"),
        x_source: get("x-source"),
    }
}

/// Parse [`NewNoteParams`] from the query string.
fn parse_new_note_params(query: &str) -> NewNoteParams {
    let pairs = parse_query_pairs(query);
    let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    let flag = |key: &str| -> bool { pairs.iter().any(|(k, v)| k == key && is_truthy(v)) };
    NewNoteParams {
        name: get("name"),
        content: get("content"),
        clipboard: flag("clipboard"),
        append: flag("append"),
        prepend: flag("prepend"),
        silent: flag("silent"),
        overwrite: flag("overwrite"),
    }
}

/// Treat `1`/`true`/`yes` (case-insensitive) as a truthy flag value.
fn is_truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_open_note() {
        let action = parse_deep_link("vaultpilot://note/abc-123");
        assert_eq!(
            action,
            DeepLinkAction::OpenNote {
                note_id: "abc-123".into(),
                xcallback: XCallback::default(),
            }
        );
    }

    #[test]
    fn test_parse_open_note_explicit_open() {
        // `note/open/<id>` is an explicit alias for `note/<id>`.
        let action = parse_deep_link("vaultpilot://note/open/xyz");
        assert_eq!(
            action,
            DeepLinkAction::OpenNote {
                note_id: "xyz".into(),
                xcallback: XCallback::default(),
            }
        );
    }

    #[test]
    fn test_parse_new_note_simple() {
        let action = parse_deep_link("vaultpilot://note/new");
        assert_eq!(
            action,
            DeepLinkAction::NewNote {
                params: NewNoteParams::default(),
                xcallback: XCallback::default(),
            }
        );
    }

    #[test]
    fn test_parse_new_note_with_params() {
        let action = parse_deep_link(
            "vaultpilot://note/new?name=My%20Note&content=Hello%20world&silent=1&append=true",
        );
        match action {
            DeepLinkAction::NewNote { params, .. } => {
                assert_eq!(params.name.as_deref(), Some("My Note"));
                assert_eq!(params.content.as_deref(), Some("Hello world"));
                assert!(params.silent);
                assert!(params.append);
                assert!(!params.clipboard);
                assert!(!params.overwrite);
            }
            other => panic!("expected NewNote, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_new_note_all_flags() {
        let action = parse_deep_link(
            "vaultpilot://note/new?clipboard=1&append=1&prepend=1&overwrite=1&silent=yes",
        );
        match action {
            DeepLinkAction::NewNote { params, .. } => {
                assert!(params.clipboard);
                assert!(params.append);
                assert!(params.prepend);
                assert!(params.overwrite);
                assert!(params.silent);
            }
            other => panic!("expected NewNote, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_flag_truthy_values() {
        for val in ["1", "true", "TRUE", "Yes", "on"] {
            assert!(is_truthy(val), "{val:?} should be truthy");
        }
        for val in ["0", "false", "no", "", "maybe"] {
            assert!(!is_truthy(val), "{val:?} should be falsy");
        }
    }

    #[test]
    fn test_parse_daily() {
        let action = parse_deep_link("vaultpilot://daily");
        assert_eq!(
            action,
            DeepLinkAction::Daily {
                xcallback: XCallback::default(),
            }
        );
    }

    #[test]
    fn test_parse_new_chat() {
        let action = parse_deep_link("vaultpilot://chat/new");
        assert_eq!(
            action,
            DeepLinkAction::NewChat {
                xcallback: XCallback::default(),
            }
        );
    }

    #[test]
    fn test_parse_search_without_query() {
        let action = parse_deep_link("vaultpilot://search");
        assert_eq!(
            action,
            DeepLinkAction::Search {
                query: None,
                xcallback: XCallback::default(),
            }
        );
    }

    #[test]
    fn test_parse_search_with_query() {
        let action = parse_deep_link("vaultpilot://search?query=rust%20async");
        assert_eq!(
            action,
            DeepLinkAction::Search {
                query: Some("rust async".into()),
                xcallback: XCallback::default(),
            }
        );
    }

    #[test]
    fn test_parse_settings() {
        assert!(matches!(
            parse_deep_link("vaultpilot://settings"),
            DeepLinkAction::Settings { .. }
        ));
    }

    #[test]
    fn test_parse_xcallback() {
        let action = parse_deep_link(
            "vaultpilot://note/new?x-success=https://x.com&x-error=https://e.com&x-source=Alfred",
        );
        match action {
            DeepLinkAction::NewNote { xcallback, .. } => {
                assert_eq!(xcallback.x_success.as_deref(), Some("https://x.com"));
                assert_eq!(xcallback.x_error.as_deref(), Some("https://e.com"));
                assert_eq!(xcallback.x_source.as_deref(), Some("Alfred"));
            }
            other => panic!("expected NewNote, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_xcallback_on_any_route() {
        // x-callback params work even on routes like `search`.
        let action = parse_deep_link("vaultpilot://search?query=hi&x-source=Raycast");
        match action {
            DeepLinkAction::Search { query, xcallback } => {
                assert_eq!(query.as_deref(), Some("hi"));
                assert_eq!(xcallback.x_source.as_deref(), Some("Raycast"));
            }
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_unknown_route() {
        let action = parse_deep_link("vaultpilot://nonsense/foo");
        match action {
            DeepLinkAction::Unknown { raw, .. } => assert_eq!(raw, "nonsense/foo"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_non_vaultpilot_scheme() {
        let action = parse_deep_link("https://example.com/note/new");
        assert!(matches!(action, DeepLinkAction::Unknown { .. }));
    }

    #[test]
    fn test_parse_case_insensitive_scheme() {
        // Scheme matching is case-insensitive per RFC 3986.
        let action = parse_deep_link("VAULTPILOT://search?query=x");
        assert!(matches!(action, DeepLinkAction::Search { .. }));
    }

    // --- Regression tests for #3734: case-insensitive route segments ---

    #[test]
    fn test_parse_case_insensitive_route_search() {
        // `vaultpilot://Search` should match Search, not Unknown (#3734).
        let action = parse_deep_link("vaultpilot://Search?query=x");
        match action {
            DeepLinkAction::Search { query, .. } => {
                assert_eq!(query.as_deref(), Some("x"));
            }
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_case_insensitive_route_daily() {
        let action = parse_deep_link("vaultpilot://Daily");
        assert!(matches!(action, DeepLinkAction::Daily { .. }));
    }

    #[test]
    fn test_parse_case_insensitive_route_settings() {
        let action = parse_deep_link("vaultpilot://Settings");
        assert!(matches!(action, DeepLinkAction::Settings { .. }));
    }

    #[test]
    fn test_parse_case_insensitive_route_note_new() {
        let action = parse_deep_link("vaultpilot://NOTE/New");
        assert!(matches!(action, DeepLinkAction::NewNote { .. }));
    }

    #[test]
    fn test_parse_case_insensitive_route_chat_new() {
        let action = parse_deep_link("vaultpilot://CHAT/NEW");
        assert!(matches!(action, DeepLinkAction::NewChat { .. }));
    }

    #[test]
    fn test_parse_case_insensitive_note_open_keyword() {
        // `note/OPEN/id` — the "open" keyword is case-insensitive.
        let action = parse_deep_link("vaultpilot://note/OPEN/MyNote");
        match action {
            DeepLinkAction::OpenNote { note_id, .. } => {
                assert_eq!(note_id, "MyNote");
            }
            other => panic!("expected OpenNote, got {other:?}"),
        }
    }

    // --- Regression tests for #3735: percent-decode note ID ---

    #[test]
    fn test_parse_open_note_percent_decoded_space() {
        // `vaultpilot://note/my%20note` → "my note" (#3735).
        let action = parse_deep_link("vaultpilot://note/my%20note");
        match action {
            DeepLinkAction::OpenNote { note_id, .. } => {
                assert_eq!(note_id, "my note");
            }
            other => panic!("expected OpenNote, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_open_note_percent_decoded_unicode() {
        // `vaultpilot://note/caf%C3%A9` → "café" (#3735).
        let action = parse_deep_link("vaultpilot://note/caf%C3%A9");
        match action {
            DeepLinkAction::OpenNote { note_id, .. } => {
                assert_eq!(note_id, "café");
            }
            other => panic!("expected OpenNote, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_open_note_explicit_open_percent_decoded() {
        // Percent-decoding also applies to `note/open/<id>` form.
        let action = parse_deep_link("vaultpilot://note/open/project%20alpha");
        match action {
            DeepLinkAction::OpenNote { note_id, .. } => {
                assert_eq!(note_id, "project alpha");
            }
            other => panic!("expected OpenNote, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_open_note_preserves_mixed_case_id() {
        // Mixed-case note IDs are preserved even with case-insensitive
        // route matching (#3734 + #3735 combined).
        let action = parse_deep_link("vaultpilot://Note/MyCamelCaseID");
        match action {
            DeepLinkAction::OpenNote { note_id, .. } => {
                assert_eq!(note_id, "MyCamelCaseID");
            }
            other => panic!("expected OpenNote, got {other:?}"),
        }
    }

    #[test]
    fn test_is_vaultpilot_uri() {
        assert!(is_vaultpilot_uri("vaultpilot://note/new"));
        assert!(is_vaultpilot_uri("VAULTPILOT://daily"));
        assert!(!is_vaultpilot_uri("https://vaultpilot.com"));
        assert!(!is_vaultpilot_uri("obsidian://note/new"));
    }

    #[test]
    fn test_url_decode() {
        assert_eq!(url_decode("Hello%20World"), "Hello World");
        assert_eq!(url_decode("a+b"), "a b");
        assert_eq!(url_decode("100%25"), "100%");
        assert_eq!(url_decode("plain"), "plain");
        // Malformed percent-encoding is left as-is.
        assert_eq!(url_decode("%ZZ"), "%ZZ");
    }

    #[test]
    fn test_trailing_slash_ignored() {
        // A trailing slash should not break route matching.
        assert!(matches!(
            parse_deep_link("vaultpilot://search/"),
            DeepLinkAction::Search { .. }
        ));
    }

    #[test]
    fn test_action_serialization_round_trip() {
        let action = parse_deep_link("vaultpilot://note/new?name=Test&content=Body");
        let json = serde_json::to_string(&action).unwrap();
        let parsed: DeepLinkAction = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, action);
    }

    #[test]
    fn test_backward_compat_existing_routes() {
        // The four routes that existed before #3728 must still parse correctly
        // (parity with mobile/App.tsx linking config).
        assert!(matches!(
            parse_deep_link("vaultpilot://note/new"),
            DeepLinkAction::NewNote { .. }
        ));
        assert!(matches!(
            parse_deep_link("vaultpilot://note/some-id"),
            DeepLinkAction::OpenNote { .. }
        ));
        assert!(matches!(
            parse_deep_link("vaultpilot://chat/new"),
            DeepLinkAction::NewChat { .. }
        ));
        assert!(matches!(
            parse_deep_link("vaultpilot://search"),
            DeepLinkAction::Search { .. }
        ));
    }
}
