//! `vaultpilot://` deep-link route parser (#3728).
//!
//! VaultPilot's three front-ends (WinUI, Mobile, CLI) all need to interpret
//! `vaultpilot://` URIs that arrive from external automation sources
//! (browser widgets, Quick Settings tiles, Alfred / Raycast, iOS Shortcuts,
//! x-callback-url flows).  Previously the only routing logic lived in the
//! mobile app's deep-link handling, which caused
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
//! | `note/delete[?id=<id>]` / `note/delete/<id>` | Delete a note (High risk) |
//! | `note/edit[?id=<id>]` / `note/edit/<id>` | Destructively rewrite a note (High risk) |
//! | `note/bulk?op=...` | Bulk destructive note operation (High risk) |
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

use std::path::{Path, PathBuf};

use anyhow::Result;
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
    /// `vaultpilot://note/delete[?id=<id>]` / `vaultpilot://note/delete/<id>` —
    /// delete a note (irreversible). **High risk.** Headless entry points
    /// (MCP server, HTTP bridge, #3964) map their delete tool here so the
    /// shared risk classifier is the single source of truth.
    DeleteNote {
        /// Target note id (`id=` query param or path segment), when known.
        note_id: Option<String>,
        #[serde(flatten)]
        xcallback: XCallback,
    },
    /// `vaultpilot://note/edit[?id=<id>]` / `vaultpilot://note/edit/<id>` —
    /// destructively rewrite a note's content. **High risk.**
    EditNote {
        /// Target note id (`id=` query param or path segment), when known.
        note_id: Option<String>,
        #[serde(flatten)]
        xcallback: XCallback,
    },
    /// `vaultpilot://note/bulk?op=delete|move|update_tags` — batch destructive
    /// operation over multiple notes (bulk delete / move / retag). **High risk.**
    BulkNoteOp {
        /// The bulk operation name (`delete`, `move`, `update_tags`, ...).
        op: String,
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
        ["note", "delete"] => DeepLinkAction::DeleteNote {
            note_id: parse_query_value(query, "id"),
            xcallback,
        },
        ["note", "edit"] => DeepLinkAction::EditNote {
            note_id: parse_query_value(query, "id"),
            xcallback,
        },
        ["note", "bulk"] => DeepLinkAction::BulkNoteOp {
            op: parse_query_value(query, "op").unwrap_or_default(),
            xcallback,
        },
        ["note", _] => DeepLinkAction::OpenNote {
            note_id: url_decode(segments[1]),
            xcallback,
        },
        ["note", "open", _] => DeepLinkAction::OpenNote {
            note_id: url_decode(segments[2]),
            xcallback,
        },
        ["note", "delete", _] => DeepLinkAction::DeleteNote {
            note_id: Some(url_decode(segments[2])),
            xcallback,
        },
        ["note", "edit", _] => DeepLinkAction::EditNote {
            note_id: Some(url_decode(segments[2])),
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

impl DeepLinkAction {
    /// The x-callback-url parameters carried by this action.
    ///
    /// Every route may carry `x-success` / `x-error` / `x-source` (see
    /// [`XCallback`]); this accessor lets callers (CLI wiring, #3958) read
    /// them without matching on every variant.
    pub fn xcallback(&self) -> &XCallback {
        match self {
            DeepLinkAction::NewNote { xcallback, .. }
            | DeepLinkAction::OpenNote { xcallback, .. }
            | DeepLinkAction::Daily { xcallback }
            | DeepLinkAction::NewChat { xcallback }
            | DeepLinkAction::DeleteNote { xcallback, .. }
            | DeepLinkAction::EditNote { xcallback, .. }
            | DeepLinkAction::BulkNoteOp { xcallback, .. }
            | DeepLinkAction::Search { xcallback, .. }
            | DeepLinkAction::Settings { xcallback }
            | DeepLinkAction::Unknown { xcallback, .. } => xcallback,
        }
    }
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
// #3822 — URI action security confirmation (Obsidian 1.13 parity)
// ---------------------------------------------------------------------------

/// Risk level of a deep-link action, used by [`should_confirm_uri_action`] to
/// decide whether the front-end must show a confirmation dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UriActionRisk {
    /// Low-risk, read-only, or navigational (open note, search, settings).
    /// No confirmation needed unless the user opted into always-confirm mode.
    Low,
    /// Mutating but reversible (create note, append/prepend).
    /// Confirm for untrusted apps; auto-allow for trusted apps.
    Medium,
    /// High-risk / destructive or potentially irreversible (overwrite existing
    /// note, start AI chat which may execute agent tools). **Always** confirm
    /// regardless of trusted status — mirrors Obsidian's "high-risk actions
    /// always require confirmation" policy.
    High,
}

/// Classify the risk of a parsed [`DeepLinkAction`].
///
/// The decision is based purely on the action type + parameters (e.g. an
/// overwrite flag bumps a Medium `new` note to High). It does NOT consider the
/// source app — that is handled separately by the trusted-app list.
pub fn classify_uri_action_risk(action: &DeepLinkAction) -> UriActionRisk {
    match action {
        // Read-only / navigational → Low.
        DeepLinkAction::OpenNote { .. }
        | DeepLinkAction::Daily { .. }
        | DeepLinkAction::Search { .. }
        | DeepLinkAction::Settings { .. } => UriActionRisk::Low,

        // Creating a note is reversible unless overwrite is set → Medium/High.
        DeepLinkAction::NewNote { params, .. } => {
            if params.overwrite {
                UriActionRisk::High
            } else {
                UriActionRisk::Medium
            }
        }

        // Starting an AI chat may trigger agent tool execution → High.
        DeepLinkAction::NewChat { .. } => UriActionRisk::High,

        // Deleting a note, destructively rewriting one, or running a bulk
        // destructive operation is irreversible → High (#3964).
        DeepLinkAction::DeleteNote { .. }
        | DeepLinkAction::EditNote { .. }
        | DeepLinkAction::BulkNoteOp { .. } => UriActionRisk::High,

        // Unknown route — be conservative: confirm (Medium).
        DeepLinkAction::Unknown { .. } => UriActionRisk::Medium,
    }
}

/// The decision returned to the front-end for a deep-link action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UriConfirmationDecision {
    /// Execute the action immediately — no dialog needed.
    /// `reason` explains why (trusted app + low/medium risk).
    Allow { reason: String },
    /// Show a confirmation dialog before executing.
    /// `message` is a human-readable description of the action for the dialog.
    Confirm { message: String },
}

/// A human-readable summary of a deep-link action for confirmation dialogs.
///
/// Examples:
/// - "Create a new note 'My Note'"
/// - "Open note abc-123"
/// - "Open today's daily note"
/// - "Start a new chat session"
/// - "Search vault for 'query'"
/// - "Open settings"
pub fn describe_uri_action(action: &DeepLinkAction) -> String {
    match action {
        DeepLinkAction::NewNote { params, .. } => {
            let title = params
                .name
                .as_deref()
                .map(|n| format!("'{n}'"))
                .unwrap_or_else(|| "without a title".into());
            if params.overwrite {
                format!("Replace existing note {title} (overwrite)")
            } else if params.append {
                format!("Append to note {title}")
            } else if params.prepend {
                format!("Prepend to note {title}")
            } else {
                format!("Create a new note {title}")
            }
        }
        DeepLinkAction::OpenNote { note_id, .. } => {
            format!("Open note '{note_id}'")
        }
        DeepLinkAction::Daily { .. } => "Open today's daily note".to_string(),
        DeepLinkAction::NewChat { .. } => "Start a new AI chat session".to_string(),
        DeepLinkAction::DeleteNote { note_id, .. } => match note_id.as_deref() {
            Some(id) if !id.is_empty() => format!("Delete note '{id}'"),
            _ => "Delete a note".to_string(),
        },
        DeepLinkAction::EditNote { note_id, .. } => match note_id.as_deref() {
            Some(id) if !id.is_empty() => format!("Destructively rewrite note '{id}'"),
            _ => "Destructively rewrite a note".to_string(),
        },
        DeepLinkAction::BulkNoteOp { op, .. } => {
            format!("Run bulk note operation '{op}'")
        }
        DeepLinkAction::Search { query, .. } => match query {
            Some(q) if !q.is_empty() => format!("Search vault for '{q}'"),
            _ => "Open search".to_string(),
        },
        DeepLinkAction::Settings { .. } => "Open settings".to_string(),
        DeepLinkAction::Unknown { raw, .. } => format!("Execute unknown action: {raw}"),
    }
}

/// The trusted-app list — persisted to `.vaultpilot/trusted_apps.yaml`.
///
/// Apps added via the "Don't ask again" checkbox in the confirmation dialog.
/// Trusted apps bypass confirmation for Low and Medium risk actions; High risk
/// actions **always** require confirmation regardless of trust.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustedAppRegistry {
    /// Lowercased source-app names that the user has trusted.
    #[serde(default)]
    pub trusted_apps: std::collections::HashSet<String>,
}

