//! Regression test for #3809: `active_provider_index` not clamped when
//! providers is empty.
//!
//! Bug: `normalize_settings` only clamped `active_provider_index` when
//! `providers` was non-empty. If `providers` was empty and the index was
//! non-zero, the invalid index persisted, and downstream code indexing
//! `providers[active_provider_index]` could panic with out-of-bounds.
//!
//! Fix: always clamp — when providers is empty the index is normalized to 0
//! (the only safe value), so no downstream indexing can go out of range.

use std::path::PathBuf;

use crate::models::AppSettings;
use crate::storage::pool::StorageContext;
use crate::storage::{load_settings_with_context, save_settings_with_context};

fn temp_context(label: &str) -> (StorageContext, PathBuf) {
    let temp = std::env::temp_dir().join(format!(
        "vaultpilot-reg3809-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).expect("temp dir");
    let ctx = StorageContext::for_test(&temp);
    // for_test() seeds default_vault_dir = temp/vault — materialise it so
    // settings validation (vault_dir must exist) passes.
    std::fs::create_dir_all(temp.join("vault")).expect("vault dir");
    (ctx, temp)
}

#[test]
fn empty_providers_clamps_active_index_on_save() {
    let (ctx, temp) = temp_context("save");

    // providers is empty; index 99 must be clamped to 0 on save.
    let mut settings = AppSettings::default();
    settings.providers = vec![];
    settings.active_provider_index = 99;
    settings.vault_dir = temp.join("vault").to_string_lossy().into_owned();

    let saved = save_settings_with_context(&ctx, settings).expect("save settings");
    assert_eq!(
        saved.active_provider_index, 0,
        "empty providers: out-of-range index must be clamped to 0, got {}",
        saved.active_provider_index
    );

    std::fs::remove_dir_all(&temp).ok();
}

#[test]
fn empty_providers_clamps_active_index_on_load() {
    let (ctx, temp) = temp_context("load");

    // Write a settings file directly with empty providers + invalid index,
    // bypassing save_settings_with_context to simulate a hand-edited or
    // legacy file reaching the load path.
    let mut settings = AppSettings::default();
    settings.providers = vec![];
    settings.active_provider_index = 42;
    settings.vault_dir = temp.join("vault").to_string_lossy().into_owned();
    let json = serde_json::to_string_pretty(&settings).expect("serialize");
    std::fs::write(temp.join("settings.json"), json).expect("write settings");

    let loaded = load_settings_with_context(&ctx).expect("load settings");
    assert_eq!(
        loaded.active_provider_index, 0,
        "empty providers: invalid index must be clamped to 0 on load, got {}",
        loaded.active_provider_index
    );

    std::fs::remove_dir_all(&temp).ok();
}
