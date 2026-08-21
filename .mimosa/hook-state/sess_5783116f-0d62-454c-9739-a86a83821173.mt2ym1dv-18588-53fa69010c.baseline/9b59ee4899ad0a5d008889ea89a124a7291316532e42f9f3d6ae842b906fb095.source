//! Plugin system for VaultPilot.
//!
//! Defines the `VaultPlugin` trait that custom plugins implement to extend
//! VaultPilot functionality. Plugins are registered with `PluginManager` and
//! invoked at specific lifecycle hooks (note saved, note deleted, etc.).
//!
//! # Design principles
//! - **Fail-safe**: plugin errors are logged and swallowed, never crash the host.
//! - **Minimal API surface**: only expose what plugins actually need.
//! - **No dynamic loading yet**: plugins are compiled in. WASM/dylib loading is
//!   tracked separately.

use std::sync::Arc;

use anyhow::Result;
use tracing::warn;

use crate::models::NoteDocument;

/// Metadata describing a plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
}

/// Context passed to plugin hooks, providing read access to vault state.
pub struct PluginContext<'a> {
    pub vault_dir: &'a str,
}

/// Trait that custom plugins implement.
///
/// All methods have default no-op implementations so plugins only need to
/// override the hooks they care about.
pub trait VaultPlugin: Send + Sync {
    /// Return metadata about this plugin.
    fn info(&self) -> PluginInfo;

    /// Called after a note is saved to disk and indexed.
    fn on_note_saved(&self, _ctx: &PluginContext, _note: &NoteDocument) -> Result<()> {
        Ok(())
    }

    /// Called after a note is deleted.
    fn on_note_deleted(&self, _ctx: &PluginContext, _note_id: &str) -> Result<()> {
        Ok(())
    }
}

/// Manages registered plugins and dispatches lifecycle hooks.
pub struct PluginManager {
    plugins: Vec<Arc<dyn VaultPlugin>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin.
    pub fn register(&mut self, plugin: Arc<dyn VaultPlugin>) {
        self.plugins.push(plugin);
    }

    /// Return metadata for all registered plugins.
    pub fn list(&self) -> Vec<PluginInfo> {
        self.plugins.iter().map(|p| p.info()).collect()
    }

    /// Number of registered plugins.
    pub fn count(&self) -> usize {
        self.plugins.len()
    }

    /// Dispatch `on_note_saved` to all plugins. Errors are logged, not propagated.
    pub fn notify_note_saved(&self, ctx: &PluginContext, note: &NoteDocument) {
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_note_saved(ctx, note) {
                warn!(
                    plugin = %plugin.info().name,
                    error = %e,
                    "plugin on_note_saved hook failed"
                );
            }
        }
    }

    /// Dispatch `on_note_deleted` to all plugins. Errors are logged, not propagated.
    pub fn notify_note_deleted(&self, ctx: &PluginContext, note_id: &str) {
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_note_deleted(ctx, note_id) {
                warn!(
                    plugin = %plugin.info().name,
                    error = %e,
                    "plugin on_note_deleted hook failed"
                );
            }
        }
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A test plugin that records hook invocations.
    struct TestPlugin {
        info: PluginInfo,
        saved_ids: Mutex<Vec<String>>,
        deleted_ids: Mutex<Vec<String>>,
    }

    impl TestPlugin {
        fn new(name: &str) -> Self {
            Self {
                info: PluginInfo {
                    name: name.to_string(),
                    version: "0.1.0".to_string(),
                    description: "test plugin".to_string(),
                },
                saved_ids: Mutex::new(Vec::new()),
                deleted_ids: Mutex::new(Vec::new()),
            }
        }
    }

    impl VaultPlugin for TestPlugin {
        fn info(&self) -> PluginInfo {
            self.info.clone()
        }

        fn on_note_saved(&self, _ctx: &PluginContext, note: &NoteDocument) -> Result<()> {
            self.saved_ids.lock().unwrap().push(note.meta.id.clone());
            Ok(())
        }

        fn on_note_deleted(&self, _ctx: &PluginContext, note_id: &str) -> Result<()> {
            self.deleted_ids.lock().unwrap().push(note_id.to_string());
            Ok(())
        }
    }

    /// A plugin that always fails — errors should be swallowed.
    struct FailingPlugin;

    impl VaultPlugin for FailingPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo {
                name: "failing".to_string(),
                version: "0.0.0".to_string(),
                description: "always fails".to_string(),
            }
        }

        fn on_note_saved(&self, _ctx: &PluginContext, _note: &NoteDocument) -> Result<()> {
            anyhow::bail!("intentional failure");
        }
    }

    #[test]
    fn plugin_manager_register_and_list() {
        let mut mgr = PluginManager::new();
        assert_eq!(mgr.count(), 0);

        mgr.register(Arc::new(TestPlugin::new("alpha")));
        mgr.register(Arc::new(TestPlugin::new("beta")));
        assert_eq!(mgr.count(), 2);

        let list = mgr.list();
        let names: Vec<&str> = list.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn plugin_manager_dispatches_note_saved() {
        let plugin = Arc::new(TestPlugin::new("tracker"));
        let mut mgr = PluginManager::new();
        mgr.register(plugin.clone());

        let ctx = PluginContext {
            vault_dir: "/tmp/test",
        };
        let note = NoteDocument::default();
        mgr.notify_note_saved(&ctx, &note);

        let saved = plugin.saved_ids.lock().unwrap();
        assert_eq!(saved.len(), 1);
    }

    #[test]
    fn plugin_manager_dispatches_note_deleted() {
        let plugin = Arc::new(TestPlugin::new("tracker"));
        let mut mgr = PluginManager::new();
        mgr.register(plugin.clone());

        let ctx = PluginContext {
            vault_dir: "/tmp/test",
        };
        mgr.notify_note_deleted(&ctx, "note-123");

        let deleted = plugin.deleted_ids.lock().unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0], "note-123");
    }

    #[test]
    fn plugin_manager_swallows_errors() {
        let mut mgr = PluginManager::new();
        mgr.register(Arc::new(FailingPlugin));

        let ctx = PluginContext {
            vault_dir: "/tmp/test",
        };
        // Should not panic — error is logged and swallowed.
        mgr.notify_note_saved(&ctx, &NoteDocument::default());
        mgr.notify_note_deleted(&ctx, "x");
    }

    #[test]
    fn plugin_manager_multiple_plugins_all_called() {
        let p1 = Arc::new(TestPlugin::new("p1"));
        let p2 = Arc::new(TestPlugin::new("p2"));
        let mut mgr = PluginManager::new();
        mgr.register(p1.clone());
        mgr.register(p2.clone());

        let ctx = PluginContext {
            vault_dir: "/tmp/test",
        };
        mgr.notify_note_saved(&ctx, &NoteDocument::default());

        assert_eq!(p1.saved_ids.lock().unwrap().len(), 1);
        assert_eq!(p2.saved_ids.lock().unwrap().len(), 1);
    }
}