impl TrustedAppRegistry {
    /// Load the registry from `<vault_dir>/.vaultpilot/trusted_apps.yaml`.
    /// Returns an empty registry if the file does not exist (first run).
    pub fn load(vault_dir: &Path) -> Self {
        let path = trusted_apps_path(vault_dir);
        match std::fs::read_to_string(&path) {
            Ok(contents) => serde_yaml_ng::from_str(&contents).unwrap_or_default(),
            Err(_) => TrustedAppRegistry::default(),
        }
    }

    /// Persist the registry to `<vault_dir>/.vaultpilot/trusted_apps.yaml`.
    ///
    /// Writes through the hardened [`crate::storage::atomic_write`] path (temp
    /// file + 0600-before-write + atomic rename), consistent with the project's
    /// permission policy for sensitive files under `.vaultpilot/` (#186).
    /// Previously this used a plain `std::fs::write`, which left the file
    /// world-readable (0644) — the registry records the user's app-trust
    /// decisions and deserves the same 0600 treatment as settings (#3959).
    pub fn save(&self, vault_dir: &Path) -> Result<()> {
        let dir = vault_dir.join(".vaultpilot");
        std::fs::create_dir_all(&dir)?;
        let yaml = serde_yaml_ng::to_string(self)?;
        crate::storage::atomic_write(&trusted_apps_path(vault_dir), yaml.as_bytes())?;
        Ok(())
    }

