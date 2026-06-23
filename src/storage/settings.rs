use std::fs;

use anyhow::{Context, Result};
use tracing::warn;

use crate::models::AppSettings;

use super::atomic_write;
use super::pool::{AppPaths, StorageContext};

pub(super) fn normalize_settings(settings: &mut AppSettings, paths: &AppPaths) {
    if let Some(vault_dir_override) = &paths.vault_dir_override {
        settings.vault_dir = vault_dir_override.to_string_lossy().to_string();
    } else if settings.vault_dir.trim().is_empty() {
        settings.vault_dir = paths.default_vault_dir.to_string_lossy().to_string();
    }
    if settings.provider.base_url.trim().is_empty() {
        settings.provider.base_url = crate::models::default_base_url();
    }
    if settings.provider.model.trim().is_empty() {
        settings.provider.model = crate::models::default_model();
    }
    if settings.provider.request_timeout_ms == 0 {
        settings.provider.request_timeout_ms = crate::models::default_timeout_ms();
    }
    if matches!(settings.provider.context_window_tokens, Some(0)) {
        settings.provider.context_window_tokens = None;
    }
    // Normalize each provider in the multi-provider list.
    for p in &mut settings.providers {
        if p.base_url.trim().is_empty() {
            p.base_url = crate::models::default_base_url();
        }
        if p.model.trim().is_empty() {
            p.model = crate::models::default_model();
        }
        if p.request_timeout_ms == 0 {
            p.request_timeout_ms = crate::models::default_timeout_ms();
        }
        if matches!(p.context_window_tokens, Some(0)) {
            p.context_window_tokens = None;
        }
    }
    // Clamp active_provider_index.
    if !settings.providers.is_empty() && settings.active_provider_index >= settings.providers.len()
    {
        settings.active_provider_index = settings.providers.len() - 1;
    }
}

pub fn load_settings_with_context(context: &StorageContext) -> Result<AppSettings> {
    // Return cached settings if available, avoiding redundant disk I/O.
    let cache = context
        .cached_settings
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(ref settings) = *cache {
        return Ok(settings.clone());
    }
    drop(cache);

    let paths = &context.paths;
    if let Some(parent) = paths.settings_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = paths.database_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let settings = if paths.settings_path.exists() {
        let raw = fs::read_to_string(&paths.settings_path)
            .with_context(|| format!("failed to read {}", paths.settings_path.display()))?;
        let normalized = raw.trim_start_matches('\u{feff}');
        let mut parsed: AppSettings = serde_json::from_str(normalized)
            .with_context(|| format!("failed to parse {}", paths.settings_path.display()))?;

        // Decrypt API key if it was stored encrypted.
        // #867: Propagate decryption errors so callers can distinguish
        // between "no key" and "key present but undecryptable".
        if !parsed.provider.api_key.is_empty() {
            parsed.provider.api_key = crate::crypto::decrypt_secret(&parsed.provider.api_key)
                .context("Failed to decrypt stored API key — the machine key may have changed. Please re-enter your API key in Settings")?;
        }
        // Decrypt keys in multi-provider list.
        for p in &mut parsed.providers {
            if !p.api_key.is_empty() {
                p.api_key = crate::crypto::decrypt_secret(&p.api_key)
                    .context("Failed to decrypt provider API key")?;
            }
        }
        // Migrate legacy single provider into providers list.
        parsed.migrate_providers();

        normalize_settings(&mut parsed, paths);
        let warnings = parsed.validate();
        for w in &warnings {
            warn!("{w}");
        }
        parsed
    } else {
        let mut defaults = AppSettings::default();
        normalize_settings(&mut defaults, paths);
        save_settings_with_context(context, defaults.clone())?;
        defaults
    };

    fs::create_dir_all(&settings.vault_dir)?;
    // Cache the parsed settings for future calls.
    {
        let mut cache = context
            .cached_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *cache = Some(settings.clone());
    }
    Ok(settings)
}

