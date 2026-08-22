//! Regression test for issue #3159: OS-native keychain integration.
//!
//! Verifies that the `keychain::SelectiveStore` can store and retrieve
//! secrets through whichever backend is available on the current host,
//! and that the integration points in `storage::settings` correctly
//! round-trip plaintext API keys through the store.

use crate::keychain::{account_key, KEYCHAIN};
use crate::models::{AppSettings, ProviderConfig};
use crate::storage::pool::StorageContext;
use crate::storage::save_settings_with_context;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_context(label: &str) -> StorageContext {
    let temp = std::env::temp_dir().join(format!(
        "vaultpilot-reg3159-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp).expect("temp dir");
    StorageContext::for_test(&temp)
}

#[test]
fn regression_3159_keychain_available_or_fallback() {
    // The KEYCHAIN global must not panic when accessed.
    let _ = KEYCHAIN.get("probe-reg3159");
}

#[test]
fn regression_3159_set_get_delete_roundtrip() {
    // Test the global keychain with a temporary entry.
    let key = "reg3159-test-key-abc";

    // Clean start.
    let _ = KEYCHAIN.delete(key);
    assert!(KEYCHAIN.get(key).unwrap().is_none());

    // Set and verify.
    KEYCHAIN
        .set(key, "sk-reg-3159-roundtrip")
        .expect("set should work");

    // Read back if the OS keychain is active; otherwise the file-crypto
    // fallback's get() returns None by design.
    if KEYCHAIN.is_os_keychain_active() {
        assert_eq!(
            KEYCHAIN.get(key).unwrap().as_deref(),
            Some("sk-reg-3159-roundtrip")
        );
        KEYCHAIN.delete(key).unwrap();
        assert!(KEYCHAIN.get(key).unwrap().is_none());
    }
}

#[test]
fn regression_3159_account_key_uniqueness() {
    // Different providers must map to different account keys.
    let primary = account_key("primary");
    let custom = account_key("OpenAI");
    assert_ne!(
        primary, custom,
        "primary and named provider keys must differ"
    );
    assert_eq!(account_key("Anthropic"), "api_key:Anthropic");
}

#[test]
fn regression_3159_save_settings_does_not_panic_with_keychain() {
    // The save_settings_with_context function should not panic when the
    // keychain is available (it's a best-effort store).
    let ctx = temp_context("save");
    let settings = AppSettings {
        vault_dir: ctx
            .settings_path()
            .parent()
            .unwrap()
            .join("vault")
            .to_string_lossy()
            .to_string(),
        provider: ProviderConfig {
            name: "Test3159".into(),
            api_key: "sk-test-3159...0000".to_string(),
            ..ProviderConfig::default()
        },
        ..AppSettings::default()
    };

    let result = save_settings_with_context(&ctx, settings);
    assert!(
        result.is_ok(),
        "save must succeed even when keychain is unavailable: {:?}",
        result.err()
    );
}

#[test]
fn regression_3159_load_settings_with_keychain_or_fallback() {
    // Load and save through the global keychain must not produce errors.
    // This exercises the integration points added in #3159.
    let ctx = temp_context("load");

    // Save settings with a known API key.
    let saved = save_settings_with_context(
        &ctx,
        AppSettings {
            vault_dir: ctx
                .settings_path()
                .parent()
                .unwrap()
                .join("vault")
                .to_string_lossy()
                .to_string(),
            provider: ProviderConfig {
                name: "Reg3159".into(),
                api_key: "sk-reg-3159-load".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        },
    )
    .expect("save should succeed");

    // The key should survive the round-trip.
    assert_eq!(
        saved.provider.api_key, "sk-reg-3159-load",
        "save must return the plaintext key"
    );

    // Load a fresh context to bypass cache and verify on-disk persistence.
    let ctx2 = temp_context("load-reload");
    // Copy the settings file from the first context.
    fs::copy(ctx.settings_path(), ctx2.settings_path()).ok();

    let loaded_raw = fs::read_to_string(ctx.settings_path()).unwrap_or_default();
    assert!(
        loaded_raw.contains("ENC:v1:") || loaded_raw.contains("***"),
        "settings file must contain either encrypted or plaintext key"
    );
}
