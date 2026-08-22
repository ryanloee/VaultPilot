//! Regression test for issue #3193: deleting a provider from the multi-provider
//! list leaves its plaintext API key lingering in the OS keychain.
//!
//! Bug:       When a provider is removed entirely from `AppSettings.providers`
//!            and the settings are saved, `save_settings_with_context` only
//!            syncs keychain state for providers still present in the list.
//!            The deleted provider's key is never removed, so it survives in
//!            the OS keychain (privacy leak + silent resurrection risk).
//! Root cause: the keychain sync loop iterates `settings.providers` only, so a
//!            removed provider has no branch that deletes its keychain entry.
//! Fix:       Before the sync loop, diff the old provider list (from
//!            `load_settings_raw`) against the new one and delete keychain
//!            entries for any provider name that disappeared.
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
            "vaultpilot-reg3193-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        StorageContext::for_test(&temp)
    }

    fn provider(name: &str, key: &str) -> ProviderConfig {
        ProviderConfig {
            name: name.into(),
            api_key: key.to_string(),
            ..ProviderConfig::default()
        }
    }

    /// Primary regression: save two providers with keys → re-save with only
    /// one provider → the removed provider's keychain entry must be gone.
    #[test]
    fn regression_3193_deleted_provider_key_removed_from_keychain() {
        if !KEYCHAIN.is_os_keychain_active() {
            // Without an OS keychain, KEYCHAIN.get always returns None and
            // delete is a no-op. The file-only path has its own round-trip
            // test in `src/storage/settings.rs`. Skip rather than false-fail.
            return;
        }

        let ctx = temp_context("delprov");
        let vault = ctx
            .settings_path()
            .parent()
            .unwrap()
            .join("vault")
            .to_string_lossy()
            .to_string();

        // Step 1: Save two providers, both with keys.
        let first = AppSettings {
            vault_dir: vault.clone(),
            provider: provider("Primary3193", ""),
            providers: vec![
                provider("P3193a", "sk-3193-a...aaaa"),
                provider("P3193b", "sk-3193-b...bbbb"),
            ],
            ..AppSettings::default()
        };
        save_settings_with_context(&ctx, first).expect("first save");

        // Step 2: Re-save keeping only P3193a — P3193b is deleted.
        let reduced = AppSettings {
            vault_dir: vault,
            provider: provider("Primary3193", ""),
            providers: vec![provider("P3193a", "sk-3193-a...aaaa")],
            ..AppSettings::default()
        };
        save_settings_with_context(&ctx, reduced).expect("reduced save");

        // Step 3: P3193b's keychain entry must have been deleted.
        let removed = crate::keychain::account_key("P3193b");
        let result = KEYCHAIN.get(&removed);
        assert!(
            result.is_err() || result.unwrap().is_none(),
            "Key for deleted provider P3193b still present in keychain (#3193)"
        );

        // Sanity: P3193a's key must still be present.
        let kept = crate::keychain::account_key("P3193a");
        let kept_result = KEYCHAIN.get(&kept);
        assert!(
            kept_result.is_ok() && kept_result.unwrap().map(|v| v == "sk-3193-a...aaaa").unwrap_or(false),
            "Key for surviving provider P3193a should NOT have been deleted"
        );
    }
}
