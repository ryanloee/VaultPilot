use serde::{Deserialize, Serialize};

use super::provider::ProviderConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub vault_dir: String,
    /// Legacy single-provider config (kept for backward compatibility).
    #[serde(default)]
    pub provider: ProviderConfig,
    /// Multi-provider list. When non-empty, overrides `provider`.
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Index into `providers` for the currently active provider.
    #[serde(default)]
    pub active_provider_index: usize,
    #[serde(default = "default_auto_check_updates")]
    pub auto_check_updates: bool,
    #[serde(default = "default_auto_wake_enabled")]
    pub auto_wake_enabled: bool,
    #[serde(default = "default_auto_wake_interval_minutes")]
    pub auto_wake_interval_minutes: u64,
    #[serde(default = "default_auto_wake_model")]
    pub auto_wake_model: String,
    #[serde(default = "default_auto_wake_start_time")]
    pub auto_wake_start_time: String,
    #[serde(default = "default_auto_wake_end_time")]
    pub auto_wake_end_time: String,
    /// Prompt sent to the AI when auto-wake fires (#861).
    #[serde(default = "default_auto_wake_prompt")]
    pub auto_wake_prompt: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            vault_dir: String::new(),
            provider: ProviderConfig::default(),
            providers: Vec::new(),
            active_provider_index: 0,
            auto_check_updates: default_auto_check_updates(),
            auto_wake_enabled: default_auto_wake_enabled(),
            auto_wake_interval_minutes: default_auto_wake_interval_minutes(),
            auto_wake_model: default_auto_wake_model(),
            auto_wake_start_time: default_auto_wake_start_time(),
            auto_wake_end_time: default_auto_wake_end_time(),
            auto_wake_prompt: default_auto_wake_prompt(),
        }
    }
}

impl AppSettings {
    /// Return the currently active provider config.
    /// If `providers` list is non-empty, returns `providers[active_provider_index]`.
    /// Otherwise falls back to the legacy single `provider` field.
    pub fn effective_provider(&self) -> &ProviderConfig {
        if !self.providers.is_empty() {
            let idx = self.active_provider_index.min(self.providers.len() - 1);
            &self.providers[idx]
        } else {
            &self.provider
        }
    }

    /// Mutable version of effective_provider for runtime overrides.
    pub fn effective_provider_mut(&mut self) -> &mut ProviderConfig {
        if !self.providers.is_empty() {
            let idx = self.active_provider_index.min(self.providers.len() - 1);
            &mut self.providers[idx]
        } else {
            &mut self.provider
        }
    }

    /// Migrate legacy single `provider` into `providers` list if empty.
    /// Called after loading settings.
    pub fn migrate_providers(&mut self) {
        if self.providers.is_empty() && !self.provider.base_url.is_empty() {
            self.provider.name = if self.provider.name.is_empty() {
                "Default".to_string()
            } else {
                self.provider.name.clone()
            };
            self.providers.push(self.provider.clone());
        }
    }

    /// Validate settings after deserialization, returning all error messages at once.
    /// An empty list means the settings are valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Validate vault_dir exists and is a directory (if non-empty).
        let vault = self.vault_dir.trim();
        if !vault.is_empty() {
            let path = std::path::Path::new(vault);
            if !path.exists() {
                errors.push(format!("vault_dir does not exist: {}", self.vault_dir));
            } else if !path.is_dir() {
                errors.push(format!("vault_dir is not a directory: {}", self.vault_dir));
            }
        }

        // Validate api_key is non-empty.
        let ep = self.effective_provider();
        if ep.api_key.trim().is_empty() {
            errors.push("provider.api_key is empty; an API key is required".to_string());
        }

        // Delegate provider-specific validation.
        errors.extend(ep.validate());

        errors
    }
}

pub fn default_auto_check_updates() -> bool {
    true
}

pub fn default_auto_wake_enabled() -> bool {
    false
}

pub fn default_auto_wake_interval_minutes() -> u64 {
    30
}

pub fn default_auto_wake_model() -> String {
    String::new()
}

pub fn default_auto_wake_start_time() -> String {
    String::new()
}

pub fn default_auto_wake_end_time() -> String {
    String::new()
}

