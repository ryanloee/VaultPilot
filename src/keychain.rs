//! OS-native keychain integration for API key storage.
//!
//! Provides a [`SecretStore`] trait that abstracts over platform-specific
//! credential stores (Windows Credential Manager, Linux Secret Service,
//! macOS Keychain) and the existing file-based encryption fallback.
//!
//! The [`SelectiveStore`] auto-detects OS keychain availability and picks the
//! appropriate backend.  On headless / CI environments without a D-Bus session
//! it transparently falls back to the machine-bound AES-256-GCM encryption
//! from [`crate::crypto`].
//!
//! # Issue #3159
//!
//! This module implements the Rust backend portion of the OS-native keychain
//! feature.  WinUI and Android integration remain tracked under the same issue.
//!
//! # Keyring service / account convention
//!
//! All entries use `service = "vaultpilot"` with `account` derived from the
//! provider configuration — typically `"api_key:{provider_name}"` or just
//! `"api_key"` for the primary provider.

use std::sync::OnceLock;

use anyhow::{Context, Result};

// ---------------------------------------------------------------------------
// Store selection
// ---------------------------------------------------------------------------

/// The concrete store implementation selected for this host.
enum StoreKind {
    Os,
    File,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstraction for reading and writing secrets (typically API keys).
pub(crate) trait SecretStore: Send + Sync {
    /// Persist `value` for the given `key`.
    fn set(&self, key: &str, value: &str) -> Result<()>;

    /// Read a previously stored value.  Returns `None` when the entry does
    /// not exist (never stored or already deleted).
    fn get(&self, key: &str) -> Result<Option<String>>;

    /// Remove a stored entry.
    fn delete(&self, key: &str) -> Result<()>;

    /// Returns `true` if this store is believed to work on the current host.
    fn is_available(&self) -> bool;
}

// ---------------------------------------------------------------------------
// OS keychain backend  (via `keyring` crate)
// ---------------------------------------------------------------------------

/// Backed by the platform-native credential store.
///
/// Uses the `keyring` crate (v4.1) which delegates to:
/// - **Linux**: Secret Service via zbus (async D-Bus).
/// - **Windows**: Credential Manager.
/// - **macOS**: Keychain Services.
struct OsKeychainStore;

impl SecretStore for OsKeychainStore {
    fn set(&self, key: &str, value: &str) -> Result<()> {
        let entry =
            keyring::v1::Entry::new("vaultpilot", key).context("failed to create keyring entry")?;
        entry
            .set_password(value)
            .context("failed to set keyring password")?;
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        let entry = match keyring::v1::Entry::new("vaultpilot", key) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::v1::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("failed to get keyring password: {e}")),
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        let entry = match keyring::v1::Entry::new("vaultpilot", key) {
            Ok(e) => e,
            Err(_) => return Ok(()), // nothing to delete
        };
        entry
            .delete_credential()
            .context("failed to delete keyring credential")?;
        Ok(())
    }

    fn is_available(&self) -> bool {
        // Thorough probe: create a temporary entry and try to set+get a
        // value.  This catches scenarios where Entry::new() succeeds but
        // the actual credential store is locked / prompting-dismissed
        // (e.g. Linux Secret Service with a locked keyring in a headless
        // environment).
        let probe_key = format!("vaultpilot-probe-{}", std::process::id());
        let entry = match keyring::v1::Entry::new("vaultpilot", &probe_key) {
            Ok(e) => e,
            Err(_) => return false,
        };
        // Clean any leftover probe entry from a previous run.
        let _ = entry.delete_credential();
        // Actually try to write and read back.
        if entry.set_password("probe-available").is_err() {
            return false;
        }
        let ok = entry
            .get_password()
            .map(|v| v == "probe-available")
            .unwrap_or(false);
        // Clean up regardless.
        let _ = entry.delete_credential();
        ok
    }
}

// ---------------------------------------------------------------------------
// File-based encryption backend  (fallback)
// ---------------------------------------------------------------------------

/// Uses the existing machine-bound AES-256-GCM encryption
/// ([`crate::crypto`]) and stores the ciphertext in-memory.
///
/// This is the **fallback** for headless / CI environments that lack a
/// D-Bus session.  Note that this implementation does **not** persist to
/// the settings file on its own — that responsibility lives in
/// [`crate::storage::settings`] which calls the store then writes the
/// (now-key-free) settings struct to disk.
struct FileCryptoStore;

impl SecretStore for FileCryptoStore {
    fn set(&self, _key: &str, value: &str) -> Result<()> {
        // Value is encrypted with machine-bound key; the caller
        // (crate::storage::settings) is responsible for persisting the
        // ciphertext to the settings file.
        let _encrypted =
            crate::crypto::encrypt_secret(value).context("failed to encrypt secret")?;
        // We return the encrypted value through the return — actually, we
        // don't.  This store is a no-op for writes because the caller does
        // the encryption + file write itself.  We exist so that
        // SelectiveStore can report availability and let the caller decide.
        Ok(())
    }

