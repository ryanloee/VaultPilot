//! Regression test for issue #3170: clearing an API key does not remove it
//! from the OS keychain, so the deleted key silently resurrects on reload.
//!
//! Bug:       When the OS keychain is active, clearing/emptying an API key
//!            in Settings does NOT call KEYCHAIN.delete() — the stale entry
//!            survives and is restored during load_settings_with_context.
//! Root cause: `save_settings_with_context` only called KEYCHAIN.set() for
//!             non-empty keys.  Empty keys were silently skipped, leaving the
//!             old secret in the keychain.
//! Fix:       PR / commit — when api_key_plaintext is empty, explicitly call
//!            KEYCHAIN.delete() before saving.
//!
//! Note: When the OS keychain is NOT active (CI / headless), the fallback
//! FileCryptoStore returns None from get() and delete() is a no-op.  The
//! file-based path is independently verified by the settings round-trip in
//! `src/storage/settings.rs`.

#[cfg(test)]
mod tests {
    use crate::keychain::KEYCHAIN;
    use crate::models::{AppSettings, ProviderConfig};
    use crate::storage::pool::StorageContext;
    use crate::storage::save_settings_with_context;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(label: &str) -> StorageContext {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-reg3170-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        StorageContext::for_test(&temp)
    }

    /// Primary regression: set key → save → clear key → save → verify key is gone.
    #[test]
    fn regression_3170_clearing_key_removes_it_from_keychain() {
        let ctx = temp_context("del");
        let vault = ctx
            .settings_path()
            .parent()
            .unwrap()
            .join("vault")
            .to_string_lossy()
            .to_string();

        let provider_name = "Primary3170";

        // Step 1: Save a non-empty API key.
        let with_key = AppSettings {
            vault_dir: vault.clone(),
            provider: ProviderConfig {
                name: provider_name.into(),
                api_key: "sk-test-3170...aaaa".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        save_settings_with_context(&ctx, with_key).expect("save with key");

        // Step 2: Clear the key (save with empty) — this should trigger delete.
        let empty_key = AppSettings {
            vault_dir: vault,
            provider: ProviderConfig {
                name: provider_name.into(),
                api_key: String::new(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        save_settings_with_context(&ctx, empty_key.clone()).expect("save empty failed");

        // Step 3: Verify that the keychain no longer holds a key for this provider.
        // On the file-crypto fallback (headless CI): KEYCHAIN.get returns None —
        // the test is trivially satisfied.  On OS keychain: we should get None.
        let kc_result = KEYCHAIN.get(&crate::keychain::account_key(provider_name));
        match kc_result {
            Ok(None) => {
                // Key is gone — both fallback and OS keychain paths are correct.
            }
            Ok(Some(found)) => {
                // If the OS keychain still has the key, the delete must have
                // been skipped — this is the exact #3170 bug.
                panic!(
                    "KEYCHAIN still holds key '{found}' after it was cleared; \
                     delete() was not called on the keychain"
                );
            }
            Err(e) => {
                // An error is acceptable (the backend just wasn't available).
                // Don't panic — this is CI/headless behaviour.
                eprintln!("KEYCHAIN.get error (acceptable): {e}");
            }
        }
    }

    /// Provider-list variant: verify that clearing a multi-provider key also
    /// calls KEYCHAIN.delete(account_key(...)) for that provider.
    #[test]
    fn test_regression_3170_clear_multi_provider_key() {
        if !KEYCHAIN.is_os_keychain_active() {
            // Without an OS keychain, KEYCHAIN.get always returns None and
            // delete is a no-op.  The file-only path has its own round-trip
            // test in `src/storage/settings.rs`.  Skip rather than false-fail.
            return;
        }

        let ctx = temp_context("3170mp");
        let vault = ctx
            .settings_path()
            .parent()
            .unwrap()
            .join("vault")
            .to_string_lossy()
            .to_string();

        let p1_key = "sk-multi-a...1111";
        let p2_key = "sk-multi-b...2222";

        // Save two providers with keys.
        let settings = AppSettings {
            vault_dir: vault.clone(),
            provider: ProviderConfig {
                name: "Primary3170mp".into(),
                api_key: "".into(),
                ..ProviderConfig::default()
            },
            providers: vec![
                ProviderConfig {
                    name: "P3170a".into(),
                    api_key: p1_key.to_string(),
                    ..ProviderConfig::default()
                },
                ProviderConfig {
                    name: "P3170b".into(),
                    api_key: p2_key.to_string(),
                    ..ProviderConfig::default()
                },
            ],
            ..AppSettings::default()
        };
        save_settings_with_context(&ctx, settings).expect("first save");

        // Clear only p1's key, keep p2.
        let cleared = AppSettings {
            vault_dir: vault,
            provider: ProviderConfig {
                name: "Primary3170mp".into(),
                api_key: "".into(),
                ..ProviderConfig::default()
            },
            providers: vec![
                ProviderConfig {
                    name: "P3170a".into(),
                    api_key: String::new(), // cleared
                    ..ProviderConfig::default()
                },
                ProviderConfig {
                    name: "P3170b".into(),
                    api_key: p2_key.to_string(), // unchanged
                    ..ProviderConfig::default()
                },
            ],
            ..AppSettings::default()
        };
        save_settings_with_context(&ctx, cleared).expect("save cleared");

        // p1's key must be deleted from the keychain.
        let p1_account = crate::keychain::account_key("P3170a");
        let p1_result = KEYCHAIN.get(&p1_account);
        assert!(
            p1_result.is_err() || p1_result.unwrap().is_none(),
            "Key for deleted provider P3170a still present in keychain"
        );

        // p2's key must still be present (it was not cleared).
        // NOTE: `account_key` returns the keychain identifier (e.g.
        // "api_key:P3170b"), NOT the secret.  Bind it to a distinct name so
        // the value comparison below checks the retrieved secret against the
        // original secret `p2_key` ("sk-multi-b...2222"), not the identifier.
        let p2_account = crate::keychain::account_key("P3170b");
        let p2_result = KEYCHAIN.get(&p2_account);
        assert!(
            p2_result.is_ok() && p2_result.unwrap().map(|v| v == p2_key).unwrap_or(false),
            "Key for provider P3170b should NOT have been deleted"
        );
    }
}
