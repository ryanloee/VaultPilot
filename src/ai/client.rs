//! HTTP client, request construction, API communication, retry logic.

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Serialize;
use tokio::time::{sleep, timeout};
use tracing::{info, instrument, warn};
use url::Url;

use super::context::{is_openai_reasoning_model, resolve_max_output_tokens};
use super::parsing::{AnthropicResponse, OpenAiResponse};
use crate::models::AppSettings;

pub(super) const MAX_RESPONSE_SIZE: usize = 50 * 1024 * 1024;

pub(super) struct CachedClient {
    client: reqwest::Client,
    // Fingerprint of the config used to build this client.
    api_key: String,
    timeout_ms: u64,
    provider_type: crate::models::ProviderType,
    base_url: String,
}

static CACHED_CLIENT: Mutex<Option<CachedClient>> = Mutex::new(None);

pub(super) fn get_or_build_client(
    api_key: &str,
    timeout_ms: u64,
    provider_type: crate::models::ProviderType,
    base_url: &str,
    resolved_addrs: &[(String, SocketAddr)],
) -> Result<reqwest::Client> {
    let mut cache = CACHED_CLIENT.lock().unwrap_or_else(|e| {
        tracing::warn!("CACHED_CLIENT lock poisoned, recovering inner value");
        e.into_inner()
    });
    if let Some(ref cached) = *cache {
        if cached.api_key == api_key
            && cached.timeout_ms == timeout_ms
            && cached.provider_type == provider_type
            && cached.base_url == base_url
        {
            return Ok(cached.client.clone());
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    use crate::models::ProviderType;
    match provider_type {
        ProviderType::Anthropic => {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(api_key).context("invalid API key")?,
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        }
        ProviderType::OpenAi => {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {api_key}"))
                    .context("invalid API key for Bearer auth")?,
            );
        }
    }

    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .default_headers(headers);

    // Pin DNS to the addresses verified by validate_base_url to prevent
    // DNS rebinding TOCTOU attacks (issue #503).
    for (host, addr) in resolved_addrs {
        builder = builder.resolve(host, *addr);
    }

    let client = builder.build()?;

    *cache = Some(CachedClient {
        client: client.clone(),
        api_key: api_key.to_string(),
        timeout_ms,
        provider_type,
        base_url: base_url.to_string(),
    });
    Ok(client)
}

#[derive(Debug, Clone, Default)]
pub struct RequestUsage {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
}

pub(super) struct ModelResponse {
    pub(super) text: String,
    pub(super) usage: RequestUsage,
}

#[derive(Debug, Serialize)]
pub(super) struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    temperature: f32,
    system: &'a str,
    messages: Vec<AnthropicMessage>,
}

#[derive(Debug, Serialize)]
pub(super) struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicInputBlock>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum AnthropicInputBlock {
    Text { text: String },
    Image { source: AnthropicImageSource },
}

#[derive(Debug, Serialize, Clone)]
pub(super) struct AnthropicImageSource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

#[derive(Debug, Serialize)]

pub(super) struct OpenAiRequest<'a> {
    pub(super) model: &'a str,
    pub(super) max_tokens: u32,
    pub(super) temperature: f32,
    pub(super) messages: Vec<OpenAiMessage>,
}

/// Request struct for OpenAI reasoning models (o1/o3/o4) which require
/// `max_completion_tokens` instead of `max_tokens` and do not support `temperature`.
#[derive(Debug, Serialize)]
pub(super) struct OpenAiReasoningRequest<'a> {
    pub(super) model: &'a str,
    pub(super) max_completion_tokens: u32,
    pub(super) messages: Vec<OpenAiMessage>,
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiMessage {
    pub(super) role: String,
    pub(super) content: OpenAiContent,
}

/// OpenAI content can be a plain string (text-only) or an array of parts
/// (when images are present).
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum OpenAiContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum OpenAiContentPart {
    Text {
        text: String,
    },
    #[serde(rename = "image_url")]
    ImageUrl {
        image_url: OpenAiImageUrl,
    },
}

#[derive(Debug, Serialize, Clone)]
pub(super) struct OpenAiImageUrl {
    url: String,
}

pub(super) async fn send_request(
    settings: &AppSettings,
    system: &str,
    prompt: &str,
    image_paths: &[String],
) -> Result<ModelResponse> {
    send_request_with_temperature(settings, system, prompt, image_paths, 0.2).await
}