    /// Returns true if `source` (case-insensitive) is in the trusted list.
    /// An empty/missing source is never trusted.
    pub fn is_trusted(&self, source: &str) -> bool {
        let s = source.trim().to_ascii_lowercase();
        !s.is_empty() && self.trusted_apps.contains(&s)
    }

    /// Add a source app to the trusted list (case-insensitive).
    pub fn trust(&mut self, source: &str) {
        let s = source.trim().to_ascii_lowercase();
        if !s.is_empty() {
            self.trusted_apps.insert(s);
        }
    }

    /// Remove a source app from the trusted list (case-insensitive).
    pub fn revoke(&mut self, source: &str) {
        self.trusted_apps
            .remove(&source.trim().to_ascii_lowercase());
    }
}

/// Path to the trusted-apps persistence file inside the vault.
fn trusted_apps_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(".vaultpilot").join("trusted_apps.yaml")
}

/// Decide whether a deep-link action requires a confirmation dialog.
///
/// Returns [`UriConfirmationDecision::Allow`] when:
/// - The action is Low risk (read-only), OR
/// - The action is Medium risk AND the source app is trusted.
///
/// Returns [`UriConfirmationDecision::Confirm`] otherwise (untrusted app,
/// High risk action, or unknown source).
///
/// `source` is the `x-source` parameter from the URI (the calling app name),
/// or an empty string if not provided.
pub fn should_confirm_uri_action(
    action: &DeepLinkAction,
    source: &str,
    trusted: &TrustedAppRegistry,
) -> UriConfirmationDecision {
    let risk = classify_uri_action_risk(action);
    let is_trusted = trusted.is_trusted(source);
    let description = describe_uri_action(action);

    match risk {
        UriActionRisk::Low => {
            // Read-only actions are always safe to execute.
            UriConfirmationDecision::Allow {
                reason: format!("read-only action: {description}"),
            }
        }
        UriActionRisk::Medium if is_trusted => {
            // Mutating but reversible, from a trusted app — skip confirmation.
            UriConfirmationDecision::Allow {
                reason: format!("trusted app '{source}': {description}"),
            }
        }
        UriActionRisk::Medium | UriActionRisk::High => {
            // Either Medium risk + untrusted, or High risk (always confirm).
            let source_label = if source.trim().is_empty() {
                "an external app"
            } else {
                source
            };
            let message = if risk == UriActionRisk::High {
                format!(
                    "{source_label} wants to: {description}\n\n\
                     This is a high-risk action and always requires confirmation."
                )
            } else {
                format!("{source_label} wants to: {description}")
            };
            UriConfirmationDecision::Confirm { message }
        }
    }
}

// ---------------------------------------------------------------------------
// #3964 — non-interactive gate for headless automation entry points
// ---------------------------------------------------------------------------

/// Non-interactive risk gate for headless entry points (#3964).
///
/// The MCP server and the HTTP bridge cannot show a confirmation dialog, so
/// they use this stricter variant of [`should_confirm_uri_action`]:
///
/// - **Low** risk actions: always allowed (read-only / navigational).
/// - **Medium** risk actions: allowed only when `source` is a trusted app
///   (see [`TrustedAppRegistry`]).
/// - **High** risk actions: **always denied**, even for trusted apps — a
///   headless server cannot present the confirmation dialog that high-risk
///   actions require (mirrors the always-confirm policy for High risk).
///
/// Returns `Ok(())` when the action may proceed, or `Err(message)` with a
/// human-readable explanation of the denial.
///
/// `source` is the calling app name (MCP `clientInfo.name`, HTTP
/// `x-vaultpilot-source` header, x-callback `x-source`); an empty source is
/// treated as untrusted.
pub fn should_allow_non_interactive(
    action: &DeepLinkAction,
    source: &str,
    trusted: &TrustedAppRegistry,
) -> Result<(), String> {
    let risk = classify_uri_action_risk(action);
    let description = describe_uri_action(action);
    should_allow_risk_non_interactive(risk, &description, source, trusted)
}

/// Gating classification for a headless automation tool call (#3964).
///
/// The MCP server and HTTP bridge cannot show a confirmation dialog, so every
/// mutating/destructive tool they expose is classified here. The struct pairs
/// the closest `vaultpilot://` deep-link route (used for parsing/description)
/// with the **effective risk class** for the tool call — the two can differ
/// when the tool's runtime behavior is safer than the raw URI route (see
/// [`automation_tool_gate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutomationToolGate {
    /// Closest `vaultpilot://` deep-link route (for description/parsing).
    pub uri: &'static str,
    /// Effective risk class governing the tool call.
    pub risk: UriActionRisk,
    /// Human-readable description of what the tool does (for denial messages).
    pub description: &'static str,
}

