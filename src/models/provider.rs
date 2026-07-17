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
    /// Local Ollama instance — uses OpenAI-compatible API at /v1/chat/completions
    /// but requires no API key and allows localhost endpoints (#2798).
    Ollama,
}

impl ProviderType {
    /// Auto-detect provider type from the base URL.
    ///
    /// URLs containing "anthropic" → Anthropic; "ollama" or ":11434" → Ollama;
    /// everything else → OpenAI (since OpenAI-compatible is the most common
    /// generic format).
    pub fn from_base_url(base_url: &str) -> Self {
        let lower = base_url.to_ascii_lowercase();
        if lower.contains("anthropic") {
            Self::Anthropic
        } else if lower.contains("ollama") || lower.contains(":11434") {
            Self::Ollama
        } else {
            Self::OpenAi
        }
    }

    /// Whether this provider type requires a non-empty API key.
    /// Ollama runs locally and does not need authentication (#2798).
    pub fn requires_api_key(&self) -> bool {
        !matches!(self, Self::Ollama)
    }

    /// Whether this provider type is allowed to use localhost / private-IP
    /// endpoints without requiring `VAULTPILOT_ALLOW_LOCAL_ENDPOINT`.
    /// Ollama runs on `http://localhost:11434` by default (#2798).
    pub fn allows_local_endpoint(&self) -> bool {
        matches!(self, Self::Ollama)
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

/// Prefix used to make masked long secrets unambiguously identifiable.
///
/// `is_masked_key` requires this prefix so that a genuine plaintext value
/// of the same `<4>…<4>` shape (e.g. a 9-char key containing an ellipsis)
/// is never misclassified as a mask (#2997, #3001, #2987).
pub(crate) const MASK_PREFIX: &str = "mask:";

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
    format!("{MASK_PREFIX}{prefix}{MASK_ELLIPSIS}{suffix}")
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

// ---------------------------------------------------------------------------
// Known model definitions (#1862)
// ---------------------------------------------------------------------------

/// Capabilities that a model may support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// Whether the model supports image/video input.
    pub vision: bool,
    /// Whether the model supports chain-of-thought / extended reasoning.
    pub reasoning: bool,
    /// Whether the model supports function/tool calling.
    pub function_calling: bool,
    /// Whether the model supports streaming responses.
    pub streaming: bool,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            vision: false,
            reasoning: false,
            function_calling: true,
            streaming: true,
        }
    }
}

/// Information about a well-known model, used for preset dropdowns and
/// capability-aware routing in the UI layer (#1862).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownModel {
    /// Model identifier string (e.g. "gpt-5.5", "claude-opus-4-7").
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// The provider type this model is typically associated with.
    pub provider: ProviderType,
    /// Maximum context window size in tokens.
    pub context_window: usize,
    /// Default maximum output tokens.
    pub max_output_tokens: u32,
    /// Model capabilities metadata.
    pub capabilities: ModelCapabilities,
}