#[instrument(skip(settings, system, prompt, image_paths), fields(model = %settings.effective_provider().model, temperature))]
pub(super) async fn send_request_with_temperature(
    settings: &AppSettings,
    system: &str,
    prompt: &str,
    image_paths: &[String],
    temperature: f32,
) -> Result<ModelResponse> {
    let provider = settings.effective_provider();
    if provider.api_key.trim().is_empty() {
        return Err(anyhow!("API key is empty"));
    }

    let provider_type = provider.effective_provider_type();
    let resolved_addrs = validate_base_url(&provider.base_url).await?;
    let client = get_or_build_client(
        &provider.api_key,
        provider.request_timeout_ms,
        provider_type,
        &provider.base_url,
        &resolved_addrs,
    )?;

    let endpoint = normalize_endpoint(&provider.base_url, provider_type);

    let body: Bytes = match provider_type {
        crate::models::ProviderType::Anthropic => {
            let content_blocks = build_input_blocks(prompt, image_paths).await?;
            let payload = AnthropicRequest {
                model: &provider.model,
                max_tokens: resolve_max_output_tokens(&provider.model, provider.max_output_tokens),
                temperature,
                system,
                messages: vec![AnthropicMessage {
                    role: "user".to_string(),
                    content: content_blocks,
                }],
            };
            serde_json::to_vec(&payload)?.into()
        }
        crate::models::ProviderType::OpenAi => {
            let messages =
                build_openai_messages(&provider.model, system, prompt, image_paths).await?;
            let max_output = resolve_max_output_tokens(&provider.model, provider.max_output_tokens);
            let body_bytes = if is_openai_reasoning_model(&provider.model) {
                // Reasoning models (o1/o3/o4) require max_completion_tokens
                // and do not support temperature.
                let payload = OpenAiReasoningRequest {
                    model: &provider.model,
                    max_completion_tokens: max_output,
                    messages,
                };
                serde_json::to_vec(&payload)?
            } else {
                let payload = OpenAiRequest {
                    model: &provider.model,
                    max_tokens: max_output,
                    temperature,
                    messages,
                };
                serde_json::to_vec(&payload)?
            };
            body_bytes.into()
        }
    };

    for attempt in 0..3 {
        let response = match client
            .post(&endpoint)
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                if should_retry_transport_error(&error) && attempt < 2 {
                    warn!(attempt = attempt + 1, error = %crate::sanitize_error(&error.to_string()), "transport error, retrying");
                    // Issue #749: add jitter to prevent thundering herd
                    let base = 2u64.pow(attempt as u32 + 1);
                    let jitter = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_nanos() as u64
                        % base;
                    sleep(Duration::from_secs(base + jitter)).await;
                    continue;
                }
                return Err(anyhow!(format_transport_error(&error, &endpoint)));
            }
        };
        let status = response.status();
        let mut buf = BytesMut::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|error| anyhow!(format_transport_error(&error, &endpoint)))?;
            if buf.len() + chunk.len() > MAX_RESPONSE_SIZE {
                return Err(anyhow!(
                    "API response body exceeds {}MB size limit, possible misconfigured endpoint",
                    MAX_RESPONSE_SIZE / (1024 * 1024)
                ));
            }
            buf.extend_from_slice(&chunk);
        }
        let text = String::from_utf8(buf.to_vec()).map_err(|e| {
            anyhow!(
                "API response is not valid UTF-8 (invalid byte at position {})",
                e.utf8_error().valid_up_to()
            )
        })?;

        if !status.is_success() {
            // Try to extract a human-readable error message from the response,
            // using the format appropriate for the provider.
            let detail = match provider_type {
                crate::models::ProviderType::Anthropic => {
                    serde_json::from_str::<AnthropicResponse>(&text)
                        .ok()
                        .and_then(|value| value.error.map(|error| error.message))
                        .filter(|message| !message.trim().is_empty())
                        .unwrap_or(text.clone())
                }
                crate::models::ProviderType::OpenAi => {
                    serde_json::from_str::<OpenAiResponse>(&text)
                        .ok()
                        .and_then(|value| value.error.map(|error| error.message))
                        .filter(|message| !message.trim().is_empty())
                        .unwrap_or(text.clone())
                }
            };

            if is_retryable_provider_error(status.as_u16(), &detail) && attempt < 2 {
                warn!(
                    attempt = attempt + 1,
                    status = status.as_u16(),
                    "retryable API error, retrying"
                );
                // Issue #749: add jitter to prevent thundering herd
                let base = 2u64.pow(attempt as u32 + 1);
                let jitter = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as u64
                    % base;
                sleep(Duration::from_secs(base + jitter)).await;
                continue;
            }

            return Err(anyhow!(
                "API request failed ({}): {}",
                status.as_u16(),
                crate::sanitize_error(&detail)
            ));
        }

        // Parse the response using the format appropriate for the provider.
        let (joined, usage) = match provider_type {
            crate::models::ProviderType::Anthropic => {
                let parsed: AnthropicResponse =
                    serde_json::from_str(&text).context("failed to parse API response")?;
                let joined = parsed
                    .content
                    .into_iter()
                    .filter(|block| block.kind == "text")
                    .filter_map(|block| block.text)
                    .collect::<Vec<_>>()
                    .join("\n");
                let usage = RequestUsage {
                    input_tokens: Some(parsed.usage.input_tokens),
                    output_tokens: Some(parsed.usage.output_tokens),
                };
                info!(
                    input_tokens = parsed.usage.input_tokens,
                    output_tokens = parsed.usage.output_tokens,
                    "API request completed"
                );
                (joined, usage)
            }
            crate::models::ProviderType::OpenAi => {
                let parsed: OpenAiResponse =
                    serde_json::from_str(&text).context("failed to parse API response")?;
                let joined = parsed
                    .choices
                    .first()
                    .and_then(|choice| choice.message.content.clone())
                    .unwrap_or_default();
                let usage = RequestUsage {
                    input_tokens: Some(parsed.usage.prompt_tokens),
                    output_tokens: Some(parsed.usage.completion_tokens),
                };
                info!(
                    input_tokens = parsed.usage.prompt_tokens,
                    output_tokens = parsed.usage.completion_tokens,
                    "API request completed"
                );
                (joined, usage)
            }
        };

        if joined.trim().is_empty() {
            return Err(anyhow!("API returned an empty response"));
        }

        return Ok(ModelResponse {
            text: joined,
            usage,
        });
    }

    Err(anyhow!("API request failed after retries"))
}