/// Map a headless automation tool call to its gate classification (#3964).
///
/// The MCP server and the HTTP bridge gate their write/delete tools through
/// this table so [`classify_uri_action_risk`] stays the single source of
/// truth for URI routes, while per-tool overrides capture tools whose runtime
/// behavior differs from the closest URI:
///
/// - `notes.preview_edit` is **read-only** (`saved:false`, no storage calls)
///   → ungated (`None`), exactly like `notes.search` (#3992).
/// - `notes.apply_edit` records a pre-edit backup and can be undone with
///   `vaultpilot revert-edit` → *reversible* → **Medium** (trusted-only)
///   rather than High, so trusted MCP clients can drive the documented
///   preview → apply workflow (#3992).
/// - HTTP subscription endpoints and the AI action endpoint mutate app data /
///   vault notes → **Medium** (trusted-only), closing the token-only bypass
///   that `http_create_note` already closed (#3993).
///
/// Tools without a direct deep-link route map to the closest equivalent
/// (`notes.delete` → `note/delete`, destructive edits → `note/edit`, bulk
/// operations → `note/bulk?op=...`). Returns `None` for tools that are not
/// gated (read-only / navigational operations / audited exemptions such as
/// `http_chat_completions`, #3993).
pub fn automation_tool_gate(tool: &str) -> Option<AutomationToolGate> {
    let (uri, risk, description) = match tool {
        // Medium — creating notes / subscription records (reversible).
        "notes.create" | "http_create_note" | "http_clip_url" | "notes.import"
        | "http_import_folder" => (
            "vaultpilot://note/new",
            UriActionRisk::Medium,
            "create a note",
        ),
        "http_create_subscription"
        | "http_update_subscription"
        | "http_toggle_subscription"
        | "http_delete_subscription" => (
            "vaultpilot://note/new",
            UriActionRisk::Medium,
            "manage a subscription",
        ),
        // Medium — running a subscription writes a research note into the
        // vault (reversible, like creating a note) (#3993).
        "http_run_subscription" => (
            "vaultpilot://note/new",
            UriActionRisk::Medium,
            "run a subscription and save its result as a note",
        ),
        // Medium — AI actions may rewrite a note when note_id is supplied;
        // keep parity with http_create_note's trusted-source bar (#3993).
        "http_ai_action" => (
            "vaultpilot://note/edit",
            UriActionRisk::Medium,
            "run an AI action on a note",
        ),
        // High — deleting notes (irreversible).
        "notes.delete" | "http_delete_note" | "http_bulk_delete_notes" => (
            "vaultpilot://note/delete",
            UriActionRisk::High,
            "delete notes",
        ),
        // #3992 — apply_edit records a pre-edit backup + revert-edit exists
        // → reversible → Medium (trusted-only), NOT High like raw note/edit.
        "notes.apply_edit" => (
            "vaultpilot://note/edit",
            UriActionRisk::Medium,
            "apply an AI edit to a note (backup recorded)",
        ),
        // High — starting an AI chat may execute agent tools.
        "chat.new" => (
            "vaultpilot://chat/new",
            UriActionRisk::High,
            "start an AI chat",
        ),
        // High — bulk destructive operations (move retargets files; retag
        // rewrites note files via save_note, so both are irreversible-ish).
        "http_bulk_move_notes" => (
            "vaultpilot://note/bulk?op=move",
            UriActionRisk::High,
            "bulk move notes",
        ),
        "http_bulk_update_tags" => (
            "vaultpilot://note/bulk?op=update_tags",
            UriActionRisk::High,
            "bulk update note tags",
        ),
        // Not gated — read-only / navigational / non-vault tools, and the
        // audited `http_chat_completions` exemption (#3993).
        _ => return None,
    };
    Some(AutomationToolGate {
        uri,
        risk,
        description,
    })
}

/// Backwards-compatible URI lookup: returns the closest deep-link route for a
/// gated automation tool (see [`automation_tool_gate`]).
pub fn automation_tool_uri(tool: &str) -> Option<&'static str> {
    automation_tool_gate(tool).map(|gate| gate.uri)
}

/// Shared risk-based decision for the #3964 non-interactive gate.
///
/// - **Low** risk actions: always allowed (read-only / navigational).
/// - **Medium** risk actions: allowed only when `source` is a trusted app
///   (see [`TrustedAppRegistry`]).
/// - **High** risk actions: **always denied** — a headless server cannot
///   present the confirmation dialog that high-risk actions require.
pub fn should_allow_risk_non_interactive(
    risk: UriActionRisk,
    description: &str,
    source: &str,
    trusted: &TrustedAppRegistry,
) -> Result<(), String> {
    match risk {
        // Read-only actions are always safe to execute, even headless.
        UriActionRisk::Low => Ok(()),
        // Mutating but reversible, from a trusted app — allow.
        UriActionRisk::Medium if trusted.is_trusted(source) => Ok(()),
        UriActionRisk::Medium => {
            let source_label = if source.trim().is_empty() {
                "an external app".to_string()
            } else {
                source.to_string()
            };
            Err(format!(
                "denied by vaultpilot URI safety gate: {description} — \
                 {source_label} is not a trusted source, and headless entry \
                 points cannot confirm medium-risk actions"
            ))
        }
        // Destructive / potentially irreversible — always deny headless.
        UriActionRisk::High => Err(format!(
            "denied by vaultpilot URI safety gate: {description} — \
             headless clients cannot confirm high-risk actions"
        )),
    }
}

