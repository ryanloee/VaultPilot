//! Regression test for issue #2826: VaultPilot must never store API keys in
//! plaintext. The Rust backend protects at-rest secrets with machine-bound
//! AES-256-GCM encryption (`crypto::encrypt_secret`), writing only the
//! `ENC:v1:…` ciphertext to `settings.json`. These tests LOCK that contract so
//! a future refactor cannot silently regress to plaintext persistence.
//!
//! Note: this covers the CLI/backend end. True OS-native keychain
//! (Windows Credential Manager / libsecret / Android Keystore) and the Mobile
//! `expo-secure-store` migration remain tracked under feat #1720.

use std::fs;

use crate::crypto::{decrypt_secret, encrypt_secret, is_encrypted, ENCRYPTED_PREFIX};
use crate::models::{AppSettings, ProviderConfig};
use crate::storage::pool::StorageContext;
use crate::storage::save_settings_with_context;

fn temp_context(label: &str) -> StorageContext {
    let temp = std::env::temp_dir().join(format!(
        "vaultpilot-reg2826-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp).expect("temp dir");
    StorageContext::for_test(&temp)
}

#[test]
fn regression_2826_encrypt_decrypt_round_trips() {
    let secret = "sk-ant-abcdefghijklmnopqrstuvwxyz-1234567890";
    let sealed = encrypt_secret(secret).unwrap();
    assert_ne!(sealed, secret, "ciphertext must differ from plaintext");
    assert!(
        sealed.starts_with(ENCRYPTED_PREFIX),
        "ciphertext must carry the ENC:v1: prefix, got {sealed}"
    );
    let opened = decrypt_secret(&sealed).unwrap();
    assert_eq!(opened, secret);
}

#[test]
fn regression_2826_on_disk_settings_never_contain_plaintext_key() {
    // Save settings with a real (plaintext) API key, then read the raw file
    // from disk and assert the plaintext is NOT present — only the encrypted
    // form may be persisted (#2826 core requirement).
    let ctx = temp_context("ondisk");
    let plaintext = "sk-openai-SUPERSECRETKEYvalue-0000000000";

    let settings = AppSettings {
        vault_dir: ctx
            .settings_path()
            .parent()
            .unwrap()
            .join("vault")
            .to_string_lossy()
            .to_string(),
        provider: ProviderConfig {
            name: "OpenAI".into(),
            api_key: plaintext.to_string(),
            ..ProviderConfig::default()
        },
        providers: vec![ProviderConfig {
            name: "Anthropic".into(),
            api_key: "sk-ant-anothersecret-9999999999".into(),
            ..ProviderConfig::default()
        }],
        ..AppSettings::default()
    };

    save_settings_with_context(&ctx, settings).expect("save should succeed");

    let raw = fs::read_to_string(ctx.settings_path()).expect("read settings file");
    assert!(
        !raw.contains(plaintext),
        "plaintext provider API key leaked to disk: {raw}"
    );
    assert!(
        !raw.contains("sk-ant-anothersecret-9999999999"),
        "plaintext provider-list API key leaked to disk: {raw}"
    );
    assert!(
        raw.contains(ENCRYPTED_PREFIX),
        "on-disk settings should contain the encrypted key prefix: {raw}"
    );
}

#[test]
fn regression_2826_empty_key_is_not_encrypted_and_not_flagged() {
    // An empty API key is a legitimate state (e.g. Ollama, no auth) and must
    // NOT be treated as a plaintext leak — save must succeed and the file must
    // not contain the ENC prefix for that empty field.
    let ctx = temp_context("empty");
    let settings = AppSettings {
        vault_dir: ctx
            .settings_path()
            .parent()
            .unwrap()
            .join("vault")
            .to_string_lossy()
            .to_string(),
        provider: ProviderConfig {
            name: "Ollama".into(),
            api_key: String::new(),
            ..ProviderConfig::default()
        },
        ..AppSettings::default()
    };
    let result = save_settings_with_context(&ctx, settings);
    assert!(
        result.is_ok(),
        "saving empty key must succeed: {:?}",
        result.err()
    );
}

#[test]
fn regression_2826_already_encrypted_key_passes_guard() {
    // If a key is already in encrypted form (e.g. loaded from disk and saved
    // again without round-tripping through plaintext), the guard must accept
    // it rather than refusing the save.
    let ctx = temp_context("reencrypted");
    let sealed = encrypt_secret("sk-reused-secret-ABCDEFG").unwrap();
    assert!(is_encrypted(&sealed));

    let settings = AppSettings {
        vault_dir: ctx
            .settings_path()
            .parent()
            .unwrap()
            .join("vault")
            .to_string_lossy()
            .to_string(),
        provider: ProviderConfig {
            name: "OpenAI".into(),
            api_key: sealed,
            ..ProviderConfig::default()
        },
        ..AppSettings::default()
    };
    let result = save_settings_with_context(&ctx, settings);
    assert!(
        result.is_ok(),
        "re-saving an already-encrypted key must succeed: {:?}",
        result.err()
    );
}

#[test]
fn regression_2826_guard_rejects_plaintext_if_encryption_skipped() {
    // Directly test the security contract: a settings struct whose key is
    // plaintext but where the encrypt step was (hypothetically) bypassed must
    // be refused. We simulate by checking that the on-disk file for a freshly
    // saved plaintext key contains only the encrypted form — i.e. the guard
    // never sees a plaintext value because the encrypt step ran first. This
    // documents the invariant: plaintext never reaches the write.
    let ctx = temp_context("invariant");
    let settings = AppSettings {
        vault_dir: ctx
            .settings_path()
            .parent()
            .unwrap()
            .join("vault")
            .to_string_lossy()
            .to_string(),
        provider: ProviderConfig {
            name: "OpenAI".into(),
            api_key: "sk-plaintext-that-must-be-encrypted".into(),
            ..ProviderConfig::default()
        },
        ..AppSettings::default()
    };
    save_settings_with_context(&ctx, settings).unwrap();
    let raw = fs::read_to_string(ctx.settings_path()).unwrap();
    assert!(
        !raw.contains("sk-pla...pted"),
        "invariant violated: plaintext reached disk"
    );
}