pub fn save_settings_with_context(
    context: &StorageContext,
    mut settings: AppSettings,
) -> Result<AppSettings> {
    let paths = &context.paths;
    normalize_settings(&mut settings, paths);
    if let Some(parent) = paths.settings_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir_all(&settings.vault_dir)?;

    // Validate provider settings after normalization — reject invalid
    // timeout/base_url values early rather than allowing them to cause
    // confusing runtime failures later (#800).  We use provider.validate()
    // rather than the full AppSettings.validate() because the save path
    // creates vault_dir itself and api_key may legitimately be empty at
    // save time.
    let errors = settings.effective_provider().validate();
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(
            "settings validation failed: {}",
            errors.join("; ")
        ));
    }

    // Encrypt API key before persisting to disk.
    let api_key_plaintext = settings.provider.api_key.clone();
    if !api_key_plaintext.is_empty() && !crate::crypto::is_encrypted(&api_key_plaintext) {
        settings.provider.api_key = crate::crypto::encrypt_secret(&api_key_plaintext)?;
    }
    // Encrypt keys in multi-provider list.
    let providers_plaintext: Vec<String> = settings
        .providers
        .iter()
        .map(|p| p.api_key.clone())
        .collect();
    for p in &mut settings.providers {
        if !p.api_key.is_empty() && !crate::crypto::is_encrypted(&p.api_key) {
            p.api_key = crate::crypto::encrypt_secret(&p.api_key)?;
        }
    }

    let content = serde_json::to_string_pretty(&settings)?;
    atomic_write(&paths.settings_path, content.as_bytes())
        .with_context(|| format!("failed to write {}", paths.settings_path.display()))?;

    // Restore the plaintext keys in the struct we return and cache.
    settings.provider.api_key = api_key_plaintext;
    for (p, plain) in settings.providers.iter_mut().zip(providers_plaintext) {
        p.api_key = plain;
    }

    let connection = context
        .pool
        .get()
        .with_context(|| "failed to get connection from pool")?;
    super::pool::ensure_schema(&connection)?;
    // Update the cached settings after successful write.
    {
        let mut cache = context
            .cached_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *cache = Some(settings.clone());
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{default_base_url, default_model, default_timeout_ms, ProviderConfig};
    use std::path::PathBuf;

    fn make_paths(vault_override: Option<&str>) -> AppPaths {
        AppPaths {
            settings_path: PathBuf::from("/tmp/test/settings.json"),
            database_path: PathBuf::from("/tmp/test/db.sqlite"),
            chat_state_path: PathBuf::from("/tmp/test/chat.json"),
            default_vault_dir: PathBuf::from("/tmp/default_vault"),
            vault_dir_override: vault_override.map(PathBuf::from),
        }
    }

    fn make_settings(vault_dir: &str) -> AppSettings {
        AppSettings {
            vault_dir: vault_dir.to_string(),
            ..AppSettings::default()
        }
    }

    #[test]
    fn normalize_uses_vault_dir_override() {
        let paths = make_paths(Some("/custom/vault"));
        let mut s = make_settings("");
        normalize_settings(&mut s, &paths);
        assert_eq!(s.vault_dir, "/custom/vault");
    }

    #[test]
    fn normalize_fills_empty_vault_dir_with_default() {
        let paths = make_paths(None);
        let mut s = make_settings("");
        normalize_settings(&mut s, &paths);
        assert_eq!(s.vault_dir, "/tmp/default_vault");
    }

    #[test]
    fn normalize_preserves_non_empty_vault_dir() {
        let paths = make_paths(None);
        let mut s = make_settings("/my/vault");
        normalize_settings(&mut s, &paths);
        assert_eq!(s.vault_dir, "/my/vault");
    }

    #[test]
    fn normalize_fills_empty_base_url() {
        let paths = make_paths(None);
        let mut s = make_settings("/v");
        s.provider.base_url = String::new();
        normalize_settings(&mut s, &paths);
        assert_eq!(s.provider.base_url, default_base_url());
    }

    #[test]
    fn normalize_fills_empty_model() {
        let paths = make_paths(None);
        let mut s = make_settings("/v");
        s.provider.model = String::new();
        normalize_settings(&mut s, &paths);
        assert_eq!(s.provider.model, default_model());
    }

    #[test]
    fn normalize_fills_zero_timeout() {
        let paths = make_paths(None);
        let mut s = make_settings("/v");
        s.provider.request_timeout_ms = 0;
        normalize_settings(&mut s, &paths);
        assert_eq!(s.provider.request_timeout_ms, default_timeout_ms());
    }

    #[test]
    fn normalize_converts_zero_context_window_to_none() {
        let paths = make_paths(None);
        let mut s = make_settings("/v");
        s.provider.context_window_tokens = Some(0);
        normalize_settings(&mut s, &paths);
        assert_eq!(s.provider.context_window_tokens, None);
    }

    #[test]
    fn normalize_preserves_non_zero_context_window() {
        let paths = make_paths(None);
        let mut s = make_settings("/v");
        s.provider.context_window_tokens = Some(8192);
        normalize_settings(&mut s, &paths);
        assert_eq!(s.provider.context_window_tokens, Some(8192));
    }

    #[test]
    fn normalize_multi_provider_list() {
        let paths = make_paths(None);
        let mut s = make_settings("/v");
        s.providers = vec![
            ProviderConfig {
                name: "p1".into(),
                base_url: String::new(),
                model: String::new(),
                request_timeout_ms: 0,
                context_window_tokens: Some(0),
                ..ProviderConfig::default()
            },
            ProviderConfig {
                name: "p2".into(),
                base_url: "https://custom.api/v1".into(),
                model: "gpt-4".into(),
                request_timeout_ms: 30000,
                context_window_tokens: Some(4096),
                ..ProviderConfig::default()
            },
        ];
        normalize_settings(&mut s, &paths);
        assert_eq!(s.providers[0].base_url, default_base_url());
        assert_eq!(s.providers[0].model, default_model());
        assert_eq!(s.providers[0].request_timeout_ms, default_timeout_ms());
        assert_eq!(s.providers[0].context_window_tokens, None);
        assert_eq!(s.providers[1].base_url, "https://custom.api/v1");
        assert_eq!(s.providers[1].model, "gpt-4");
        assert_eq!(s.providers[1].request_timeout_ms, 30000);
        assert_eq!(s.providers[1].context_window_tokens, Some(4096));
    }

    #[test]
    fn normalize_clamps_active_provider_index() {
        let paths = make_paths(None);
        let mut s = make_settings("/v");
        s.providers = vec![ProviderConfig::default()];
        s.active_provider_index = 5;
        normalize_settings(&mut s, &paths);
        assert_eq!(s.active_provider_index, 0);
    }

    #[test]
    fn normalize_preserves_valid_active_provider_index() {
        let paths = make_paths(None);
        let mut s = make_settings("/v");
        s.providers = vec![ProviderConfig::default(), ProviderConfig::default()];
        s.active_provider_index = 1;
        normalize_settings(&mut s, &paths);
        assert_eq!(s.active_provider_index, 1);
    }

    #[test]
    fn normalize_empty_providers_no_clamp() {
        let paths = make_paths(None);
        let mut s = make_settings("/v");
        s.active_provider_index = 99;
        normalize_settings(&mut s, &paths);
        assert_eq!(s.active_provider_index, 99);
    }
}