/// #3964 — evaluate the non-interactive gate for one automation tool call
/// (MCP tool name or HTTP bridge endpoint), using the tool's *effective* risk
/// class from [`automation_tool_gate`]. Ungated tools (read-only) always pass.
///
/// `source` is the calling app name (MCP `clientInfo.name` / per-session
/// header, HTTP `x-vaultpilot-source`); an empty source is treated as
/// untrusted.
pub fn should_allow_tool_non_interactive(
    tool: &str,
    source: &str,
    trusted: &TrustedAppRegistry,
) -> Result<(), String> {
    let Some(gate) = automation_tool_gate(tool) else {
        return Ok(());
    };
    should_allow_risk_non_interactive(gate.risk, gate.description, source, trusted)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

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
        // (parity with the shared Tauri UI deep-link handling).
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

    // ════════════════════════════════════════════════════════════════════════
    // #3822 — URI action security confirmation tests
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_3822_classify_risk_low_actions() {
        // Read-only / navigational actions are Low risk.
        assert_eq!(
            classify_uri_action_risk(&parse_deep_link("vaultpilot://note/abc")),
            UriActionRisk::Low
        );
        assert_eq!(
            classify_uri_action_risk(&parse_deep_link("vaultpilot://daily")),
            UriActionRisk::Low
        );
        assert_eq!(
            classify_uri_action_risk(&parse_deep_link("vaultpilot://search")),
            UriActionRisk::Low
        );
        assert_eq!(
            classify_uri_action_risk(&parse_deep_link("vaultpilot://settings")),
            UriActionRisk::Low
        );
    }

    #[test]
    fn test_3822_classify_risk_medium_new_note() {
        // Creating a note (no overwrite) is Medium risk.
        assert_eq!(
            classify_uri_action_risk(&parse_deep_link("vaultpilot://note/new")),
            UriActionRisk::Medium
        );
        assert_eq!(
            classify_uri_action_risk(&parse_deep_link("vaultpilot://note/new?append=1")),
            UriActionRisk::Medium
        );
    }

    #[test]
    fn test_3822_classify_risk_high_overwrite() {
        // Overwrite bumps to High risk (destructive).
        let action = parse_deep_link("vaultpilot://note/new?overwrite=1");
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::High);
    }

    #[test]
    fn test_3822_classify_risk_high_new_chat() {
        // Starting an AI chat is High risk (may execute agent tools).
        let action = parse_deep_link("vaultpilot://chat/new");
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::High);
    }

    #[test]
    fn test_3822_classify_risk_medium_unknown() {
        let action = parse_deep_link("vaultpilot://unknown/route");
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::Medium);
    }

    #[test]
    fn test_3822_describe_actions() {
        assert_eq!(
            describe_uri_action(&parse_deep_link("vaultpilot://note/new?name=My%20Note")),
            "Create a new note 'My Note'"
        );
        assert_eq!(
            describe_uri_action(&parse_deep_link("vaultpilot://note/abc-123")),
            "Open note 'abc-123'"
        );
        assert_eq!(
            describe_uri_action(&parse_deep_link("vaultpilot://daily")),
            "Open today's daily note"
        );
        assert_eq!(
            describe_uri_action(&parse_deep_link("vaultpilot://chat/new")),
            "Start a new AI chat session"
        );
        assert_eq!(
            describe_uri_action(&parse_deep_link("vaultpilot://search?query=rust")),
            "Search vault for 'rust'"
        );
    }

    #[test]
    fn test_3822_should_confirm_low_risk_always_allowed() {
        // Low-risk actions never need confirmation, even from unknown apps.
        let trusted = TrustedAppRegistry::default();
        let action = parse_deep_link("vaultpilot://note/abc-123");
        let decision = should_confirm_uri_action(&action, "unknown-app", &trusted);
        assert!(
            matches!(decision, UriConfirmationDecision::Allow { .. }),
            "Low risk should be allowed"
        );
    }

    #[test]
    fn test_3822_should_confirm_medium_untrusted() {
        // Medium risk from untrusted app → Confirm.
        let trusted = TrustedAppRegistry::default();
        let action = parse_deep_link("vaultpilot://note/new");
        let decision = should_confirm_uri_action(&action, "Raycast", &trusted);
        assert!(
            matches!(decision, UriConfirmationDecision::Confirm { .. }),
            "Medium risk from untrusted app should require confirmation"
        );
    }

    #[test]
    fn test_3822_should_confirm_medium_trusted_allowed() {
        // Medium risk from trusted app → Allow.
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Raycast");
        let action = parse_deep_link("vaultpilot://note/new");
        let decision = should_confirm_uri_action(&action, "Raycast", &trusted);
        assert!(
            matches!(decision, UriConfirmationDecision::Allow { .. }),
            "Medium risk from trusted app should be allowed"
        );
    }

    #[test]
    fn test_3822_should_confirm_high_always_confirm_even_trusted() {
        // High risk (overwrite) always confirms, even from trusted apps.
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Raycast");
        let action = parse_deep_link("vaultpilot://note/new?overwrite=1");
        let decision = should_confirm_uri_action(&action, "Raycast", &trusted);
        assert!(
            matches!(decision, UriConfirmationDecision::Confirm { .. }),
            "High risk should always require confirmation, even for trusted apps"
        );
    }

    #[test]
    fn test_3822_should_confirm_new_chat_always_confirm() {
        // NewChat is High risk → always confirm.
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Alfred");
        let action = parse_deep_link("vaultpilot://chat/new");
        let decision = should_confirm_uri_action(&action, "Alfred", &trusted);
        assert!(
            matches!(decision, UriConfirmationDecision::Confirm { .. }),
            "NewChat (High risk) should always require confirmation"
        );
    }

    #[test]
    fn test_3822_trusted_registry_case_insensitive() {
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Raycast");
        assert!(trusted.is_trusted("raycast"));
        assert!(trusted.is_trusted("RAYCAST"));
        assert!(trusted.is_trusted("Raycast"));
        assert!(!trusted.is_trusted("Alfred"));
    }

    #[test]
    fn test_3822_trusted_registry_empty_source_never_trusted() {
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("");
        assert!(!trusted.is_trusted(""));
        assert!(!trusted.is_trusted("   "));
    }

    #[test]
    fn test_3822_trusted_registry_revoke() {
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Raycast");
        assert!(trusted.is_trusted("Raycast"));
        trusted.revoke("raycast");
        assert!(!trusted.is_trusted("Raycast"));
    }

    #[test]
    fn test_3822_trusted_registry_round_trip() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-trusted-3822-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");

        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Raycast");
        trusted.trust("Alfred");
        trusted.save(&temp).expect("save");

        // Load from disk and verify persistence.
        let loaded = TrustedAppRegistry::load(&temp);
        assert!(loaded.is_trusted("Raycast"));
        assert!(loaded.is_trusted("Alfred"));
        assert!(!loaded.is_trusted("UnknownApp"));

        // Non-existent vault dir returns empty registry.
        let empty = TrustedAppRegistry::load(&temp.join("nonexistent"));
        assert!(!empty.is_trusted("Raycast"));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[cfg(unix)]
    #[test]
    fn test_3959_trusted_apps_save_is_0600_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;

        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-trusted-3959-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));

        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Alfred");
        trusted.save(&temp).expect("save");

        let path = trusted_apps_path(&temp);
        let mode = std::fs::metadata(&path)
            .expect("trusted_apps.yaml exists")
            .permissions()
            .mode();
        // The registry records user trust decisions under .vaultpilot/ and
        // must not be world-readable (0644) — same 0600 policy as settings.
        assert_eq!(
            mode & 0o777,
            0o600,
            "trusted_apps.yaml must be 0600, got {mode:o}"
        );

        // Round-trip still works through the hardened write path.
        let loaded = TrustedAppRegistry::load(&temp);
        assert!(loaded.is_trusted("Alfred"));

        let _ = std::fs::remove_dir_all(&temp);
    }

    // ════════════════════════════════════════════════════════════════════════
    // #3964 — non-interactive gate for headless automation entry points
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_3964_parse_delete_note_routes() {
        // `vaultpilot://note/delete` (symbolic form used by the MCP/HTTP gate).
        let action = parse_deep_link("vaultpilot://note/delete");
        assert!(matches!(
            action,
            DeepLinkAction::DeleteNote { note_id: None, .. }
        ));
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::High);

        // Path-segment id form.
        let action = parse_deep_link("vaultpilot://note/delete/abc-123");
        assert!(matches!(
            action,
            DeepLinkAction::DeleteNote {
                note_id: Some(ref id),
                ..
            } if id == "abc-123"
        ));
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::High);

        // Query-param id form.
        let action = parse_deep_link("vaultpilot://note/delete?id=abc-123");
        assert!(matches!(
            action,
            DeepLinkAction::DeleteNote {
                note_id: Some(ref id),
                ..
            } if id == "abc-123"
        ));
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::High);
    }

    #[test]
    fn test_3964_parse_edit_note_routes() {
        let action = parse_deep_link("vaultpilot://note/edit");
        assert!(matches!(
            action,
            DeepLinkAction::EditNote { note_id: None, .. }
        ));
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::High);

        let action = parse_deep_link("vaultpilot://note/edit/my-note");
        assert!(matches!(
            action,
            DeepLinkAction::EditNote {
                note_id: Some(ref id),
                ..
            } if id == "my-note"
        ));
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::High);
    }

    #[test]
    fn test_3964_parse_bulk_note_route() {
        let action = parse_deep_link("vaultpilot://note/bulk?op=update_tags");
        assert!(matches!(
            action,
            DeepLinkAction::BulkNoteOp { ref op, .. } if op == "update_tags"
        ));
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::High);

        // Missing op → empty string, still High.
        let action = parse_deep_link("vaultpilot://note/bulk");
        assert!(matches!(
            action,
            DeepLinkAction::BulkNoteOp { ref op, .. } if op.is_empty()
        ));
        assert_eq!(classify_uri_action_risk(&action), UriActionRisk::High);
    }

    #[test]
    fn test_3964_gate_low_always_allowed() {
        // Read-only actions are allowed even headless, with no source at all.
        let trusted = TrustedAppRegistry::default();
        let action = parse_deep_link("vaultpilot://note/abc-123");
        assert!(should_allow_non_interactive(&action, "", &trusted).is_ok());
        let action = parse_deep_link("vaultpilot://search?query=rust");
        assert!(should_allow_non_interactive(&action, "", &trusted).is_ok());
    }

    #[test]
    fn test_3964_gate_medium_trusted_allowed() {
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Alfred");
        let action = parse_deep_link("vaultpilot://note/new");
        assert!(
            should_allow_non_interactive(&action, "Alfred", &trusted).is_ok(),
            "Medium risk from a trusted source should be allowed headless"
        );
        // Case-insensitive trust lookup.
        assert!(should_allow_non_interactive(&action, "ALFRED", &trusted).is_ok());
    }

    #[test]
    fn test_3964_gate_medium_untrusted_denied() {
        let trusted = TrustedAppRegistry::default();
        let action = parse_deep_link("vaultpilot://note/new");
        let err = should_allow_non_interactive(&action, "Raycast", &trusted)
            .expect_err("untrusted Medium must be denied");
        assert!(
            err.contains("denied by vaultpilot URI safety gate"),
            "denial must name the gate, got: {err}"
        );
        assert!(
            err.contains("Create a new note"),
            "denial must describe the action: {err}"
        );
    }

    #[test]
    fn test_3964_gate_medium_empty_source_denied() {
        // Empty source = untrusted → Medium denied.
        let trusted = TrustedAppRegistry::default();
        let action = parse_deep_link("vaultpilot://note/new");
        assert!(should_allow_non_interactive(&action, "", &trusted).is_err());
        assert!(should_allow_non_interactive(&action, "  ", &trusted).is_err());
    }

    #[test]
    fn test_3964_gate_high_always_denied_even_trusted() {
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Alfred");

        // New chat: High, denied even from a trusted app.
        let chat = parse_deep_link("vaultpilot://chat/new");
        let err = should_allow_non_interactive(&chat, "Alfred", &trusted)
            .expect_err("High risk must be denied headless even when trusted");
        assert!(err.contains("headless clients cannot confirm high-risk actions"));

        // Delete: High, denied even from a trusted app.
        let del = parse_deep_link("vaultpilot://note/delete");
        assert!(should_allow_non_interactive(&del, "Alfred", &trusted).is_err());

        // Destructive edit: High, denied.
        let edit = parse_deep_link("vaultpilot://note/edit");
        assert!(should_allow_non_interactive(&edit, "Alfred", &trusted).is_err());

        // Bulk op: High, denied.
        let bulk = parse_deep_link("vaultpilot://note/bulk?op=move");
        assert!(should_allow_non_interactive(&bulk, "Alfred", &trusted).is_err());

        // Overwrite flag bumps note/new to High → denied.
        let overwrite = parse_deep_link("vaultpilot://note/new?overwrite=1");
        assert!(should_allow_non_interactive(&overwrite, "Alfred", &trusted).is_err());
    }

    #[test]
    fn test_3964_automation_tool_uri_mcp_mapping_risks() {
        // The MCP tool → gate mapping must classify with the expected risk
        // (#3964, #3992): chat.new High, notes.delete High, notes.create
        // Medium. notes.apply_edit records a pre-edit backup (revert-edit) so
        // it is Medium (reversible) rather than High like the raw note/edit
        // URI; notes.preview_edit is read-only and ungated (#3992).
        let cases = [
            ("chat.new", UriActionRisk::High),
            ("notes.delete", UriActionRisk::High),
            ("notes.apply_edit", UriActionRisk::Medium),
            ("notes.create", UriActionRisk::Medium),
            ("notes.import", UriActionRisk::Medium),
        ];
        for (tool, expected) in cases {
            let gate =
                automation_tool_gate(tool).unwrap_or_else(|| panic!("{tool} must map to a gate"));
            assert_eq!(gate.risk, expected, "tool {tool} risk mismatch");
        }
        // preview_edit is read-only — NOT gated.
        assert!(
            automation_tool_gate("notes.preview_edit").is_none(),
            "notes.preview_edit is read-only and must not be gated"
        );
    }

    #[test]
    fn test_3964_automation_tool_uri_http_mapping_risks() {
        let cases = [
            ("http_create_note", UriActionRisk::Medium),
            ("http_clip_url", UriActionRisk::Medium),
            ("http_import_folder", UriActionRisk::Medium),
            ("http_delete_note", UriActionRisk::High),
            ("http_bulk_delete_notes", UriActionRisk::High),
            ("http_bulk_move_notes", UriActionRisk::High),
            ("http_bulk_update_tags", UriActionRisk::High),
            // #3993 — subscription lifecycle + run + AI actions were ungated;
            // they mutate app data / vault notes, so they are now gated.
            ("http_create_subscription", UriActionRisk::Medium),
            ("http_update_subscription", UriActionRisk::Medium),
            ("http_toggle_subscription", UriActionRisk::Medium),
            ("http_delete_subscription", UriActionRisk::Medium),
            ("http_run_subscription", UriActionRisk::Medium),
            ("http_ai_action", UriActionRisk::Medium),
        ];
        for (tool, expected) in cases {
            let gate =
                automation_tool_gate(tool).unwrap_or_else(|| panic!("{tool} must map to a gate"));
            assert_eq!(gate.risk, expected, "tool {tool} risk mismatch");
        }
    }

    #[test]
    fn test_3964_automation_tool_uri_read_only_tools_ungated() {
        // Read-only / non-vault tools must NOT be gated.
        for tool in [
            "notes.list",
            "notes.get",
            "notes.search",
            "notes.related",
            "notes.follow_links",
            "notes.backlinks",
            "notes.preview_edit", // read-only, #3992
            "chat.list_sessions",
            "chat.get_state",
            "chat.send",
            "email.search",
            "email.get",
            "calendar.today",
            "tags.list",
            "index.rebuild",
            "ask",
            "http_list_notes",
            "http_get_note",
            "http_search_notes",
            "http_typeahead",
            "http_progressive_search",
            "http_health",
            "http_vault_health",
            "http_graph",
            "http_get_subscription",
            "http_list_subscriptions",
            "http_list_ai_actions",
            "http_settings_definitions",
            // #3993 — the OpenAI-compat chat endpoint is exempted (audited in
            // the bridge) so it stays usable by token-only clients.
            "http_chat_completions",
        ] {
            assert!(
                automation_tool_gate(tool).is_none(),
                "{tool} should not be gated"
            );
        }
    }

    #[test]
    fn test_3992_preview_edit_and_apply_edit_gate_semantics() {
        // The MCP preview → apply workflow (#3108) must not be locked out:
        // - preview_edit is read-only → always allowed, even with no source.
        // - apply_edit is reversible (backup + revert-edit) → Medium: allowed
        //   only from a trusted source (#3992).
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("VaultPilot-WinUI");

        assert!(
            should_allow_tool_non_interactive("notes.preview_edit", "someone", &trusted).is_ok(),
            "read-only preview must be ungated"
        );
        assert!(
            should_allow_tool_non_interactive("notes.preview_edit", "", &trusted).is_ok(),
            "read-only preview must be ungated even without a source"
        );

        let err = should_allow_tool_non_interactive("notes.apply_edit", "", &trusted)
            .expect_err("untrusted apply_edit must be denied");
        assert!(
            err.contains("not a trusted source"),
            "denial should explain the trusted-source requirement: {err}"
        );
        assert!(
            should_allow_tool_non_interactive("notes.apply_edit", "VaultPilot-WinUI", &trusted)
                .is_ok(),
            "apply_edit from a trusted source must pass the gate"
        );
    }

    #[test]
    fn test_3993_new_http_endpoints_require_trusted_source() {
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("VaultPilot-WinUI");

        for tool in [
            "http_create_subscription",
            "http_update_subscription",
            "http_delete_subscription",
            "http_run_subscription",
            "http_ai_action",
        ] {
            assert!(
                should_allow_tool_non_interactive(tool, "", &trusted).is_err(),
                "{tool} must be denied for an untrusted source"
            );
            assert!(
                should_allow_tool_non_interactive(tool, "VaultPilot-WinUI", &trusted).is_ok(),
                "{tool} must pass for a trusted source"
            );
        }
        // chat_completions is intentionally exempt (audited in the bridge).
        assert!(
            should_allow_tool_non_interactive("http_chat_completions", "", &trusted).is_ok(),
            "http_chat_completions is exempt from the URI gate (#3993)"
        );
    }

    #[test]
    fn test_3964_gate_via_automation_mapping() {
        // End-to-end through the mapping + gate: notes.create (Medium) from a
        // trusted source passes; from an empty/untrusted source it is denied;
        // chat.new (High) is denied regardless of trust.
        let mut trusted = TrustedAppRegistry::default();
        trusted.trust("Claude");

        let create_uri = automation_tool_uri("notes.create").unwrap();
        let create = parse_deep_link(create_uri);
        assert!(should_allow_non_interactive(&create, "Claude", &trusted).is_ok());
        assert!(should_allow_non_interactive(&create, "", &trusted).is_err());
        assert!(should_allow_non_interactive(&create, "Cursor", &trusted).is_err());

        let chat_uri = automation_tool_uri("chat.new").unwrap();
        let chat = parse_deep_link(chat_uri);
        assert!(should_allow_non_interactive(&chat, "Claude", &trusted).is_err());

        let delete_uri = automation_tool_uri("notes.delete").unwrap();
        let delete = parse_deep_link(delete_uri);
        assert!(should_allow_non_interactive(&delete, "Claude", &trusted).is_err());
    }
}
