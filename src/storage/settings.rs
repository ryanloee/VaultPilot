use std::fs;

use anyhow::{Context, Result};
use tracing::warn;

use crate::models::AppSettings;

use super::atomic_write;
use super::pool::{AppPaths, StorageContext};

/// Heuristic: could `s` be a value produced by [`mask_secret`]?
///
/// This is **format-aware** so it does not false-positive on genuine plaintext
/// keys that merely *contain* the masking characters (#2987):
///
/// * **Short key** (≤ 12 chars): entirely `*` chars, length 1..=12.
///   `mask_secret` only emits all-`*` for inputs of length ≤ 12, so a longer
///   `*`-only string (or one containing `…`) is treated as plaintext.
/// * **Long key** (> 12 chars): exactly `<4 chars>…<4 chars>` where the middle
///   char is [`MASK_ELLIPSIS`].
///
/// ⚠️ This is a *format* heuristic only. Because [`mask_secret`] is lossy, a
/// genuine 9-char plaintext value `abcd…wxyz` shares the exact long-key shape
/// and will be flagged here (#2997/#3001). When the real (unmasked) key is
/// available, prefer [`is_masked_form_of`] which verifies the prefix/suffix
/// actually match that key before treating the value as a display mask.
///
/// Uses [`MASK_ELLIPSIS`] from the provider module so that changes to the
/// masking format don't silently corrupt stored keys (#2539).
pub(crate) fn is_masked_key(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return false;
    }
    // Long-key mask: exactly "<4><ELLIPSIS><4>" == 9 chars with the middle
    // char being MASK_ELLIPSIS. This is emitted by mask_secret for inputs
    // longer than 12 chars. Checked first because a 9-char string is also
    // "<= 12" by length, which would otherwise be misclassified as a short
    // mask below.
    if chars.len() == 9 && chars[4] == crate::models::provider::MASK_ELLIPSIS {
        return true;
    }
    // Short-key mask: all '*' chars, length 1..=12. mask_secret only emits
    // all-'*' for inputs of length ≤ 12, so a longer *-only string (or any
    // string containing '…' that isn't the exact long form above) is plaintext.
    !chars.is_empty() && chars.len() <= 12 && chars.iter().all(|c| *c == '*')
}

/// Returns `true` only if `candidate` is the actual masked form of `real_key` —
/// i.e. it was produced by `mask_secret(real_key)` and therefore represents
/// "the user left this key unchanged" rather than a genuine new value.
///
/// Unlike [`is_masked_key`] (a pure format heuristic), this verifies the
/// candidate's prefix/suffix against `real_key`, so a genuine 9-char plaintext
/// value such as `abcd…wxyz` is **not** mistaken for a display mask and is
/// kept as a real key change (#2997/#3001). Used when the existing stored key
/// is available so the silent key-loss path is fully disambiguated.
pub(crate) fn is_masked_form_of(candidate: &str, real_key: &str) -> bool {
    if !is_masked_key(candidate) {
        return false;
    }
    let chars: Vec<char> = candidate.chars().collect();
    let real: Vec<char> = real_key.chars().collect();
    if chars.len() == 9 && chars[4] == crate::models::provider::MASK_ELLIPSIS {
        // Long-key form: must match real_key's first/last 4 chars.
        if real.len() <= 12 {
            // A short real key would have been masked as all-*, never the
            // long form, so this candidate cannot be its mask.
            return false;
        }
        let prefix: String = chars[..4].iter().collect();
        let suffix: String = chars[5..].iter().collect();
        let rprefix: String = real[..4].iter().collect();
        let rsuffix: String = real[real.len() - 4..].iter().collect();
        prefix == rprefix && suffix == rsuffix
    } else {
        // Short-key form: all '*' of length L. mask_secret emits all-* for any
        // key of length ≤ 12, so match iff real_key has the same length.
        chars.len() == real.len()
    }
}

