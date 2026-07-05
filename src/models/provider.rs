use serde::{Deserialize, Serialize};

/// The type of AI provider, used to select correct API headers and endpoint format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    /// Anthropic Messages API (x-api-key header, /v1/messages endpoint).
    #[default]
    Anthropic,
    /// OpenAI-compatible Chat Completions API (Bearer token, /v1/chat/completions endpoint).
    OpenAi,
}

impl ProviderType {
    /// Auto-detect provider type from the base URL.
    ///
    /// URLs containing "anthropic" → Anthropic; everything else → OpenAI
    /// (since OpenAI-compatible is the most common generic format).
    pub fn from_base_url(base_url: &str) -> Self {
        let lower = base_url.to_ascii_lowercase();
        if lower.contains("anthropic") {
            Self::Anthropic
        } else {
            Self::OpenAi
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// Display name for this provider (e.g. "OpenCode Zen", "OpenRouter").
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default)]
    pub context_window_tokens: Option<usize>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// Explicit provider type override. When `None`, auto-detected from `base_url`.
    #[serde(default)]
    pub provider_type: Option<ProviderType>,
}

impl ProviderConfig {
    /// Return a clone with the API key masked for safe serialization.
    pub fn masked(&self) -> Self {
        Self {
            name: self.name.clone(),
            api_key: mask_secret(&self.api_key),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            request_timeout_ms: self.request_timeout_ms,
            context_window_tokens: self.context_window_tokens,
            max_output_tokens: self.max_output_tokens,
            provider_type: self.provider_type,
        }
    }

    /// Return the effective provider type, using the explicit override if set,
    /// otherwise auto-detecting from the base URL.
    pub fn effective_provider_type(&self) -> ProviderType {
        self.provider_type
            .unwrap_or_else(|| ProviderType::from_base_url(&self.base_url))
    }

    /// Validate provider configuration, returning a list of error messages.
    /// An empty list means the configuration is valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Validate base_url is a valid HTTP(S) URL (if non-empty).
        let url = self.base_url.trim();
        if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
            errors.push(format!(
                "provider.base_url must be an HTTP or HTTPS URL, got: {}",
                self.base_url
            ));
        }

        // Validate request_timeout_ms is in a reasonable range (1s to 10min).
        if self.request_timeout_ms < 1_000 {
            errors.push(format!(
                "provider.request_timeout_ms is too low ({}ms); minimum is 1000ms",
                self.request_timeout_ms
            ));
        } else if self.request_timeout_ms > 600_000 {
            errors.push(format!(
                "provider.request_timeout_ms is too high ({}ms); maximum is 600000ms",
                self.request_timeout_ms
            ));
        }

        errors
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            api_key: String::new(),
            base_url: default_base_url(),
            model: default_model(),
            request_timeout_ms: default_timeout_ms(),
            context_window_tokens: None,
            max_output_tokens: None,
            provider_type: None,
        }
    }
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("api_key", &mask_secret(&self.api_key))
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("provider_type", &self.provider_type)
            .finish()
    }
}

/// The Unicode ellipsis character used to mask API keys for safe display.
/// Shared between [`mask_secret`] (which produces masked strings) and
/// [`is_masked_key`] (which detects them) so that changes to one side
/// don't silently corrupt stored keys (#2539).
pub(crate) const MASK_ELLIPSIS: char = '\u{2026}';

/// Mask a secret string for safe display: show first 4 and last 4 chars.
pub(crate) fn mask_secret(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 12 {
        if chars.is_empty() {
            return String::new();
        }
        return "*".repeat(chars.len());
    }
    let prefix: String = chars[..4].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}{MASK_ELLIPSIS}{suffix}")
}

pub fn default_base_url() -> String {
    "https://opencode.ai/zen/v1".to_string()
}

pub fn default_model() -> String {
    "deepseek-v4-flash-free".to_string()
}