pub(super) fn should_retry_transport_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

pub(super) fn format_transport_error(error: &reqwest::Error, endpoint: &str) -> String {
    // Extract just the host from the endpoint URL to avoid leaking API paths
    let host = endpoint
        .split("://")
        .nth(1)
        .and_then(|s| s.split('/').next())
        // Strip userinfo (user:pass@host) to avoid leaking credentials in error messages
        .and_then(|s| s.split('@').next_back())
        .unwrap_or("(unknown)");
    if error.is_timeout() {
        return format!("请求超时。模型服务长时间没有响应：{}", host);
    }
    if error.is_connect() {
        return format!("网络连接失败，无法连接到模型服务：{}", host);
    }
    if error.is_request() {
        return format!("请求发送失败，请检查 Base URL、网络或代理配置：{}", host);
    }
    if error.is_decode() {
        return "模型服务返回的数据格式无法解析。".to_string();
    }
    format!(
        "调用模型服务失败：{}",
        crate::sanitize_error(&error.to_string())
    )
}

pub(super) async fn build_input_blocks(
    prompt: &str,
    image_paths: &[String],
) -> Result<Vec<AnthropicInputBlock>> {
    let mut blocks = vec![AnthropicInputBlock::Text {
        text: prompt.to_string(),
    }];

    for path in image_paths {
        let media_type = detect_image_media_type(path)?;
        // Guard against OOM from excessively large image files (issue #141)
        const MAX_IMAGE_SIZE: u64 = 20 * 1024 * 1024; // 20 MB
        let metadata = tokio::fs::metadata(path).await.with_context(|| {
            format!(
                "failed to stat image: {}",
                std::path::Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        })?;
        if metadata.len() > MAX_IMAGE_SIZE {
            return Err(anyhow!(
                "image file too large: {} ({} MB > 20 MB limit)",
                std::path::Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                metadata.len() / (1024 * 1024)
            ));
        }
        let data = tokio::fs::read(path).await.with_context(|| {
            format!(
                "failed to read image: {}",
                std::path::Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        })?;
        blocks.push(AnthropicInputBlock::Image {
            source: AnthropicImageSource {
                kind: "base64".to_string(),
                media_type: media_type.to_string(),
                data: STANDARD.encode(data),
            },
        });
    }

    Ok(blocks)
}

/// Build the messages array for an OpenAI-compatible request.
///
/// The system prompt is emitted as a `system` role message (or `developer` for
/// OpenAI reasoning models o1/o3/o4 per #742). User content is a plain string
/// when no images are attached, or an array of content parts when images are
/// present.
pub(super) async fn build_openai_messages(
    model: &str,
    system: &str,
    prompt: &str,
    image_paths: &[String],
) -> Result<Vec<OpenAiMessage>> {
    let mut messages = Vec::new();

    if !system.is_empty() {
        // #742: Reasoning models (o1/o3/o4) require "developer" role instead of "system"
        let system_role = if is_openai_reasoning_model(model) {
            "developer"
        } else {
            "system"
        };
        messages.push(OpenAiMessage {
            role: system_role.to_string(),
            content: OpenAiContent::Text(system.to_string()),
        });
    }

    if image_paths.is_empty() {
        messages.push(OpenAiMessage {
            role: "user".to_string(),
            content: OpenAiContent::Text(prompt.to_string()),
        });
    } else {
        let mut parts: Vec<OpenAiContentPart> = vec![OpenAiContentPart::Text {
            text: prompt.to_string(),
        }];

        for path in image_paths {
            let media_type = detect_image_media_type(path)?;
            const MAX_IMAGE_SIZE: u64 = 20 * 1024 * 1024;
            let metadata = tokio::fs::metadata(path).await.with_context(|| {
                format!(
                    "failed to stat image: {}",
                    std::path::Path::new(path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )
            })?;
            if metadata.len() > MAX_IMAGE_SIZE {
                return Err(anyhow!(
                    "image file too large: {} ({} MB > 20 MB limit)",
                    std::path::Path::new(path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    metadata.len() / (1024 * 1024)
                ));
            }
            let data = tokio::fs::read(path).await.with_context(|| {
                format!(
                    "failed to read image: {}",
                    std::path::Path::new(path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )
            })?;
            parts.push(OpenAiContentPart::ImageUrl {
                image_url: OpenAiImageUrl {
                    url: format!("data:{};base64,{}", media_type, STANDARD.encode(data)),
                },
            });
        }

        messages.push(OpenAiMessage {
            role: "user".to_string(),
            content: OpenAiContent::Parts(parts),
        });
    }

    Ok(messages)
}

pub(super) fn detect_image_media_type(path: &str) -> Result<&'static str> {
    let fname = Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("unknown");
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .ok_or_else(|| anyhow!("unsupported image format: {fname}"))?;

    match extension.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "webp" => Ok("image/webp"),
        "gif" => Ok("image/gif"),
        _ => Err(anyhow!("unsupported image format: {fname}")),
    }
}

/// Validate that a base_url is safe to use as a request endpoint, and
/// resolve DNS to pin the verified addresses.
///
/// Checks:
/// 1. URL must parse as valid HTTP or HTTPS.
/// 2. Host must be present.
/// 3. Rejects RFC 1918 / loopback / link-local addresses (SSRF protection)
///    unless `VAULTPILOT_ALLOW_LOCAL_ENDPOINT` env var is set.
///
/// Returns a list of `(hostname, SocketAddr)` pairs that were verified to be
/// non-private. These can be passed to `reqwest::ClientBuilder::resolve()` to
/// pin DNS and prevent rebinding attacks (TOCTOU fix, #503).
pub(super) async fn validate_base_url(base_url: &str) -> Result<Vec<(String, SocketAddr)>> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("base_url is empty"));
    }

    let parsed = Url::parse(trimmed).context("base_url is not a valid URL")?;

    match parsed.scheme() {
        "https" => {}
        "http" => {
            warn!(
                "base_url uses HTTP (not HTTPS); \
                 consider using HTTPS for production endpoints"
            );
        }
        other => {
            return Err(anyhow!(
                "base_url scheme '{}' is not supported; use http or https",
                other
            ));
        }
    }

    let host_str = parsed
        .host_str()
        .ok_or_else(|| anyhow!("base_url has no host component"))?;

    // Allow explicit opt-in to local/private endpoints via env var.
    // Still resolve DNS for pinning even when local endpoints are allowed.
    let allow_local = std::env::var("VAULTPILOT_ALLOW_LOCAL_ENDPOINT").is_ok();
    if allow_local && (host_str == "localhost" || host_str.parse::<IpAddr>().is_ok()) {
        return Ok(Vec::new());
    }
    if allow_local {
        // Resolve DNS and return addresses (skip private IP check) to enable DNS pinning.
        let port = parsed
            .port_or_known_default()
            .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
        // 10s DNS timeout to avoid consuming the full request budget (issue #603).
        return match timeout(
            Duration::from_secs(10),
            tokio::net::lookup_host(format!("{}:{}", host_str, port)),
        )
        .await
        {
            Ok(Ok(addrs)) => Ok(addrs.map(|a| (host_str.to_string(), a)).collect()),
            Ok(Err(e)) => Err(anyhow!(
                "failed to resolve base_url host '{}': {}",
                host_str,
                e
            )),
            Err(_) => Err(anyhow!(
                "DNS resolution timed out (10s) for base_url host '{}'",
                host_str
            )),
        };
    }

    if host_str == "localhost" {
        return Err(anyhow!(
            "base_url points to 'localhost'; set VAULTPILOT_ALLOW_LOCAL_ENDPOINT=1 \
             to allow local endpoints"
        ));
    }

    if let Ok(ip) = host_str.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err(anyhow!(
                "base_url resolves to a private/reserved IP ({}); \
                 set VAULTPILOT_ALLOW_LOCAL_ENDPOINT=1 to allow",
                ip
            ));
        }
        Ok(Vec::new())
    } else {
        // For hostnames that aren't literal IPs, resolve DNS and check each address.
        let port = parsed
            .port_or_known_default()
            .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
        // 10s DNS timeout to avoid consuming the full request budget (issue #603).
        match timeout(
            Duration::from_secs(10),
            tokio::net::lookup_host(format!("{}:{}", host_str, port)),
        )
        .await
        {
            Ok(Ok(addrs)) => {
                let mut resolved = Vec::new();
                for addr in addrs {
                    if is_private_ip(addr.ip()) {
                        return Err(anyhow!(
                            "base_url host '{}' resolves to a private/reserved IP ({}); \\
                             set VAULTPILOT_ALLOW_LOCAL_ENDPOINT=1 to allow",
                            host_str,
                            addr.ip()
                        ));
                    }
                    resolved.push((host_str.to_string(), addr));
                }
                Ok(resolved)
            }
            Ok(Err(e)) => Err(anyhow!(
                "failed to resolve base_url host '{}': {}",
                host_str,
                e
            )),
            Err(_) => Err(anyhow!(
                "DNS resolution timed out (10s) for base_url host '{}'",
                host_str
            )),
        }
    }
}

