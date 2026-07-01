//! Real-time Context Surface — proactive "relevant notes" panel (#1995).
//!
//! While editing a note or during a meeting, VaultPilot can surface notes
//! related to the *current* content without the user searching — or even saving.
//! This module implements **Phase 1** of #1995: a lightweight **live mode** that
//! debounces recomputation as text changes and queries the vault over a sliding
//! window of recent text.
//!
//! It builds on [`crate::storage::find_related_notes_for_text_with_context`],
//! which ranks the vault against free-form text (no saved note required).
//!
//! # Design goals
//! - **Cheap**: never re-rank on every keystroke. [`LiveContextSession`] only
//!   refreshes when the windowed text has changed *and* at least
//!   [`LiveContextConfig::min_interval`] has elapsed since the last refresh.
//! - **Self-contained**: pure orchestration over the existing storage ranking —
//!   no new indexes, threads, or background services are required for the core
//!   logic. A UI/CLI host drives it by feeding the latest text + a clock value.
//! - **Vault-safe**: only reads notes; the live panel never mutates the vault.
//!
//! # Example
//! ```no_run
//! use std::time::{Duration, Instant};
//! use vaultpilot_lib::context_surface::{LiveContextConfig, LiveContextSession};
//! use vaultpilot_lib::storage::StorageContext;
//!
//! let ctx = StorageContext::for_cli(Some(".".into()))?; // your real vault dir
//! let cfg = LiveContextConfig { min_interval: Duration::from_millis(800), ..Default::default() };
//! let mut session = LiveContextSession::new(cfg);
//!
//! // As the user types, feed the latest buffer + a monotonic clock:
//! let now = Instant::now();
//! let surfaced = session.consider(&ctx, "meeting notes on Rust ownership model", now)?;
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::models::RelatedNote;
use crate::storage::{find_related_notes_for_text_with_context, StorageContext};

/// Configuration for a [`LiveContextSession`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveContextConfig {
    /// Minimum time between two recomputations (debounce). Within this window
    /// [`LiveContextSession::should_refresh`] returns `false` even if the text
    /// changed, to keep the live panel cheap under rapid typing.
    pub min_interval: Duration,
    /// Number of trailing characters of the current text considered when
    /// building the query (sliding window). Keeps the query focused on the most
    /// recent context (a meeting's latest transcript window, or the tail of a
    /// note being drafted) rather than the whole document.
    pub window_chars: usize,
    /// Maximum number of related notes to surface per refresh.
    pub limit: usize,
}

impl Default for LiveContextConfig {
    fn default() -> Self {
        Self {
            min_interval: Duration::from_millis(1500),
            window_chars: 600,
            limit: 5,
        }
    }
}

/// Stateful controller that decides *when* to refresh the live context panel
/// (#1995 Phase 1).
///
/// A host (WinUI editor, CLI, Android notification surface) feeds the latest
/// text plus a monotonic clock value via [`consider`]; the session returns the
/// freshly surfaced notes only when a refresh is due, otherwise `None`. This
/// keeps recomputation cheap and predictable.
///
/// [`consider`]: LiveContextSession::consider
#[derive(Debug, Clone)]
pub struct LiveContextSession {
    config: LiveContextConfig,
    last_query: String,
    last_refresh: Option<Instant>,
}

impl LiveContextSession {
    /// Create a new session with the given configuration.
    pub fn new(config: LiveContextConfig) -> Self {
        Self {
            config,
            last_query: String::new(),
            last_refresh: None,
        }
    }

    /// The configured limits.
    pub fn config(&self) -> &LiveContextConfig {
        &self.config
    }

    /// Trailing-window view of `text` (last [`window_chars`] bytes, aligned to a
    /// UTF-8 char boundary so multibyte content is never split).
    ///
    /// [`window_chars`]: LiveContextConfig::window_chars
    pub fn windowed<'a>(&self, text: &'a str) -> &'a str {
        let len = text.len();
        if len <= self.config.window_chars {
            return text;
        }
        let mut start = len - self.config.window_chars;
        while start < len && !text.is_char_boundary(start) {
            start += 1;
        }
        &text[start..]
    }

    /// True when a refresh should run now: the windowed query differs from the
    /// last refresh **and** at least [`min_interval`] has elapsed (or no refresh
    /// has happened yet).
    ///
    /// [`min_interval`]: LiveContextConfig::min_interval
    pub fn should_refresh(&self, text: &str, now: Instant) -> bool {
        let query = self.windowed(text);
        if query == self.last_query {
            return false;
        }
        match self.last_refresh {
            None => true,
            Some(last) => now.duration_since(last) >= self.config.min_interval,
        }
    }

    /// Force a recomputation now using the windowed text, then update internal
    /// state. Returns the surfaced related notes (possibly empty).
    pub fn refresh(
        &mut self,
        context: &StorageContext,
        text: &str,
        now: Instant,
    ) -> Result<Vec<RelatedNote>> {
        let windowed = self.windowed(text).to_string();
        let notes =
            find_related_notes_for_text_with_context(context, &windowed, self.config.limit)?;
        self.last_query = windowed;
        self.last_refresh = Some(now);
        Ok(notes)
    }

    /// Convenience: if [`should_refresh`] is true, run [`refresh`] and return
    /// the notes; otherwise return `None` (no work done this tick).
    ///
    /// [`should_refresh`]: LiveContextSession::should_refresh
    /// [`refresh`]: LiveContextSession::refresh
    pub fn consider(
        &mut self,
        context: &StorageContext,
        text: &str,
        now: Instant,
    ) -> Result<Option<Vec<RelatedNote>>> {
        if self.should_refresh(text, now) {
            Ok(Some(self.refresh(context, text, now)?))
        } else {
            Ok(None)
        }
    }

    /// Reset session state (e.g. when the user switches to a different note).
    pub fn reset(&mut self) {
        self.last_query.clear();
        self.last_refresh = None;
    }
}

/// One-shot helper: surface up to `limit` notes related to `text` with no
/// session state. Convenient for non-interactive callers (CLI, MCP tool).
pub fn surface_for_text(
    context: &StorageContext,
    text: &str,
    limit: usize,
) -> Result<Vec<RelatedNote>> {
    find_related_notes_for_text_with_context(context, text, limit)
}
