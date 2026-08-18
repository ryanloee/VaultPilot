//! Machine-bound encryption for sensitive settings (API keys).
//!
//! Uses AES-256-GCM with a key derived from machine-specific identifiers
//! via PBKDF2-HMAC-SHA256 (600,000 iterations).  The derived key never
//! leaves the host; ciphertext includes a 12-byte random nonce prepended
//! to the payload.  A version prefix (`ENC:v1:`) distinguishes encrypted
//! values from legacy plaintext.

use aes_gcm::{
    aead::{Aead, Generate, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use std::sync::OnceLock;

/// Prefix that identifies a value as encrypted by this module.
pub const ENCRYPTED_PREFIX: &str = "ENC:v1:";

/// Number of PBKDF2 iterations for key derivation.
/// OWASP recommends ≥ 600,000 for PBKDF2-SHA256 (2023).
const PBKDF2_ITERATIONS: u32 = 600_000;

/// PBKDF2-HMAC-SHA256 key derivation (RustCrypto `pbkdf2` crate).
fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password, salt, iterations, &mut key);
    key
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
                        // Verify write succeeded; if it fails (common in macOS
                        // App Sandbox), fall back to a deterministic salt based
                        // on the HOME path so that the derived key remains
                        // stable across restarts (fixes #1676).
                        if std::fs::write(&id_path, &new_id).is_ok()
                            && std::fs::read_to_string(&id_path)
                                .map(|s| s.trim().to_string())
                                .ok()
                                .as_deref()
                                == Some(new_id.as_str())
                        {
                            salt.extend_from_slice(new_id.as_bytes());
                        } else {
                            tracing::warn!(
                                "Failed to persist .machine-id, \
                                 using deterministic fallback"
                            );
                            salt.extend_from_slice(home.to_string_lossy().as_bytes());
                            salt.extend_from_slice(b"vaultpilot-macos-fallback-v1");
                        }
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

    let nonce = Nonce::generate();

    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow!("AES-GCM encryption failed: {e}"))?;

    // Concatenate nonce + ciphertext, then base64-encode.
    let mut payload = Vec::with_capacity(12 + ciphertext.len());
    payload.extend_from_slice(&nonce[..]);
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
    let nonce = Nonce::try_from(nonce_bytes).map_err(|e| anyhow!("invalid nonce: {e}"))?;

    let plaintext = cipher.decrypt(&nonce, ciphertext).map_err(|_| {
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

// ── User-passphrase vault encryption (issue #1887) ────────────────────────
//
// The machine-bound [`encrypt_secret`] above binds the key to a specific host,
// which is ideal for API keys but useless for *portable* vault content: a user
// who moves their vault to another machine (or restores a backup) can no longer
// decrypt it.  Issue #1887 (Vault content end-to-end encryption) requires the
// key to be derived from a **user master passphrase** instead, so the encrypted
// bytes are self-contained and travel with the vault.
//
// Scheme: `ENC:v2:<base64(salt[16] || nonce[12] || ciphertext)>`
//   - salt is random per payload, so two encryptions of the same plaintext
//     with the same passphrase produce different ciphertexts (prevents
//     replay / equality inference across notes).
//   - key = PBKDF2-HMAC-SHA256(passphrase, salt, PBKDF2_ITERATIONS)
//   - payload is AES-256-GCM sealed under that key with a random 12-byte nonce.

/// Prefix that identifies a vault payload encrypted with a user passphrase
/// by this module (distinct from the machine-bound [`ENCRYPTED_PREFIX`]).
pub const PASSPHRASE_PREFIX: &str = "ENC:v2:";

/// Salt length (bytes) for passphrase-derived vault encryption.
const VAULT_SALT_LEN: usize = 16;

/// Encrypt `plaintext` under a user-supplied `passphrase`.
///
/// The result is fully self-describing (carries its own random salt + nonce)
/// and decodable on any host given the correct passphrase, which is the
/// foundation for issue #1887's portable vault-level encryption.
pub fn encrypt_with_passphrase(plaintext: &str, passphrase: &str) -> Result<String> {
    // Use a v4 UUID (16 random bytes) as the per-payload salt.  `uuid` with
    // the `v4` feature is already a workspace dependency, so no new crate is
    // needed; this keeps the salt generation aligned with the existing
    // randomness source used elsewhere in the crate.
    let salt = *uuid::Uuid::new_v4().as_bytes();

    let key = pbkdf2_hmac_sha256(passphrase.as_bytes(), &salt, PBKDF2_ITERATIONS);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow!("failed to create AES-256-GCM cipher: {e}"))?;

    let nonce = Nonce::generate();
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow!("AES-GCM vault encryption failed: {e}"))?;

    let mut payload = Vec::with_capacity(VAULT_SALT_LEN + 12 + ciphertext.len());
    payload.extend_from_slice(&salt);
    payload.extend_from_slice(&nonce[..]);
    payload.extend_from_slice(&ciphertext);

    Ok(format!("{}{}", PASSPHRASE_PREFIX, B64.encode(payload)))
}