/// Load settings directly from the disk file, bypassing the in-memory cache.
/// Returns `Err` if the file doesn't exist or can't be parsed, which is
/// perfectly normal on first run — callers should use `Result<Option<..>>`
/// semantics via `if let Ok`.
fn load_settings_raw(context: &StorageContext) -> Result<AppSettings> {
    let paths = &context.paths;
    if !paths.settings_path.exists() {
        return Err(anyhow::anyhow!("settings file not found"));
    }
    let raw = fs::read_to_string(&paths.settings_path)
        .with_context(|| format!("failed to read {}", paths.settings_path.display()))?;
    let normalized = raw.trim_start_matches('\u{feff}');
    let mut parsed: AppSettings = serde_json::from_str(normalized)
        .with_context(|| format!("failed to parse {}", paths.settings_path.display()))?;

    // Decrypt API keys so comparison with incoming (plaintext) values works.
    // Propagate decryption errors so callers can distinguish between "no key"
    // and "key present but undecryptable" (#2406).
    if !parsed.provider.api_key.is_empty() {
        parsed.provider.api_key = crate::crypto::decrypt_secret(&parsed.provider.api_key)
            .context("Failed to decrypt stored API key — the machine key may have changed. Please re-enter your API key in Settings")?;
    }
    // If the file has an empty key (or decryption produced empty), try the OS
    // keychain as an alternative store (#3159).
    if parsed.provider.api_key.is_empty() {
        if let Ok(Some(kc_key)) =
            crate::keychain::KEYCHAIN.get(&crate::keychain::account_key(&parsed.provider.name))
        {
            parsed.provider.api_key = kc_key;
        }
    }
    for p in &mut parsed.providers {
        if !p.api_key.is_empty() {
            p.api_key = crate::crypto::decrypt_secret(&p.api_key)
                .context("Failed to decrypt provider API key")?;
        }
        if p.api_key.is_empty() {
            if let Ok(Some(kc_key)) =
                crate::keychain::KEYCHAIN.get(&crate::keychain::account_key(&p.name))
            {
                p.api_key = kc_key;
            }
        }
    }
    parsed.migrate_providers();
    normalize_settings(&mut parsed, paths);
    Ok(parsed)
}

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
        // If the file has an empty key, try the OS keychain (#3159).
        if parsed.provider.api_key.is_empty() {
            if let Ok(Some(kc_key)) =
                crate::keychain::KEYCHAIN.get(&crate::keychain::account_key(&parsed.provider.name))
            {
                parsed.provider.api_key = kc_key;
            }
        }
        // Decrypt keys in multi-provider list.
        for p in &mut parsed.providers {
            if !p.api_key.is_empty() {
                p.api_key = crate::crypto::decrypt_secret(&p.api_key)
                    .context("Failed to decrypt provider API key")?;
            }
            if p.api_key.is_empty() {
                if let Ok(Some(kc_key)) =
                    crate::keychain::KEYCHAIN.get(&crate::keychain::account_key(&p.name))
                {
                    p.api_key = kc_key;
                }
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
    // Double-check: another thread may have populated the cache while we
    // were reading the settings file from disk (TOCTOU race).  If so, prefer
    // the value already in the cache — it is at least as fresh as ours.
    {
        let mut cache = context
            .cached_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cache.is_some() {
            // Another writer beat us — return its value.
            return Ok(cache.as_ref().unwrap().clone());
        }
        *cache = Some(settings.clone());
    }
    Ok(settings)
}

pub fn save_settings_with_context(
    context: &StorageContext,
    mut settings: AppSettings,
) -> Result<AppSettings> {
    let paths = &context.paths;
    settings.migrate_providers();
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

    // Preserve existing API keys if the incoming values are masked.
    // The UI sends masked keys (e.g. "sk-a…qrst") which would overwrite
    // the real encrypted key if saved as-is. Load the current settings
    // and keep any key that hasn't been explicitly changed by the user.
    // If decryption fails (e.g. machine key changed), fall back to reading
    // the encrypted keys from raw JSON so we don't overwrite them with
    // masked values (#2406).
    let existing_settings = load_settings_raw(context).or_else(|_| -> Result<AppSettings> {
        // Decryption failed — read raw JSON to preserve encrypted keys.
        let raw = fs::read_to_string(&paths.settings_path)
            .with_context(|| format!("failed to read {}", paths.settings_path.display()))?;
        let normalized = raw.trim_start_matches('\u{feff}');
        let mut parsed: AppSettings = serde_json::from_str(normalized)
            .with_context(|| format!("failed to parse {}", paths.settings_path.display()))?;
        parsed.migrate_providers();
        Ok(parsed)
    });
    match &existing_settings {
        Ok(existing_settings) => {
            if existing_settings.provider.api_key != settings.provider.api_key
                && is_masked_form_of(
                    &settings.provider.api_key,
                    &existing_settings.provider.api_key,
                )
                && !settings.provider.api_key.is_empty()
            {
                settings.provider.api_key = existing_settings.provider.api_key.clone();
            }
            for p in settings.providers.iter_mut() {
                // Match by (name, base_url) first — handles reordering (#2514).
                let existing = existing_settings
                    .providers
                    .iter()
                    .find(|ep| ep.name == p.name && ep.base_url == p.base_url)
                    .or_else(|| {
                        // Fallback: match by name alone (user changed base_url, etc.).
                        existing_settings
                            .providers
                            .iter()
                            .find(|ep| ep.name == p.name)
                    })
                    .or_else(|| {
                        // Final fallback: match by base_url alone (#2587).
                        // If the user renamed the provider (name changed but base_url
                        // unchanged), neither (name, base_url) nor name alone match.
                        // Matching by base_url identifies this as a rename and
                        // preserves the existing API key instead of silently losing it.
                        existing_settings
                            .providers
                            .iter()
                            .find(|ep| ep.base_url == p.base_url)
                    });
                if let Some(existing) = existing {
                    if p.api_key != existing.api_key
                        && is_masked_form_of(&p.api_key, &existing.api_key)
                        && !p.api_key.is_empty()
                    {
                        p.api_key = existing.api_key.clone();
                    }
                }
            }
        }
        Err(e) => {
            // #2557: we could not load the existing settings to resolve masked
            // API keys (the file is corrupt/unreadable and the raw-JSON
            // fallback also failed). If the incoming request contains any
            // masked key (e.g. "sk-a…qrst"), persisting it as-is would
            // overwrite the real (encrypted) key on disk with a display-only
            // mask, permanently losing the user's API key with no warning.
            // Refuse the save and surface the underlying load error so the
            // user can fix the settings file or re-enter the key.
            let main_masked =
                !settings.provider.api_key.is_empty() && is_masked_key(&settings.provider.api_key);
            let any_provider_masked = settings
                .providers
                .iter()
                .any(|p| !p.api_key.is_empty() && is_masked_key(&p.api_key));
            if main_masked || any_provider_masked {
                return Err(anyhow::anyhow!(
                    "Could not load existing settings to resolve masked API key ({}); \
                     refusing to save to avoid overwriting the real key with a display \
                     mask. Please fix the settings file or re-enter the API key.",
                    crate::sanitize_error(&e.to_string())
                ));
            }
            // No masked keys present — incoming values are real keys (or
            // empty), so it is safe to persist them without the preservation step.
        }
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

    // Delete keychain entries for providers that were removed from the
    // settings entirely (#3193). The loop below only syncs keychain state
    // for providers still present in `settings.providers`, so a deleted
    // provider's plaintext credential would otherwise linger in the OS
    // keychain indefinitely — a privacy leak and a source of silent
    // resurrection if a same-named provider is later recreated.
    if let Ok(existing) = &existing_settings {
        for old_p in &existing.providers {
            if !settings.providers.iter().any(|p| p.name == old_p.name) {
                let _ =
                    crate::keychain::KEYCHAIN.delete(&crate::keychain::account_key(&old_p.name));
            }
        }
    }

    // Optionally store plaintext keys in the OS keychain (#3159) as a
    // supplemental store.  The file still carries the encrypted fallback
    // so existing behaviour is preserved on all platforms.
    //
    // When a key is *empty* (user cleared it), explicitly delete the
    // corresponding keychain entry — otherwise the stale secret survives
    // in the OS keychain and is silently resurrected on the next load
    // (#3170).
    if api_key_plaintext.is_empty() {
        let _ = crate::keychain::KEYCHAIN
            .delete(&crate::keychain::account_key(&settings.provider.name));
    } else {
        let _ = crate::keychain::KEYCHAIN.set(
            &crate::keychain::account_key(&settings.provider.name),
            &api_key_plaintext,
        );
    }
    for (p, plain) in settings.providers.iter().zip(&providers_plaintext) {
        if plain.is_empty() {
            let _ = crate::keychain::KEYCHAIN.delete(&crate::keychain::account_key(&p.name));
        } else {
            let _ = crate::keychain::KEYCHAIN.set(&crate::keychain::account_key(&p.name), plain);
        }
    }

    // Security contract (#2826): never persist a plaintext API key to disk.
    // The loops above encrypt every non-empty key, so by this point each key
    // must be either empty or already carry the `ENC:v1:` prefix. If a
    // non-empty key escaped encryption (e.g. a future refactor that writes
    // settings before the encrypt step runs), refuse the save instead of
    // leaking the secret in plaintext inside settings.json.
    if !settings.provider.api_key.is_empty()
        && !crate::crypto::is_encrypted(&settings.provider.api_key)
    {
        return Err(anyhow::anyhow!(
            "refusing to persist plaintext provider API key (#2826); encrypt before saving"
        ));
    }
    for p in &settings.providers {
        if !p.api_key.is_empty() && !crate::crypto::is_encrypted(&p.api_key) {
            return Err(anyhow::anyhow!(
                "refusing to persist plaintext API key for provider '{}' (#2826); encrypt before saving",
                p.name
            ));
        }
    }

    let content = serde_json::to_string_pretty(&settings)?;
    atomic_write(&paths.settings_path, content.as_bytes())
        .with_context(|| format!("failed to write {}", paths.settings_path.display()))?;

    // Restore the plaintext keys in the struct we return and cache.
    // If the key is still encrypted (undecryptable fallback from
    // load_settings_raw failing), keep the encrypted value rather than
    // clearing to empty — clearing poisons the cache with empty keys,
    // causing subsequent loads to return falsified no-key settings
    // even though the disk has valid (but undecryptable) keys (#2513).
    // The encrypted value is already machine-bound and safe in memory.
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

    // ── #2557: masked key must not be persisted when settings file is corrupt ──

    fn unique_temp(label: &str) -> std::path::PathBuf {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-settings-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        temp
    }

    #[test]
    fn save_masked_key_rejected_when_settings_corrupt() {
        // When the settings file is corrupt (unparseable), the masked-key
        // preservation logic cannot resolve a masked incoming key against the
        // existing real key. Saving must be refused rather than persisting the
        // mask string and permanently losing the real key (#2557).
        let temp = unique_temp("2557");
        let ctx = StorageContext::for_test(&temp);
        // Both load_settings_raw and its raw-JSON fallback fail to parse this.
        std::fs::write(&ctx.paths.settings_path, "{ this is not valid json }")
            .expect("write corrupt file");

        let settings = AppSettings {
            vault_dir: temp.join("vault").to_string_lossy().to_string(),
            provider: ProviderConfig {
                // A VALID mask in the exact "<4><ELLIPSIS><4>" format produced
                // by mask_secret (9 chars). This is genuinely masked, so saving
                // over a corrupt file must be refused. A plaintext key that
                // merely contains '…' (e.g. "sk-abcd…wxyz", 13 chars) is NOT a
                // valid mask and is treated as real per #2987.
                api_key: "sk-a…wxyz".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };

        let result = save_settings_with_context(&ctx, settings);
        assert!(
            result.is_err(),
            "saving a masked key over a corrupt settings file must be refused"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("masked"),
            "error should mention masked key resolution, got: {msg}"
        );
    }

    #[test]
    fn save_real_key_succeeds_when_settings_corrupt() {
        // When the file is corrupt but the incoming key is real (not masked),
        // the save should proceed and repair the corrupt file (#2557).
        let temp = unique_temp("2557ok");
        let ctx = StorageContext::for_test(&temp);
        std::fs::write(&ctx.paths.settings_path, "{ corrupt").expect("write corrupt file");

        let settings = AppSettings {
            vault_dir: temp.join("vault").to_string_lossy().to_string(),
            provider: ProviderConfig {
                api_key: "sk-real-key-for-test".to_string(), // real, unmasked
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };

        let result = save_settings_with_context(&ctx, settings);
        assert!(
            result.is_ok(),
            "saving a real key should repair the corrupt file: {:?}",
            result.err()
        );
    }

    // ── #2997 / #3001: is_masked_form_of must not false-positive on a genuine
    // 9-char plaintext value that shares the long-mask shape `XXXX…XXXX`. ──

    #[test]
    fn is_masked_key_still_heuristic_true_for_plaintext_shape() {
        // is_masked_key is a pure format heuristic and WILL flag the shape;
        // that is why callers must use is_masked_form_of when the real key is known.
        assert!(is_masked_key("abcd…wxyz"));
    }

    #[test]
    fn is_masked_form_of_rejects_plaintext_ellipsis_key() {
        // A genuine 9-char plaintext key with an ellipsis must NOT be treated
        // as the mask of a different (real) key.
        let real_key = "sk-abc...6789"; // 13+ char stored key
        assert!(!is_masked_form_of("abcd…wxyz", real_key));
    }

    #[test]
    fn is_masked_form_of_accepts_true_mask() {
        // The actual mask of the stored key is recognised as "unchanged".
        let real_key = "sk-abc...6789";
        let masked = crate::models::provider::mask_secret(real_key);
        assert!(is_masked_form_of(&masked, real_key));
        // And a different real key is NOT mistaken for the mask.
        assert!(!is_masked_form_of(&masked, "sk-xyz...0000"));
    }

    #[test]
    fn plaintext_ellipsis_key_not_silently_discarded() {
        // #2997/#3001 end-to-end: saving a 9-char plaintext key containing an
        // ellipsis must persist that key, not replace it with the stale one.
        let temp = unique_temp("2997");
        let ctx = StorageContext::for_test(&temp);
        let vault = temp.join("vault").to_string_lossy().to_string();

        // First save a real stored key.
        let stored = AppSettings {
            vault_dir: vault.clone(),
            provider: ProviderConfig {
                api_key: "sk-abc...6789".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        save_settings_with_context(&ctx, stored).expect("first save ok");

        // Now the user changes the key to a genuine 9-char plaintext value that
        // happens to contain an ellipsis (the false-positive shape).
        let new_key = "abcd…wxyz";
        let incoming = AppSettings {
            vault_dir: vault,
            provider: ProviderConfig {
                api_key: new_key.to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        save_settings_with_context(&ctx, incoming).expect("second save ok");

        let reloaded = load_settings_raw(&ctx).expect("reload");
        assert_eq!(
            reloaded.provider.api_key, new_key,
            "the plaintext ellipsis key must be persisted, not replaced by the stale key"
        );
    }
}