pub fn default_timeout_ms() -> u64 {
    60_000
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── mask_secret ──

    #[test]
    fn mask_secret_empty_returns_empty() {
        assert_eq!(mask_secret(""), "");
    }

    #[test]
    fn mask_secret_short_fully_masked() {
        assert_eq!(mask_secret("abc"), "***");
        assert_eq!(mask_secret("123456789012"), "************");
    }

    #[test]
    fn mask_secret_long_shows_prefix_suffix() {
        let key = "sk-abc...qrst";
        let masked = mask_secret(key);
        assert_eq!(masked, "sk-a…qrst");
        assert!(!masked.contains("bcdefghijklmnop"));
    }

    #[test]
    fn mask_secret_exactly_13_chars() {
        let masked = mask_secret("1234567890123");
        assert_eq!(masked, "1234…0123");
    }

    // ── mask_secret ↔ is_masked_key round-trip (#2539) ──
    //
    // These tests ensure that every masking format produced by mask_secret
    // is correctly detected by is_masked_key. If either function drifts
    // independently, the other will catch it here.

    #[test]
    fn is_masked_key_detects_short_all_star() {
        let masked = mask_secret("abc");
        assert!(crate::storage::is_masked_key(&masked));
    }

    #[test]
    fn is_masked_key_detects_12_char_all_star() {
        let masked = mask_secret("123456789012");
        assert!(crate::storage::is_masked_key(&masked));
    }

    #[test]
    fn is_masked_key_detects_long_with_ellipsis() {
        let masked = mask_secret("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789-0123456789");
        assert!(crate::storage::is_masked_key(&masked));
        // Verify the ellipsis from MASK_ELLIPSIS is present
        assert!(masked.contains(MASK_ELLIPSIS));
    }

    #[test]
    fn is_masked_key_returns_false_for_plaintext() {
        assert!(!crate::storage::is_masked_key("sk-real-key-12345-abcde"));
    }

    #[test]
    fn is_masked_key_returns_false_for_empty() {
        assert!(!crate::storage::is_masked_key(""));
    }

    #[test]
    fn is_masked_key_round_trip_all_formats() {
        // Every masking format that mask_secret can produce
        let inputs = &[
            "",
            "a",
            "ab",
            "abc",
            "123456789012",  // exactly 12 chars — all stars
            "1234567890123", // 13 chars — prefix…suffix
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789",
        ];
        for input in inputs {
            let masked = mask_secret(input);
            if input.is_empty() {
                assert!(
                    !crate::storage::is_masked_key(&masked),
                    "empty key produces empty masked output, which is not masked"
                );
            } else {
                assert!(
                    crate::storage::is_masked_key(&masked),
                    "mask_secret({input:?}) = {masked:?} should be detected as masked"
                );
            }
        }
    }

    // ── ProviderType::from_base_url ──

    #[test]
    fn provider_type_detects_anthropic() {
        assert_eq!(
            ProviderType::from_base_url("https://api.anthropic.com/v1"),
            ProviderType::Anthropic
        );
        assert_eq!(
            ProviderType::from_base_url("https://ANTHROPIC.example.com"),
            ProviderType::Anthropic
        );
    }

    #[test]
    fn provider_type_defaults_to_openai() {
        assert_eq!(
            ProviderType::from_base_url("https://api.openai.com/v1"),
            ProviderType::OpenAi
        );
        assert_eq!(
            ProviderType::from_base_url("https://openrouter.ai/api/v1"),
            ProviderType::OpenAi
        );
        assert_eq!(
            ProviderType::from_base_url("http://localhost:8080/v1"),
            ProviderType::OpenAi
        );
    }

    #[test]
    fn provider_type_from_base_url_anthropic() {
        assert_eq!(
            ProviderType::from_base_url("https://api.anthropic.com/v1"),
            ProviderType::Anthropic
        );
    }

    #[test]
    fn provider_type_from_base_url_openai() {
        assert_eq!(
            ProviderType::from_base_url("https://api.openai.com/v1"),
            ProviderType::OpenAi
        );
    }

    #[test]
    fn provider_type_from_base_url_unknown() {
        assert_eq!(
            ProviderType::from_base_url("https://custom.api.com"),
            ProviderType::OpenAi
        );
    }

    #[test]
    fn provider_type_from_empty_url_defaults_to_openai() {
        assert_eq!(ProviderType::from_base_url(""), ProviderType::OpenAi);
    }

    #[test]
    fn provider_type_from_proxy_url_with_anthropic_in_path() {
        assert_eq!(
            ProviderType::from_base_url("https://proxy.example.com/anthropic/v1"),
            ProviderType::Anthropic
        );
    }

    #[test]
    fn provider_type_case_insensitive() {
        assert_eq!(
            ProviderType::from_base_url("https://API.Anthropic.Com/v1"),
            ProviderType::Anthropic
        );
        assert_eq!(
            ProviderType::from_base_url("https://ANTHROPIC"),
            ProviderType::Anthropic
        );
    }

    // ── masked() ──

    #[test]
    fn provider_config_masked_hides_api_key() {
        let provider = ProviderConfig {
            name: "test".to_string(),
            api_key: "sk-ver...2345".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            request_timeout_ms: 60_000,
            context_window_tokens: None,
            max_output_tokens: None,
            provider_type: None,
        };
        let masked = provider.masked();
        assert!(!masked.api_key.contains("very-long-secret"));
        assert!(masked.api_key.contains("sk-v"));
        assert!(masked.api_key.contains("2345"));
        assert_eq!(masked.name, "test");
        assert_eq!(masked.base_url, "https://api.openai.com/v1");
        assert_eq!(masked.model, "gpt-4o");
    }

    #[test]
    fn provider_config_masked_with_empty_key() {
        let provider = ProviderConfig {
            api_key: String::new(),
            ..ProviderConfig::default()
        };
        let masked = provider.masked();
        assert!(masked.api_key.is_empty());
    }

    #[test]
    fn provider_config_masked_preserves_all_fields() {
        let provider = ProviderConfig {
            name: "my-provider".to_string(),
            api_key: "short".to_string(),
            base_url: "https://custom.api.com/v1".to_string(),
            model: "claude-3".to_string(),
            request_timeout_ms: 45_000,
            context_window_tokens: Some(200_000),
            max_output_tokens: Some(8192),
            provider_type: Some(ProviderType::Anthropic),
        };
        let masked = provider.masked();
        assert_eq!(masked.name, "my-provider");
        assert_eq!(masked.base_url, "https://custom.api.com/v1");
        assert_eq!(masked.model, "claude-3");
        assert_eq!(masked.request_timeout_ms, 45_000);
        assert_eq!(masked.context_window_tokens, Some(200_000));
        assert_eq!(masked.max_output_tokens, Some(8192));
        assert_eq!(masked.provider_type, Some(ProviderType::Anthropic));
        assert_eq!(masked.api_key, "*****");
    }

    // ── effective_provider_type() ──

    #[test]
    fn effective_provider_type_explicit_override() {
        let provider = ProviderConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            provider_type: Some(ProviderType::Anthropic),
            ..ProviderConfig::default()
        };
        assert_eq!(provider.effective_provider_type(), ProviderType::Anthropic);
    }

    #[test]
    fn effective_provider_type_auto_from_url() {
        let provider = ProviderConfig {
            base_url: "https://api.anthropic.com/v1".to_string(),
            provider_type: None,
            ..ProviderConfig::default()
        };
        assert_eq!(provider.effective_provider_type(), ProviderType::Anthropic);
    }
}