/// Decrypt a value produced by [`encrypt_with_passphrase`] using `passphrase`.
///
/// Returns `Err` if the value is not passphrase-encrypted or if the passphrase
/// is wrong (AES-GCM authentication fails).  The latter is indistinguishable
/// from corruption, which is the desired property for vault encryption.
pub fn decrypt_with_passphrase(value: &str, passphrase: &str) -> Result<String> {
    if !value.starts_with(PASSPHRASE_PREFIX) {
        return Err(anyhow!(
            "value is not passphrase-encrypted (missing '{}' prefix)",
            PASSPHRASE_PREFIX
        ));
    }

    let b64_part = &value[PASSPHRASE_PREFIX.len()..];
    let decoded = B64
        .decode(b64_part)
        .map_err(|e| anyhow!("ENC:v2: value is not valid base64: {e}"))?;

    if decoded.len() < VAULT_SALT_LEN + 12 {
        return Err(anyhow!(
            "ENC:v2: payload too short ({} bytes, need >= {})",
            decoded.len(),
            VAULT_SALT_LEN + 12
        ));
    }

    let (salt, rest) = decoded.split_at(VAULT_SALT_LEN);
    let (nonce_bytes, ciphertext) = rest.split_at(12);

    let key = pbkdf2_hmac_sha256(passphrase.as_bytes(), salt, PBKDF2_ITERATIONS);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow!("failed to create AES-256-GCM cipher: {e}"))?;
    let nonce = Nonce::try_from(nonce_bytes).map_err(|e| anyhow!("invalid nonce: {e}"))?;

    let plaintext = cipher.decrypt(&nonce, ciphertext).map_err(|_| {
        anyhow!("vault decryption failed — wrong passphrase or corrupted ciphertext")
    })?;

    String::from_utf8(plaintext)
        .map_err(|e| anyhow!("decrypted vault content is not valid UTF-8: {e}"))
}

/// Returns `true` if `value` is encrypted with a user passphrase
/// (vs. the machine-bound [`is_encrypted`] form).
pub fn is_passphrase_encrypted(value: &str) -> bool {
    value.starts_with(PASSPHRASE_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_low_iterations() {
        // RFC 6070 known vector for PBKDF2-HMAC-SHA256 with c=1:
        //   password = "password", salt = "salt"
        //   output   = 120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b
        let dk = pbkdf2_hmac_sha256(b"password", b"salt", 1);
        let expected: [u8; 32] = [
            0x12, 0x0f, 0xb6, 0xcf, 0xfc, 0xf8, 0xb3, 0x2c, 0x43, 0xe7, 0x22, 0x52, 0x56, 0xc4,
            0xf8, 0x37, 0xa8, 0x65, 0x48, 0xc9, 0x2c, 0xcc, 0x35, 0x48, 0x08, 0x05, 0x98, 0x7c,
            0xb7, 0x0b, 0xe1, 0x7b,
        ];
        assert_eq!(dk, expected);
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
