//! Machine-bound encryption for sensitive settings (API keys).
//!
//! Uses AES-256-GCM with a key derived from machine-specific identifiers
//! via HKDF-SHA256 (RFC 5869).  The derived key never leaves the host;
//! ciphertext includes
//! a 12-byte random nonce prepended to the payload.  A version prefix
//! (`ENC:v1:`) distinguishes encrypted values from legacy plaintext.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hkdf::Hkdf;
use sha2::Sha256;

/// Prefix that identifies a value as encrypted by this module.
pub const ENCRYPTED_PREFIX: &str = "ENC:v1:";

/// Derive a 256-bit key from machine-specific entropy.
///
/// Sources include hostname, OS architecture, and a fixed application
/// salt.  The result is deterministic for a given host — no external key
/// store is needed.
fn derive_machine_key() -> [u8; 32] {
    let mut ikm = Vec::new();

    // Machine hostname (stable across reboots on most systems).
    if let Ok(name) = hostname::get() {
        ikm.extend_from_slice(name.to_string_lossy().as_bytes());
    }

    // OS and architecture — adds entropy on shared-hostname setups.
    ikm.extend_from_slice(std::env::consts::OS.as_bytes());
    ikm.extend_from_slice(std::env::consts::ARCH.as_bytes());

    // Machine-id on Linux (systemd) and macOS.
    #[cfg(target_os = "linux")]
    {
        if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
            ikm.extend_from_slice(id.trim().as_bytes());
        } else if let Ok(id) = std::fs::read_to_string("/var/lib/dbus/machine-id") {
            ikm.extend_from_slice(id.trim().as_bytes());
        }
    }
    #[cfg(target_os = "macos")]
    {
        // macOS IOPlatformUUID is not directly accessible without IOKit;
        // hostname + arch is sufficient on consumer macOS machines.
    }

    // HKDF (RFC 5869) with domain-separated salt.
    let salt = b"vaultpilot-machine-key-v1";
    let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut key = [0u8; 32];
    hk.expand(b"vaultpilot-aes-256-gcm", &mut key)
        .expect("HKDF expand to 32 bytes should never fail");
    key
}

/// Encrypt a plaintext string, returning `ENC:v1:<base64(nonce||ciphertext)>`.
///
/// The nonce is 12 bytes, generated from the OS CSPRNG.
pub fn encrypt_secret(plaintext: &str) -> Result<String> {
    let key = derive_machine_key();
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow!("failed to create AES-256-GCM cipher: {e}"))?;

    let nonce_bytes: [u8; 12] = {
        use aes_gcm::aead::rand_core::RngCore;
        let mut buf = [0u8; 12];
        OsRng.fill_bytes(&mut buf);
        buf
    };
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow!("AES-GCM encryption failed: {e}"))?;

    // Concatenate nonce + ciphertext, then base64-encode.
    let mut payload = Vec::with_capacity(12 + ciphertext.len());
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&ciphertext);

    Ok(format!("{}{}", ENCRYPTED_PREFIX, B64.encode(payload)))
}

/// Decrypt a value previously produced by [`encrypt_secret`].
///
/// If the value does not start with [`ENCRYPTED_PREFIX`] it is returned
/// as-is (transparent plaintext migration).
pub fn decrypt_secret(value: &str) -> Result<String> {
    let b64_part = value.strip_prefix(ENCRYPTED_PREFIX).unwrap_or(value);

    // If no prefix, treat as legacy plaintext.
    if !value.starts_with(ENCRYPTED_PREFIX) {
        return Ok(value.to_string());
    }

    let decoded = B64
        .decode(b64_part)
        .map_err(|e| anyhow!("failed to decode base64 encrypted secret: {e}"))?;

    if decoded.len() < 12 {
        anyhow::bail!("encrypted payload too short (no room for nonce)");
    }

    let (nonce_bytes, ciphertext) = decoded.split_at(12);

    let key = derive_machine_key();
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow!("failed to create AES-256-GCM cipher: {e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|e| {
        anyhow!("AES-GCM decryption failed — key may have changed or data is corrupt: {e}")
    })?;

    String::from_utf8(plaintext).map_err(|e| anyhow!("decrypted secret is not valid UTF-8: {e}"))
}

/// Returns `true` if the value appears to be encrypted by this module.
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENCRYPTED_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_encrypt_decrypt() {
        let secret = "sk-ant-api03-abcdefghij1234567890";
        let encrypted = encrypt_secret(secret).unwrap();

        // Encrypted value should be different from plaintext.
        assert_ne!(encrypted, secret);
        // Should start with the version prefix.
        assert!(encrypted.starts_with(ENCRYPTED_PREFIX));
        assert!(is_encrypted(&encrypted));

        // Decrypting should recover the original.
        let decrypted = decrypt_secret(&encrypted).unwrap();
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn plaintext_passthrough() {
        let plaintext = "my-old-unencrypted-key";
        let result = decrypt_secret(plaintext).unwrap();
        assert_eq!(result, plaintext);
    }

    #[test]
    fn empty_string_round_trip() {
        let encrypted = encrypt_secret("").unwrap();
        let decrypted = decrypt_secret(&encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn long_key_round_trip() {
        let long_key = "a".repeat(1024);
        let encrypted = encrypt_secret(&long_key).unwrap();
        let decrypted = decrypt_secret(&encrypted).unwrap();
        assert_eq!(decrypted, long_key);
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let encrypted = encrypt_secret("secret").unwrap();
        // Flip a character in the base64 portion.
        let mut tampered = encrypted.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == 'A' { 'B' } else { 'A' });
        let result = decrypt_secret(&tampered);
        assert!(result.is_err());
    }

    #[test]
    fn is_encrypted_detects_prefix() {
        assert!(is_encrypted("ENC:v1:abc"));
        assert!(!is_encrypted("plaintext-key"));
        assert!(!is_encrypted(""));
    }
}
