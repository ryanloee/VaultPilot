//! Machine-bound encryption for sensitive settings (API keys).
//!
//! Uses AES-256-GCM with a key derived from machine-specific identifiers
//! via PBKDF2-HMAC-SHA256 (600,000 iterations).  The derived key never
//! leaves the host; ciphertext includes a 12-byte random nonce prepended
//! to the payload.  A version prefix (`ENC:v1:`) distinguishes encrypted
//! values from legacy plaintext.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Prefix that identifies a value as encrypted by this module.
pub const ENCRYPTED_PREFIX: &str = "ENC:v1:";

/// Number of PBKDF2 iterations for key derivation.
/// OWASP recommends ≥ 600,000 for PBKDF2-SHA256 (2023).
const PBKDF2_ITERATIONS: u32 = 600_000;

/// HMAC-SHA256 using only sha2 (no external hmac crate dependency).
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    // Block size for SHA-256 is 64 bytes.
    const BLOCK_SIZE: usize = 64;

    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hash = Sha256::digest(key);
        key_block[..32].copy_from_slice(&hash);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    // ipad and opad
    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let inner_hash = Sha256::new()
        .chain_update(ipad)
        .chain_update(message)
        .finalize();

    Sha256::new()
        .chain_update(opad)
        .chain_update(inner_hash)
        .finalize()
        .into()
}

/// PBKDF2-HMAC-SHA256 key derivation.
fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let u1 = hmac_sha256(password, &{
        let mut buf = Vec::with_capacity(salt.len() + 4);
        buf.extend_from_slice(salt);
        buf.extend_from_slice(&1u32.to_be_bytes());
        buf
    });

    let mut result = u1;
    let mut u_prev = u1;

    for i in 2..=iterations {
        let u_next = hmac_sha256(password, &u_prev);
        for j in 0..32 {
            result[j] ^= u_next[j];
        }
        u_prev = u_next;

        // Progress indicator every 100k iterations (only in debug builds).
        #[cfg(debug_assertions)]
        {
            if i % 100_000 == 0 {
                eprintln!("PBKDF2: {i}/{iterations} iterations...");
            }
        }
    }

    result
}

/// Build the salt from machine-specific entropy.
fn machine_salt() -> Vec<u8> {
    let mut salt = Vec::with_capacity(256);

    // Fixed application salt to namespace the derivation.
    salt.extend_from_slice(b"vaultpilot-api-key-encryption-v2:");

    // Machine hostname (stable across reboots on most systems).
    if let Ok(name) = hostname::get() {
        salt.extend_from_slice(name.to_string_lossy().as_bytes());
    }

    // OS and architecture — adds entropy on shared-hostname setups.
    salt.extend_from_slice(std::env::consts::OS.as_bytes());
    salt.extend_from_slice(std::env::consts::ARCH.as_bytes());

    // Machine-id on Linux (systemd) and macOS.
    #[cfg(target_os = "linux")]
    {
        if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
            salt.extend_from_slice(id.trim().as_bytes());
        } else if let Ok(id) = std::fs::read_to_string("/var/lib/dbus/machine-id") {
            salt.extend_from_slice(id.trim().as_bytes());
        }
    }
    // Windows MachineGuid — a per-installation unique identifier stored in
    // the registry, equivalent to Linux's /etc/machine-id.  Without this,
    // two Windows machines sharing the same hostname would derive identical
    // encryption keys (#595).
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Cryptography').MachineGuid",
            ])
            .output()
        {
            if output.status.success() {
                let guid = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !guid.is_empty() {
                    salt.extend_from_slice(guid.as_bytes());
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        // IOPlatformUUID — the macOS equivalent of /etc/machine-id or
        // Windows MachineGuid.  Readable without IOKit via ioreg(8).
        let ioreg_uuid = Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    // Extract the value of "IOPlatformUUID" = "XXXX-..."
                    stdout
                        .lines()
                        .find(|l| l.contains("IOPlatformUUID"))
                        .and_then(|line| {
                            // Format:  |   "IOPlatformUUID" = "XXXXXXXX-XXXX-..."
                            line.split('"').nth(3).map(String::from)
                        })
                } else {
                    None
                }
            });

        if let Some(uuid) = ioreg_uuid {
            if !uuid.is_empty() {
                salt.extend_from_slice(uuid.as_bytes());
            }
        } else {
            // Fallback: persistent random UUID stored in the app support dir.
            // This handles sandboxed environments where ioreg may be
            // unavailable (e.g., App Sandbox, CI runners).
            if let Some(home) = std::env::var_os("HOME") {
                let id_path = std::path::PathBuf::from(&home)
                    .join("Library/Application Support/VaultPilot/.machine-id");
                match std::fs::read_to_string(&id_path) {
                    Ok(id) if !id.trim().is_empty() => {
                        salt.extend_from_slice(id.trim().as_bytes());
                    }
                    _ => {
                        // Generate a random UUID and persist it.
                        let new_id = uuid::Uuid::new_v4().to_string();
                        // Best-effort: create dir and write file.
                        if let Some(dir) = id_path.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        let _ = std::fs::write(&id_path, &new_id);
                        salt.extend_from_slice(new_id.as_bytes());
                    }
                }
            }
        }
    }

    salt
}