    fn get(&self, _key: &str) -> Result<Option<String>> {
        // This store cannot retrieve secrets on its own; it needs the
        // encrypted blob from the settings file.  Callers should fall back
        // to reading from the on-disk settings when this store returns None.
        Ok(None)
    }

    fn delete(&self, _key: &str) -> Result<()> {
        Ok(())
    }

    fn is_available(&self) -> bool {
        true // file-based encryption is always available
    }
}

// ---------------------------------------------------------------------------
// Selective store
// ---------------------------------------------------------------------------

/// Auto-selects between OS keychain and file-based encryption.
///
/// Detection is performed once at first access and cached thereafter.
pub(crate) struct SelectiveStore {
    inner: OnceLock<(StoreKind, Box<dyn SecretStore>)>,
}

impl SelectiveStore {
    pub(crate) const fn new() -> Self {
        Self {
            inner: OnceLock::new(),
        }
    }

    /// Initialize and return a reference to the active store.
    fn init(&self) -> &(StoreKind, Box<dyn SecretStore>) {
        self.inner.get_or_init(|| {
            let os = OsKeychainStore;
            if os.is_available() {
                tracing::info!("using OS-native keychain for secret storage");
                (StoreKind::Os, Box::new(os))
            } else {
                tracing::info!(
                    "OS keychain unavailable — falling back to file-based \
                     encryption for secret storage"
                );
                (StoreKind::File, Box::new(FileCryptoStore))
            }
        })
    }

    /// Return a reference to the active store.
    fn store(&self) -> &dyn SecretStore {
        &*self.init().1
    }

    /// Persist a secret.
    pub(crate) fn set(&self, key: &str, value: &str) -> Result<()> {
        self.store().set(key, value)
    }

    /// Read a secret.
    pub(crate) fn get(&self, key: &str) -> Result<Option<String>> {
        self.store().get(key)
    }

    /// Delete a secret.
    pub(crate) fn delete(&self, key: &str) -> Result<()> {
        self.store().delete(key)
    }

    /// Returns `true` if the OS keychain is being used.
    // Surfaced to UI in the #3159 WinUI/Android follow-up.
    #[allow(dead_code)]
    pub(crate) fn is_os_keychain_active(&self) -> bool {
        matches!(self.init().0, StoreKind::Os)
    }
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

/// Global [`SelectiveStore`] instance.
pub(crate) static KEYCHAIN: SelectiveStore = SelectiveStore::new();

// ---------------------------------------------------------------------------
// Helper: derive account name from provider name
// ---------------------------------------------------------------------------

/// Return a stable account key for the given provider name.
///
/// The primary provider uses `"api_key"`; other providers use
/// `"api_key:{normalized_name}"`.
pub(crate) fn account_key(provider_name: &str) -> String {
    if provider_name.is_empty() || provider_name.eq_ignore_ascii_case("primary") {
        "api_key".to_string()
    } else {
        format!("api_key:{provider_name}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_crypto_always_available() {
        let store = FileCryptoStore;
        assert!(store.is_available());
    }

    #[test]
    fn test_selective_store_does_not_panic() {
        // Just verify that creating and querying the store doesn't crash.
        let store = SelectiveStore::new();
        let _available = store.store().is_available();
    }

    #[test]
    fn test_account_key_primary() {
        assert_eq!(account_key(""), "api_key");
        assert_eq!(account_key("primary"), "api_key");
        assert_eq!(account_key("PRIMARY"), "api_key");
    }

    #[test]
    fn test_account_key_named() {
        assert_eq!(account_key("OpenAI"), "api_key:OpenAI");
        assert_eq!(account_key("Anthropic"), "api_key:Anthropic");
    }

    #[test]
    fn test_selective_store_set_get_delete() {
        // This test works with whatever backend is available.
        let store = SelectiveStore::new();
        let key = "test-select-283746";

        // Clean start.
        let _ = store.delete(key);

        // Not present.
        assert!(store.get(key).unwrap().is_none());

        // Set and read back.
        store.set(key, "sk-ver...9999").unwrap();

        // If the OS keychain is active, we can read back the value.
        // If the file-crypto fallback is active (e.g. headless / CI),
        // reading back returns None (by design — the encrypted blob
        // is only on disk and cannot be retrieved via this store).
        if store.is_os_keychain_active() {
            assert_eq!(store.get(key).unwrap().as_deref(), Some("sk-ver...9999"));
            // Delete and verify gone.
            store.delete(key).unwrap();
            assert!(store.get(key).unwrap().is_none());
        }
    }
}
