//! Application state shared across all Tauri commands.
//!
//! Holds a single [`StorageContext`] — the same layout the sidecar agent uses —
//! so every command operates on the exact same settings.json / SQLite / chat
//! state across Windows, Linux and Android.

use std::sync::Arc;
use vaultpilot_lib::storage::StorageContext;

/// Tauri-managed app state. Cheap to clone (internally `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<StorageContext>,
}

impl AppState {
    /// Initialize storage using the same on-disk layout as the sidecar agent:
    /// `%APPDATA%/com.local.vaultpilot/settings.json` etc. on Windows,
    /// `~/.config/...` on Linux, and the Tauri app data dir on Android/iOS.
    pub fn new() -> anyhow::Result<Self> {
        let storage = StorageContext::for_sidecar()
            .map_err(|e| anyhow::anyhow!("failed to initialize storage context: {e}"))?;
        Ok(Self {
            storage: Arc::new(storage),
        })
    }
}