/// Derive a 256-bit key from machine-specific entropy using PBKDF2-HMAC-SHA256.
///
/// Uses 600,000 iterations per OWASP 2023 recommendations.  The result is
/// deterministic for a given host — no external key store is needed.
/// Cached machine key — derived once per process lifetime.
/// The inputs (hostname, OS, machine-id) do not change while the process
/// is running, so caching is safe and avoids ~600ms PBKDF2 on every call.
static MACHINE_KEY: OnceLock<[u8; 32]> = OnceLock::new();

fn derive_machine_key() -> [u8; 32] {
    *MACHINE_KEY.get_or_init(|| {
        let salt = machine_salt();
        // Password is the fixed application identifier (the salt carries the entropy).
        pbkdf2_hmac_sha256(b"vaultpilot-machine-key", &salt, PBKDF2_ITERATIONS)
    })
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
///
/// If the value starts with the prefix but decryption fails (e.g., the
/// machine key has changed), an error is returned so callers can
/// distinguish between successful decryption and failure (#867).
pub fn decrypt_secret(value: &str) -> Result<String> {
    // If no prefix, treat as legacy plaintext.
    if !value.starts_with(ENCRYPTED_PREFIX) {
        return Ok(value.to_string());
    }

    let b64_part = &value[ENCRYPTED_PREFIX.len()..];

    let decoded = B64
        .decode(b64_part)
        .map_err(|e| anyhow!("ENC:v1: value is not valid base64: {e}"))?;

    if decoded.len() < 12 {
        return Err(anyhow!(
            "ENC:v1: payload too short ({decoded_len} bytes, need >= 12)",
            decoded_len = decoded.len()
        ));
    }

    let (nonce_bytes, ciphertext) = decoded.split_at(12);

    let key = derive_machine_key();
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow!("failed to create AES-256-GCM cipher: {e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        anyhow!(
            "AES-GCM decryption failed — the machine key may have changed; \
             please re-enter your API key in Settings"
        )
    })?;

    String::from_utf8(plaintext).map_err(|e| anyhow!("decrypted secret is not valid UTF-8: {e}"))
}

/// Like [`decrypt_secret`], but returns the raw value as-is when
/// decryption fails instead of propagating the error.  Use this in
/// contexts where a graceful fallback is acceptable (e.g. display).
pub fn decrypt_secret_lossy(value: &str) -> String {
    decrypt_secret(value).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "decrypt_secret_lossy: returning raw value");
        value.to_string()
    })
}