/// Returns `true` for RFC 1918, loopback, link-local, and other reserved IPs.
pub(super) fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || matches!(v4.octets(), [100, 64..=127, _, _]) // CGNAT 100.64.0.0/10
                || matches!(v4.octets(), [198, 18..=19, _, _]) // Benchmarking 198.18.0.0/15
                || matches!(v4.octets()[0], 240..=255)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_private_ip(IpAddr::V4(v4))) // IPv4-mapped
        }
    }
}

/// Route to the correct API endpoint based on provider type.
pub(super) fn normalize_endpoint(
    base_url: &str,
    provider_type: crate::models::ProviderType,
) -> String {
    use crate::models::ProviderType;
    let trimmed = base_url.trim().trim_end_matches('/');
    match provider_type {
        ProviderType::Anthropic => {
            // Only recognize the full canonical path, not arbitrary suffixes
            // like /custom/messages (issue #602).
            if trimmed.ends_with("/v1/messages") {
                trimmed.to_string()
            } else if trimmed.ends_with("/v1") {
                format!("{trimmed}/messages")
            } else {
                format!("{trimmed}/v1/messages")
            }
        }
        ProviderType::OpenAi => {
            if trimmed.ends_with("/v1/chat/completions") {
                trimmed.to_string()
            } else if trimmed.ends_with("/v1") {
                format!("{trimmed}/chat/completions")
            } else {
                format!("{trimmed}/v1/chat/completions")
            }
        }
    }
}

/// Legacy wrapper for backward compatibility (Anthropic-only).
#[allow(dead_code)]
pub(super) fn normalize_messages_endpoint(base_url: &str) -> String {
    normalize_endpoint(base_url, crate::models::ProviderType::Anthropic)
}

pub(super) fn is_retryable_provider_error(status: u16, detail: &str) -> bool {
    // #792: Only retry specific 5xx codes that indicate transient failures.
    // 501 (Not Implemented), 505 (HTTP Version Not Supported), etc. are
    // permanent failures that waste retry attempts with exponential backoff.
    status == 429
        || status == 500  // Internal Server Error — transient backend failure
        || status == 502  // Bad Gateway
        || status == 503  // Service Unavailable
        || status == 504  // Gateway Timeout
        || detail.contains("访问量过大")
        || detail.to_ascii_lowercase().contains("too many requests")
        || detail.to_ascii_lowercase().contains("rate limit")
}
