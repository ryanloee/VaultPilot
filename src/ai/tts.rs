//! Text-to-Speech provider abstraction for Audio Overview.
//!
//! Defines a generic [`TtsProvider`] trait and provides an OpenAI TTS
//! implementation. New TTS backends (ElevenLabs, Gemini TTS, etc.) can
//! be added by implementing the trait and registering in [`create_tts_provider`].
//!
//! # Provider config reuse
//! OpenAI TTS reuses the existing [`ProviderConfig`] from the settings — the
//! same API key and base URL work (OpenAI TTS is behind `/v1/audio/speech`).
//! The TTS-specific voice/model is configured via [`TtsConfig`] which has
//! sensible defaults and can be overridden per-call.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::ProviderConfig;

// ── Voices ───────────────────────────────────────────────────────────────

/// Supported TTS voices for OpenAI TTS.
///
/// See https://platform.openai.com/docs/guides/text-to-speech#voice-options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TtsVoice {
    #[default]
    Alloy,
    Echo,
    Fable,
    Onyx,
    Nova,
    Shimmer,
}

impl TtsVoice {
    /// Return the OpenAI API voice identifier.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Alloy => "alloy",
            Self::Echo => "echo",
            Self::Fable => "fable",
            Self::Onyx => "onyx",
            Self::Nova => "nova",
            Self::Shimmer => "shimmer",
        }
    }

    /// All available voices.
    pub fn all() -> Vec<Self> {
        vec![
            Self::Alloy,
            Self::Echo,
            Self::Fable,
            Self::Onyx,
            Self::Nova,
            Self::Shimmer,
        ]
    }
}

impl std::fmt::Display for TtsVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ── TTS Configuration ────────────────────────────────────────────────────

/// TTS-specific configuration that can be overridden per call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsConfig {
    /// The TTS model to use (OpenAI: "tts-1" or "tts-1-hd").
    #[serde(default = "default_tts_model")]
    pub model: String,
    /// Voice for the first speaker (Host A / primary).
    #[serde(default)]
    pub voice_a: TtsVoice,
    /// Voice for the second speaker (Host B / co-host). Only used in dual-host formats.
    #[serde(default = "default_voice_b")]
    pub voice_b: TtsVoice,
    /// Playback speed (0.25–4.0, default 1.0).
    #[serde(default = "default_speed")]
    pub speed: f64,
    /// Output audio format (OpenAI supports "mp3", "opus", "aac", "flac", "wav", "pcm").
    #[serde(default = "default_format")]
    pub format: String,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            model: default_tts_model(),
            voice_a: TtsVoice::Alloy,
            voice_b: default_voice_b(),
            speed: default_speed(),
            format: default_format(),
        }
    }
}

fn default_tts_model() -> String {
    "tts-1".to_string()
}

fn default_voice_b() -> TtsVoice {
    TtsVoice::Nova
}

fn default_speed() -> f64 {
    1.0
}

fn default_format() -> String {
    "mp3".to_string()
}

// ── Provider trait ───────────────────────────────────────────────────────

/// Result of a TTS synthesis call.
#[derive(Debug, Clone)]
pub struct TtsResult {
    /// Raw audio bytes (format depends on [`TtsConfig::format`]).
    pub audio_data: Vec<u8>,
    /// Duration of the audio in seconds (approximate).
    pub duration_secs: f64,
}

/// Abstract TTS provider with async synthesis.
#[async_trait::async_trait]
pub trait TtsProvider: Send + Sync {
    /// Synthesize text into audio.
    async fn synthesize(
        &self,
        text: &str,
        voice: TtsVoice,
        config: &TtsConfig,
    ) -> Result<TtsResult>;
}

// ── OpenAI TTS provider ──────────────────────────────────────────────────

/// OpenAI TTS provider that calls `POST /v1/audio/speech`.
pub struct OpenAiTtsProvider {
    provider: ProviderConfig,
    client: reqwest::Client,
}

impl OpenAiTtsProvider {
    /// Create a new OpenAI TTS provider using the given provider config.
    pub fn new(provider: ProviderConfig) -> Result<Self> {
        let timeout =
            std::time::Duration::from_millis(std::cmp::max(provider.request_timeout_ms, 30_000));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .context("failed to build HTTP client for TTS")?;
        Ok(Self { provider, client })
    }

    /// Build the OpenAI audio-speech endpoint URL.
    fn endpoint(&self) -> String {
        let base = self.provider.base_url.trim_end_matches('/');
        if base.contains("/v1") {
            format!("{}/audio/speech", base)
        } else {
            format!("{}/v1/audio/speech", base)
        }
    }
}

#[async_trait::async_trait]
impl TtsProvider for OpenAiTtsProvider {
    async fn synthesize(
        &self,
        text: &str,
        voice: TtsVoice,
        config: &TtsConfig,
    ) -> Result<TtsResult> {
        let endpoint = self.endpoint();
        let api_key = self.provider.api_key.trim();

        let body = serde_json::json!({
            "model": config.model,
            "input": text,
            "voice": voice.as_str(),
            "speed": config.speed,
            "response_format": config.format,
        });

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("TTS API request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(anyhow::anyhow!(
                "TTS API returned {}: {}",
                status,
                crate::sanitize_error(&err_text)
            ));
        }

        let audio_data = response
            .bytes()
            .await
            .context("failed to read TTS response body")?
            .to_vec();

        // Approximate duration: OpenAI tts-1 produces ~180 chars/sec at speed 1.0.
        let approx_chars_per_sec = 180.0 * config.speed;
        let duration_secs = text.len() as f64 / approx_chars_per_sec;

        Ok(TtsResult {
            audio_data,
            duration_secs,
        })
    }
}

// ── Factory ──────────────────────────────────────────────────────────────

/// Create a TTS provider from the given provider config.
pub fn create_tts_provider(provider: ProviderConfig) -> Result<Box<dyn TtsProvider>> {
    Ok(Box::new(OpenAiTtsProvider::new(provider)?))
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tts_voice_roundtrip() {
        for voice in TtsVoice::all() {
            let s = voice.as_str();
            assert!(!s.is_empty(), "voice should have a name");
        }
    }

    #[test]
    fn tts_voice_default_is_alloy() {
        assert_eq!(TtsVoice::default(), TtsVoice::Alloy);
    }

    #[test]
    fn tts_config_defaults() {
        let cfg = TtsConfig::default();
        assert_eq!(cfg.model, "tts-1");
        assert_eq!(cfg.voice_a, TtsVoice::Alloy);
        assert_eq!(cfg.voice_b, TtsVoice::Nova);
        assert_eq!(cfg.speed, 1.0);
        assert_eq!(cfg.format, "mp3");
    }

    #[test]
    fn tts_voice_display() {
        assert_eq!(TtsVoice::Alloy.to_string(), "alloy");
        assert_eq!(TtsVoice::Echo.to_string(), "echo");
        assert_eq!(TtsVoice::Nova.to_string(), "nova");
    }

    #[test]
    fn openai_tts_endpoint_detection() {
        let provider = ProviderConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: "test-key".to_string(),
            ..ProviderConfig::default()
        };
        let tts = OpenAiTtsProvider::new(provider).unwrap();
        assert!(tts.endpoint().contains("/v1/audio/speech"));
    }

    #[test]
    fn openai_tts_endpoint_bare_host() {
        let provider = ProviderConfig {
            base_url: "https://api.openai.com".to_string(),
            api_key: "test-key".to_string(),
            ..ProviderConfig::default()
        };
        let tts = OpenAiTtsProvider::new(provider).unwrap();
        assert!(tts.endpoint().contains("/v1/audio/speech"));
    }
}
