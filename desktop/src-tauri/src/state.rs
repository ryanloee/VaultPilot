//! Application state shared across all Tauri commands.
//!
//! Holds a single [`StorageContext`] — the same one the sidecar agent uses —
//! so every command operates on the exact same settings.json / SQLite / chat
//! state that the existing WinUI client does. Switching back and forth between
//! the old and new desktop frontends loses no data.

use std::sync::Arc;
use vaultpilot_lib::storage::StorageContext;

/// Tauri-managed app state. Cheap to clone (internally `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<StorageContext>,
}

impl AppState {
    /// Initialize storage using the same on-disk layout as the WinUI sidecar:
    /// `%APPDATA%/com.local.vaultpilot/settings.json` etc. on Windows,
    /// `~/.config/...` on Linux, `~/Library/Application Support/...` on macOS.
    pub fn new() -> anyhow::Result<Self> {
        let storage = StorageContext::for_sidecar()
            .map_err(|e| anyhow::anyhow!("failed to initialize storage context: {e}"))?;
        Ok(Self {
            storage: Arc::new(storage),
        })
    }
}
