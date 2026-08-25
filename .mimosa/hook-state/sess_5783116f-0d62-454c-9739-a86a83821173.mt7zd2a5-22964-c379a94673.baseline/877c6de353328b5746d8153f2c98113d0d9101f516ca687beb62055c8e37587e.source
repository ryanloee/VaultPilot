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
        // Initialize the LAN sync engine (pairing state + peer list) using the
        // same vault and an out-of-vault config dir so peers aren't synced.
        vaultpilot_lib::sync::init_sync_state(
            storage.vault_dir().to_path_buf(),
            storage.app_config_dir(),
        );
        Ok(Self {
            storage: Arc::new(storage),
        })
    }
}