pub fn default_auto_wake_prompt() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::provider::{default_base_url, default_model, default_timeout_ms};

    #[test]
    fn app_settings_round_trips_with_camel_case() {
        let settings = AppSettings {
            vault_dir: "D:\\Vault".to_string(),
            provider: ProviderConfig {
                name: "test".to_string(),
                api_key: "test-key".to_string(),
                base_url: "https://api.example.com".to_string(),
                model: "test-model".to_string(),
                request_timeout_ms: 30_000,
                context_window_tokens: Some(128_000),
                max_output_tokens: Some(16384),
                provider_type: None,
            },
            providers: Vec::new(),
            active_provider_index: 0,
            auto_check_updates: false,
            auto_wake_enabled: true,
            auto_wake_interval_minutes: 60,
            auto_wake_model: "claude-3-5-haiku-latest".to_string(),
            auto_wake_start_time: "05:00".to_string(),
            auto_wake_end_time: "23:00".to_string(),
            auto_wake_prompt: String::new(),
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        assert!(json.contains("\"vaultDir\""));
        assert!(json.contains("\"apiKey\""));
        assert!(json.contains("\"baseUrl\""));
        assert!(json.contains("\"requestTimeoutMs\""));
        assert!(json.contains("\"contextWindowTokens\""));
        assert!(json.contains("\"maxOutputTokens\""));
        assert!(json.contains("\"autoCheckUpdates\""));
        assert!(json.contains("\"autoWakeEnabled\""));
        assert!(json.contains("\"autoWakeIntervalMinutes\""));
        assert!(json.contains("\"autoWakeModel\""));
        assert!(json.contains("\"autoWakeStartTime\""));
        assert!(json.contains("\"autoWakeEndTime\""));

        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.vault_dir, settings.vault_dir);
        assert_eq!(parsed.provider.api_key, settings.provider.api_key);
        assert_eq!(parsed.provider.context_window_tokens, Some(128_000));
        assert_eq!(parsed.provider.max_output_tokens, Some(16384));
    }

    #[test]
    fn default_values_are_correct() {
        let settings = AppSettings::default();
        assert!(settings.vault_dir.is_empty());
        assert_eq!(settings.provider.base_url, default_base_url());
        assert_eq!(settings.provider.model, default_model());
        assert_eq!(settings.provider.request_timeout_ms, default_timeout_ms());
        assert!(settings.provider.context_window_tokens.is_none());
        assert!(settings.auto_check_updates);
        assert!(!settings.auto_wake_enabled);
        assert_eq!(settings.auto_wake_interval_minutes, 30);
        assert!(settings.auto_wake_model.is_empty());
        assert!(settings.auto_wake_start_time.is_empty());
        assert!(settings.auto_wake_end_time.is_empty());
        assert!(settings.auto_wake_prompt.is_empty());
        assert_eq!(default_model(), "deepseek-v4-flash-free");
        assert_eq!(default_timeout_ms(), 60_000);
        assert!(default_auto_check_updates());
    }

    #[test]
    fn validate_accepts_valid_settings() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "sk-test-key".to_string(),
                base_url: "https://api.anthropic.com/v1/messages".to_string(),
                request_timeout_ms: 60_000,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        assert!(settings.validate().is_empty());
    }

    #[test]
    fn validate_catches_empty_api_key() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: String::new(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors.iter().any(|e| e.contains("api_key")));
    }

    #[test]
    fn validate_catches_whitespace_only_api_key() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "   ".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors.iter().any(|e| e.contains("api_key")));
    }

    #[test]
    fn validate_catches_invalid_base_url_scheme() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                base_url: "ftp://example.com".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors.iter().any(|e| e.contains("base_url")));
    }

    #[test]
    fn validate_accepts_http_base_url() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                base_url: "http://localhost:8080/v1".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(!errors.iter().any(|e| e.contains("base_url")));
    }

    #[test]
    fn validate_catches_timeout_too_low() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                request_timeout_ms: 500,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors
            .iter()
            .any(|e| e.contains("request_timeout_ms") && e.contains("too low")));
    }

    #[test]
    fn validate_catches_timeout_too_high() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                request_timeout_ms: 999_999,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors
            .iter()
            .any(|e| e.contains("request_timeout_ms") && e.contains("too high")));
    }

    #[test]
    fn validate_catches_nonexistent_vault_dir() {
        let settings = AppSettings {
            vault_dir: "/nonexistent/path/that/does/not/exist".to_string(),
            provider: ProviderConfig {
                api_key: "key".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors
            .iter()
            .any(|e| e.contains("vault_dir") && e.contains("not exist")));
    }

    #[test]
    fn validate_returns_all_errors_at_once() {
        let settings = AppSettings {
            vault_dir: "/nonexistent/path".to_string(),
            provider: ProviderConfig {
                api_key: String::new(),
                base_url: "ftp://bad".to_string(),
                request_timeout_ms: 0,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(
            errors.len() >= 4,
            "expected at least 4 errors, got: {}",
            errors.len()
        );
        assert!(errors.iter().any(|e| e.contains("vault_dir")));
        assert!(errors.iter().any(|e| e.contains("api_key")));
        assert!(errors.iter().any(|e| e.contains("base_url")));
        assert!(errors.iter().any(|e| e.contains("request_timeout_ms")));
    }

    // ── effective_provider() ──

    #[test]
    fn effective_provider_falls_back_to_legacy_when_empty() {
        let settings = AppSettings {
            provider: ProviderConfig {
                name: "legacy".into(),
                base_url: "https://legacy.api".into(),
                ..Default::default()
            },
            providers: Vec::new(),
            ..Default::default()
        };
        assert_eq!(settings.effective_provider().name, "legacy");
    }

    #[test]
    fn effective_provider_uses_active_from_list() {
        let settings = AppSettings {
            providers: vec![
                ProviderConfig {
                    name: "first".into(),
                    ..Default::default()
                },
                ProviderConfig {
                    name: "second".into(),
                    ..Default::default()
                },
            ],
            active_provider_index: 1,
            ..Default::default()
        };
        assert_eq!(settings.effective_provider().name, "second");
    }

    #[test]
    fn effective_provider_clamps_out_of_bounds_index() {
        let settings = AppSettings {
            providers: vec![ProviderConfig {
                name: "only".into(),
                ..Default::default()
            }],
            active_provider_index: 99,
            ..Default::default()
        };
        assert_eq!(settings.effective_provider().name, "only");
    }

    #[test]
    fn effective_provider_mut_modifies_correct_entry() {
        let mut settings = AppSettings {
            providers: vec![
                ProviderConfig {
                    name: "first".into(),
                    model: "m1".into(),
                    ..Default::default()
                },
                ProviderConfig {
                    name: "second".into(),
                    model: "m2".into(),
                    ..Default::default()
                },
            ],
            active_provider_index: 0,
            ..Default::default()
        };
        settings.effective_provider_mut().model = "updated".into();
        assert_eq!(settings.providers[0].model, "updated");
        assert_eq!(settings.providers[1].model, "m2");
    }

    // ── migrate_providers() ──

    #[test]
    fn migrate_providers_moves_legacy_to_list() {
        let mut settings = AppSettings {
            provider: ProviderConfig {
                name: String::new(),
                base_url: "https://api.example.com".into(),
                model: "test-model".into(),
                ..Default::default()
            },
            providers: Vec::new(),
            ..Default::default()
        };
        settings.migrate_providers();
        assert_eq!(settings.providers.len(), 1);
        assert_eq!(settings.providers[0].name, "Default");
        assert_eq!(settings.providers[0].base_url, "https://api.example.com");
    }

    #[test]
    fn migrate_providers_preserves_existing_name() {
        let mut settings = AppSettings {
            provider: ProviderConfig {
                name: "MyProvider".into(),
                base_url: "https://api.example.com".into(),
                ..Default::default()
            },
            providers: Vec::new(),
            ..Default::default()
        };
        settings.migrate_providers();
        assert_eq!(settings.providers[0].name, "MyProvider");
    }

    #[test]
    fn migrate_providers_skips_when_list_non_empty() {
        let mut settings = AppSettings {
            provider: ProviderConfig {
                base_url: "https://legacy.api".into(),
                ..Default::default()
            },
            providers: vec![ProviderConfig {
                name: "existing".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        settings.migrate_providers();
        assert_eq!(settings.providers.len(), 1);
        assert_eq!(settings.providers[0].name, "existing");
    }

    #[test]
    fn migrate_providers_skips_when_base_url_empty() {
        let mut settings = AppSettings {
            provider: ProviderConfig {
                base_url: String::new(),
                ..Default::default()
            },
            providers: Vec::new(),
            ..Default::default()
        };
        settings.migrate_providers();
        assert!(settings.providers.is_empty());
    }
}