/// Return a list of well-known model presets, covering the latest models
/// from all major providers as of July 2026 (#1862).
pub fn known_models() -> Vec<KnownModel> {
    vec![
        // ── OpenAI / OpenAI-compatible ──
        KnownModel {
            id: "gpt-5.5".into(),
            name: "GPT-5.5".into(),
            provider: ProviderType::OpenAi,
            context_window: 256_000,
            max_output_tokens: 16384,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: true,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "gpt-5.4-mini".into(),
            name: "GPT-5.4 Mini".into(),
            provider: ProviderType::OpenAi,
            context_window: 128_000,
            max_output_tokens: 16384,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: true,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "deepseek-v4-flash-free".into(),
            name: "DeepSeek V4 Flash (Free)".into(),
            provider: ProviderType::OpenAi,
            context_window: 128_000,
            max_output_tokens: 8192,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: false,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "deepseek-v4".into(),
            name: "DeepSeek V4".into(),
            provider: ProviderType::OpenAi,
            context_window: 128_000,
            max_output_tokens: 8192,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: true,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "grok-4.3".into(),
            name: "Grok 4.3".into(),
            provider: ProviderType::OpenAi,
            context_window: 131_072,
            max_output_tokens: 8192,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: true,
                function_calling: true,
                streaming: true,
            },
        },
        // ── Anthropic ──
        KnownModel {
            id: "claude-opus-4-7".into(),
            name: "Claude Opus 4.7".into(),
            provider: ProviderType::Anthropic,
            context_window: 200_000,
            max_output_tokens: 8192,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: true,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "claude-sonnet-4-6".into(),
            name: "Claude Sonnet 4.6".into(),
            provider: ProviderType::Anthropic,
            context_window: 200_000,
            max_output_tokens: 8192,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: true,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "claude-haiku-4-5".into(),
            name: "Claude Haiku 4.5".into(),
            provider: ProviderType::Anthropic,
            context_window: 200_000,
            max_output_tokens: 8192,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: false,
                function_calling: true,
                streaming: true,
            },
        },
        // ── Google Gemini ──
        KnownModel {
            id: "gemini-3.5-flash".into(),
            name: "Gemini 3.5 Flash".into(),
            provider: ProviderType::OpenAi,
            context_window: 1_048_576,
            max_output_tokens: 8192,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: true,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "gemini-3.5-pro".into(),
            name: "Gemini 3.5 Pro".into(),
            provider: ProviderType::OpenAi,
            context_window: 2_097_152,
            max_output_tokens: 16384,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: true,
                function_calling: true,
                streaming: true,
            },
        },
        // ── Local / Ollama models (#1706, #2798) ──
        //
        // These run locally via Ollama (http://localhost:11434) using
        // OpenAI-compatible Chat Completions API. Context windows and
        // capabilities vary by quantisation and model variant.
        KnownModel {
            id: "llama3.3".into(),
            name: "Llama 3.3 70B (Ollama)".into(),
            provider: ProviderType::Ollama,
            context_window: 128_000,
            max_output_tokens: 4096,
            capabilities: ModelCapabilities {
                vision: false,
                reasoning: false,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "llama3.2-vision".into(),
            name: "Llama 3.2 11B Vision (Ollama)".into(),
            provider: ProviderType::Ollama,
            context_window: 128_000,
            max_output_tokens: 4096,
            capabilities: ModelCapabilities {
                vision: true,
                reasoning: false,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "gemma4".into(),
            name: "Gemma 4 27B (Ollama)".into(),
            provider: ProviderType::Ollama,
            context_window: 32_768,
            max_output_tokens: 4096,
            capabilities: ModelCapabilities {
                vision: false,
                reasoning: false,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "mistral".into(),
            name: "Mistral 7B (Ollama)".into(),
            provider: ProviderType::Ollama,
            context_window: 32_768,
            max_output_tokens: 4096,
            capabilities: ModelCapabilities {
                vision: false,
                reasoning: false,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "qwen2.5".into(),
            name: "Qwen 2.5 32B (Ollama)".into(),
            provider: ProviderType::Ollama,
            context_window: 32_768,
            max_output_tokens: 4096,
            capabilities: ModelCapabilities {
                vision: false,
                reasoning: false,
                function_calling: true,
                streaming: true,
            },
        },
        KnownModel {
            id: "phi-4".into(),
            name: "Phi-4 14B (Ollama)".into(),
            provider: ProviderType::Ollama,
            context_window: 16_384,
            max_output_tokens: 4096,
            capabilities: ModelCapabilities {
                vision: false,
                reasoning: false,
                function_calling: true,
                streaming: true,
            },
        },
    ]
}

/// Look up a known model by its id string.
/// Returns `None` if the model is not in the built-in list.
pub fn lookup_model(id: &str) -> Option<KnownModel> {
    known_models().into_iter().find(|m| m.id == id)
}

/// Return the subset of [`known_models`] that satisfy *every* capability flag
/// set to `true` in `require`.
///
/// Used by the model-selector UI to narrow the preset dropdown — e.g. only
/// show vision-capable models when the user is about to attach an image (#2970).
/// Capability flags left as `false`/`false` in `require` are treated as
/// "don't care" and never filter a model out.
///
/// Deterministic and allocation-light so it can be called on every render.
pub fn models_with_capabilities(require: ModelCapabilities) -> Vec<KnownModel> {
    known_models()
        .into_iter()
        .filter(|m| {
            (!require.vision || m.capabilities.vision)
                && (!require.reasoning || m.capabilities.reasoning)
                && (!require.function_calling || m.capabilities.function_calling)
                && (!require.streaming || m.capabilities.streaming)
        })
        .collect()
}

/// Return the subset of [`known_models`] for a single provider type (#2970).
///
/// Lets the selector group presets by provider (Anthropic / OpenAI / Ollama)
/// so the UI can present a per-provider section rather than one flat list.
pub fn models_for_provider(provider: ProviderType) -> Vec<KnownModel> {
    known_models()
        .into_iter()
        .filter(|m| m.provider == provider)
        .collect()
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
        assert_eq!(masked, "mask:sk-a…qrst");
        assert!(!masked.contains("bcdefghijklmnop"));
    }

    #[test]
    fn mask_secret_exactly_13_chars() {
        let masked = mask_secret("1234567890123");
        assert_eq!(masked, "mask:1234…0123");
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

    // ── #2987 regression: false-positives on plaintext keys ──

    #[test]
    fn is_masked_key_plaintext_with_ellipsis_is_not_masked() {
        // Genuine plaintext key that happens to contain U+2026.
        assert!(!crate::storage::is_masked_key("sk-…real…key"));
    }

    #[test]
    fn is_masked_key_long_all_star_is_not_masked() {
        // 24 stars: longer than mask_secret's short-key limit (12).
        assert!(!crate::storage::is_masked_key("************************"));
    }

    #[test]
    fn is_masked_key_eight_star_is_masked() {
        // 8 stars: valid short-key mask produced by mask_secret.
        assert!(crate::storage::is_masked_key("********"));
    }

    #[test]
    fn is_masked_key_long_ellipsis_wrong_length_is_not_masked() {
        // Contains ellipsis but not the exact "<4>…<4>" shape.
        assert!(!crate::storage::is_masked_key("sk-abc…qrstuvwxyz"));
    }

    #[test]
    fn is_masked_key_long_ellipsis_no_middle_is_not_masked() {
        // 9 chars but the 5th char is not the ellipsis.
        assert!(!crate::storage::is_masked_key("sk-abXqrst"));
    }

    // ── #2997 / #3001 regression: 9-char plaintext containing an ellipsis ──
    //
    // A genuine 9-char plaintext value of the exact "<4>…<4>" shape (e.g.
    // "abcd…wxyz") previously collided with a masked 13-char key. The MASK_PREFIX
    // sentinel in mask_secret's long form now makes these unambiguously distinct.

    #[test]
    fn is_masked_key_9char_plaintext_with_ellipsis_is_not_masked() {
        // Genuine plaintext: 9 chars, ellipsis at index 4, no "mask:" prefix.
        let mut p = String::new();
        p.push_str("abcd");
        p.push(MASK_ELLIPSIS);
        p.push_str("wxyz");
        assert_eq!(p.chars().count(), 9);
        assert!(!crate::storage::is_masked_key(&p));
    }

    #[test]
    fn is_masked_key_real_mask_still_detected() {
        // A value produced by mask_secret must still be detected as masked.
        let masked = mask_secret("sk-ant-api-key-xyz-6789");
        assert!(masked.starts_with(MASK_PREFIX));
        assert!(crate::storage::is_masked_key(&masked));
    }

    #[test]
    fn is_masked_key_9char_with_prefix_is_masked() {
        // Only the sentinel-prefixed form is treated as a mask.
        assert!(crate::storage::is_masked_key("mask:abcd…wxyz"));
        assert!(!crate::storage::is_masked_key("abcd…wxyz"));
    }

    #[test]
    fn is_masked_key_round_trip_all_formats_extended() {
        // Short keys 1..=12 chars must all be detected as masked (all stars).
        for n in 1..=12 {
            let input: String = "x".repeat(n);
            let masked = mask_secret(&input);
            assert!(
                crate::storage::is_masked_key(&masked),
                "short key len {n} -> {masked:?} should be masked"
            );
        }
        // A 13-char (long) key produces the "<4>…<4>" mask.
        let masked = mask_secret("1234567890123");
        assert!(crate::storage::is_masked_key(&masked));
        // A 24-char all-star string must NOT be detected as masked.
        assert!(!crate::storage::is_masked_key(&"*".repeat(24)));
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

    // ── known_models (#1862) ──

    #[test]
    fn known_models_contains_expected_count() {
        let models = known_models();
        // At least 10 well-known models: GPT-5.5, GPT-5.4 Mini, DeepSeek V4 Flash,
        // DeepSeek V4, Grok 4.3, Claude Opus 4.7, Claude Sonnet 4.6, Claude Haiku 4.5,
        // Gemini 3.5 Flash, Gemini 3.5 Pro
        assert!(
            models.len() >= 10,
            "expected >=10 models, got {}",
            models.len()
        );
    }

    #[test]
    fn known_models_has_unique_ids() {
        let models = known_models();
        let mut ids = std::collections::HashSet::new();
        for m in &models {
            assert!(ids.insert(&m.id), "duplicate model id: {}", m.id);
        }
    }

    #[test]
    fn known_models_all_have_valid_context_windows() {
        for m in known_models() {
            assert!(
                m.context_window >= 4096,
                "model {} has context_window {} < 4096",
                m.id,
                m.context_window
            );
        }
    }

    #[test]
    fn known_models_all_have_positive_max_output() {
        for m in known_models() {
            assert!(
                m.max_output_tokens > 0,
                "model {} has max_output_tokens == 0",
                m.id
            );
        }
    }

    #[test]
    fn known_models_include_gpt55() {
        let m = lookup_model("gpt-5.5").expect("gpt-5.5 not found");
        assert_eq!(m.name, "GPT-5.5");
        assert!(m.capabilities.vision);
        assert!(m.capabilities.reasoning);
        assert!(m.capabilities.function_calling);
        assert!(m.capabilities.streaming);
        assert_eq!(m.provider, ProviderType::OpenAi);
    }

    #[test]
    fn known_models_include_claude_opus() {
        let m = lookup_model("claude-opus-4-7").expect("claude-opus-4-7 not found");
        assert_eq!(m.name, "Claude Opus 4.7");
        assert_eq!(m.provider, ProviderType::Anthropic);
        assert_eq!(m.context_window, 200_000);
    }

    #[test]
    fn known_models_include_gemini_flash() {
        let m = lookup_model("gemini-3.5-flash").expect("gemini-3.5-flash not found");
        assert_eq!(m.name, "Gemini 3.5 Flash");
        assert_eq!(m.context_window, 1_048_576);
    }

    #[test]
    fn known_models_include_deepseek_flash() {
        let m = lookup_model("deepseek-v4-flash-free").expect("deepseek-v4-flash-free not found");
        assert_eq!(m.name, "DeepSeek V4 Flash (Free)");
        assert!(
            !m.capabilities.reasoning,
            "flash-free should not have reasoning"
        );
    }

    #[test]
    fn lookup_model_unknown_returns_none() {
        assert!(lookup_model("nonexistent-model-v99").is_none());
        assert!(lookup_model("").is_none());
    }

    #[test]
    fn known_models_haiku_no_reasoning() {
        let m = lookup_model("claude-haiku-4-5").expect("claude-haiku-4-5 not found");
        assert!(!m.capabilities.reasoning, "Haiku should not have reasoning");
    }

    // ── Local/Ollama model presets (#1706) ──

    #[test]
    fn known_models_include_llama33() {
        let m = lookup_model("llama3.3").expect("llama3.3 not found");
        assert_eq!(m.name, "Llama 3.3 70B (Ollama)");
        assert_eq!(m.provider, ProviderType::Ollama);
        assert_eq!(m.context_window, 128_000);
        assert!(!m.capabilities.vision);
    }

    #[test]
    fn known_models_include_gemma4() {
        let m = lookup_model("gemma4").expect("gemma4 not found");
        assert_eq!(m.name, "Gemma 4 27B (Ollama)");
        assert_eq!(m.provider, ProviderType::Ollama);
        assert_eq!(m.context_window, 32_768);
    }

    #[test]
    fn known_models_include_qwen25() {
        let m = lookup_model("qwen2.5").expect("qwen2.5 not found");
        assert_eq!(m.name, "Qwen 2.5 32B (Ollama)");
        assert_eq!(m.provider, ProviderType::Ollama);
    }

    #[test]
    fn known_models_include_phi4() {
        let m = lookup_model("phi-4").expect("phi-4 not found");
        assert_eq!(m.name, "Phi-4 14B (Ollama)");
        assert_eq!(m.context_window, 16_384);
    }

    #[test]
    fn known_models_llama32_vision_has_vision() {
        let m = lookup_model("llama3.2-vision").expect("llama3.2-vision not found");
        assert!(m.capabilities.vision, "Llama 3.2 Vision should have vision");
    }

    #[test]
    fn known_models_local_count_matches() {
        let models = known_models();
        let local: Vec<_> = models
            .iter()
            .filter(|m| {
                matches!(
                    m.id.as_str(),
                    "llama3.3" | "llama3.2-vision" | "gemma4" | "mistral" | "qwen2.5" | "phi-4"
                )
            })
            .collect();
        assert_eq!(local.len(), 6, "expected 6 local model presets");
    }

    #[test]
    fn model_capabilities_default() {
        let caps = ModelCapabilities::default();
        assert!(!caps.vision);
        assert!(!caps.reasoning);
        assert!(caps.function_calling);
        assert!(caps.streaming);
    }

    // ── models_with_capabilities / models_for_provider (#2970) ──

    #[test]
    fn models_with_capabilities_empty_require_returns_all() {
        // No capabilities required → every preset passes through.
        let all = models_with_capabilities(ModelCapabilities::default());
        assert_eq!(all.len(), known_models().len());
    }

    #[test]
    fn models_with_capabilities_vision_narrows_set() {
        let vision_only = models_with_capabilities(ModelCapabilities {
            vision: true,
            ..Default::default()
        });
        assert!(
            !vision_only.is_empty(),
            "expected at least one vision model"
        );
        assert!(vision_only.iter().all(|m| m.capabilities.vision));
        // Local/Ollama non-vision models should be excluded.
        assert!(vision_only.iter().all(|m| m.id != "llama3.3"));
    }

    #[test]
    fn models_with_capabilities_reasoning_and_vision() {
        let both = models_with_capabilities(ModelCapabilities {
            vision: true,
            reasoning: true,
            ..Default::default()
        });
        assert!(both
            .iter()
            .all(|m| m.capabilities.vision && m.capabilities.reasoning));
        // A vision-only model without reasoning (Llama 3.2 Vision) must be dropped.
        assert!(both.iter().all(|m| m.id != "llama3.2-vision"));
    }

    #[test]
    fn models_for_provider_anthropic_only() {
        let anthropic = models_for_provider(ProviderType::Anthropic);
        assert!(!anthropic.is_empty());
        assert!(anthropic
            .iter()
            .all(|m| m.provider == ProviderType::Anthropic));
        assert!(anthropic.iter().all(|m| m.id != "gpt-5.5"));
    }

    #[test]
    fn models_for_provider_olama_only() {
        let ollama = models_for_provider(ProviderType::Ollama);
        assert_eq!(ollama.len(), 6, "expected 6 local Ollama presets");
        assert!(ollama.iter().all(|m| m.provider == ProviderType::Ollama));
    }
}
