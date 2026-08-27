//! Regression test for issue #1887: Vault content end-to-end encryption.
//!
//! This test covers the *backend foundation* landed for #1887 — passphrase-
//! derived, portable AES-256-GCM vault encryption
//! (`crypto::encrypt_with_passphrase` / `crypto::decrypt_with_passphrase`).
//! The user-master-passphrase key derivation (PBKDF2-HMAC-SHA256) makes the
//! ciphertext self-describing (carries its own random salt + nonce) so the
//! encrypted bytes travel with the vault and are not bound to a single host,
//! unlike the machine-bound API-key encryption.

use crate::crypto::{
    decrypt_with_passphrase, encrypt_with_passphrase, is_passphrase_encrypted, PASSPHRASE_PREFIX,
};

#[test]
fn regression_1887_round_trip_recovers_plaintext() {
    let plaintext = "# Secret note\nThis is private vault content.";
    let pass = "correct horse battery staple";

    let sealed = encrypt_with_passphrase(plaintext, pass).unwrap();
    assert_ne!(sealed, plaintext);
    assert!(sealed.starts_with(PASSPHRASE_PREFIX));
    assert!(is_passphrase_encrypted(&sealed));

    let opened = decrypt_with_passphrase(&sealed, pass).unwrap();
    assert_eq!(opened, plaintext);
}

#[test]
fn regression_1887_wrong_passphrase_fails() {
    let sealed = encrypt_with_passphrase("top secret", "right-pass").unwrap();
    let result = decrypt_with_passphrase(&sealed, "wrong-pass");
    assert!(
        result.is_err(),
        "decryption with the wrong passphrase must fail (GCM auth tag)"
    );
}

#[test]
fn regression_1887_different_salt_per_call() {
    // Random per-payload salt must make two encryptions of identical input
    // with the same passphrase produce different ciphertext (prevents
    // cross-note equality inference — a #1887 privacy requirement).
    let pass = "shared-passphrase";
    let a = encrypt_with_passphrase("identical note body", pass).unwrap();
    let b = encrypt_with_passphrase("identical note body", pass).unwrap();
    assert_ne!(a, b, "encryptions must not be deterministic across calls");
    assert_eq!(
        decrypt_with_passphrase(&a, pass).unwrap(),
        "identical note body"
    );
    assert_eq!(
        decrypt_with_passphrase(&b, pass).unwrap(),
        "identical note body"
    );
}

#[test]
fn regression_1887_rejects_non_vault_payload() {
    let result = decrypt_with_passphrase("plaintext note", "any");
    assert!(
        result.is_err(),
        "decrypting a non-passphrase payload must error, not silently passthrough"
    );
}

#[test]
fn regression_1887_empty_plaintext_round_trip() {
    let sealed = encrypt_with_passphrase("", "p").unwrap();
    assert_eq!(decrypt_with_passphrase(&sealed, "p").unwrap(), "");
}

#[test]
fn regression_1887_unicode_plaintext_round_trip() {
    let plaintext = "你好，世界 🔐 秘密笔记 — café résumé";
    let sealed = encrypt_with_passphrase(plaintext, "pw").unwrap();
    assert_eq!(decrypt_with_passphrase(&sealed, "pw").unwrap(), plaintext);
}