/// Returns `true` if the value appears to be encrypted by this module.
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENCRYPTED_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_known_vector() {
        // RFC 4231 Test Case 2
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let mac = hmac_sha256(key, data);
        // Expected: 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843
        let expected: [u8; 32] = [
            0x5b, 0xdc, 0xc1, 0x46, 0xbf, 0x60, 0x75, 0x4e, 0x6a, 0x04, 0x24, 0x26, 0x08, 0x95,
            0x75, 0xc7, 0x5a, 0x00, 0x3f, 0x08, 0x9d, 0x27, 0x39, 0x83, 0x9d, 0xec, 0x58, 0xb9,
            0x64, 0xec, 0x38, 0x43,
        ];
        assert_eq!(mac, expected);
    }

    #[test]
    fn pbkdf2_low_iterations() {
        // Quick sanity test with low iterations
        let dk = pbkdf2_hmac_sha256(b"password", b"salt", 1);
        // Just verify it produces a deterministic 32-byte output
        assert_eq!(dk.len(), 32);
        let dk2 = pbkdf2_hmac_sha256(b"password", b"salt", 1);
        assert_eq!(dk, dk2);
        // Different input → different output
        let dk3 = pbkdf2_hmac_sha256(b"password", b"salt2", 1);
        assert_ne!(dk, dk3);
    }

    #[test]
    fn round_trip_encrypt_decrypt() {
        let secret = "sk-ant...7890";
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
    fn tampered_ciphertext_returns_error() {
        let encrypted = encrypt_secret("secret").unwrap();
        // Flip a character in the base64 portion.
        let mut tampered = encrypted.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == 'A' { 'B' } else { 'A' });
        // #867: Tampered ciphertext should return an error so callers can
        // distinguish decryption failure from success.
        let result = decrypt_secret(&tampered);
        assert!(
            result.is_err(),
            "decryption of tampered ciphertext must fail"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("decryption failed")
                || err_msg.contains("not valid UTF-8")
                || err_msg.contains("not valid base64")
                || err_msg.contains("too short"),
            "error should indicate decryption failure, got: {err_msg}"
        );
    }

    #[test]
    fn decrypt_secret_lossy_returns_raw_on_failure() {
        let encrypted = encrypt_secret("secret").unwrap();
        let mut tampered = encrypted.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == 'A' { 'B' } else { 'A' });
        // decrypt_secret_lossy returns the raw value on failure
        let result = decrypt_secret_lossy(&tampered);
        assert_eq!(result, tampered);
    }

    #[test]
    fn invalid_base64_returns_error() {
        let bad = format!("{}not-valid-base64!!!", ENCRYPTED_PREFIX);
        let result = decrypt_secret(&bad);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not valid base64"));
    }

    #[test]
    fn payload_too_short_returns_error() {
        // Encode just 5 bytes (< 12 required for nonce)
        let short_payload = format!("{}{}", ENCRYPTED_PREFIX, B64.encode([0u8; 5]));
        let result = decrypt_secret(&short_payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn is_encrypted_detects_prefix() {
        assert!(is_encrypted("ENC:v1:abc"));
        assert!(!is_encrypted("plaintext-key"));
        assert!(!is_encrypted(""));
    }

    #[test]
    fn machine_salt_is_deterministic() {
        // Calling machine_salt() twice must return the same bytes.
        let s1 = machine_salt();
        let s2 = machine_salt();
        assert_eq!(s1, s2);
    }

    #[test]
    fn machine_salt_contains_application_namespace() {
        let salt = machine_salt();
        assert!(
            salt.windows(b"vaultpilot-api-key-encryption-v2:".len())
                .any(|w| w == b"vaultpilot-api-key-encryption-v2:"),
            "machine_salt must contain the application namespace prefix"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn machine_salt_contains_linux_machine_id() {
        let salt = machine_salt();
        // On Linux, salt should include /etc/machine-id content.
        if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
            let id = id.trim();
            if !id.is_empty() {
                assert!(
                    salt.windows(id.len()).any(|w| w == id.as_bytes()),
                    "machine_salt must contain /etc/machine-id on Linux"
                );
            }
        }
    }

    #[test]
    fn machine_salt_contains_hostname() {
        let salt = machine_salt();
        if let Ok(name) = hostname::get() {
            let name = name.to_string_lossy();
            if !name.is_empty() {
                assert!(
                    salt.windows(name.len()).any(|w| w == name.as_bytes()),
                    "machine_salt must contain the hostname"
                );
            }
        }
    }
}
