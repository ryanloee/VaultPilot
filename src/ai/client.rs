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
use serde::{Deserialize, Serialize};
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
    resolved_addrs: Vec<(String, SocketAddr)>,
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
            && cached.resolved_addrs == resolved_addrs
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
        resolved_addrs: resolved_addrs.to_vec(),
    });
    Ok(client)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub(super) stream: bool,
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
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub(super) stream: bool,
}

/// Request struct for OpenAI reasoning models (o1/o3/o4) which require
/// `max_completion_tokens` instead of `max_tokens` and do not support `temperature`.
#[derive(Debug, Serialize)]
pub(super) struct OpenAiReasoningRequest<'a> {
    pub(super) model: &'a str,
    pub(super) max_completion_tokens: u32,
    pub(super) messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub(super) stream: bool,
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
                stream: false,
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
                    stream: false,
                };
                serde_json::to_vec(&payload)?
            } else {
                let payload = OpenAiRequest {
                    model: &provider.model,
                    max_tokens: max_output,
                    temperature,
                    messages,
                    stream: false,
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
/// Send a streaming request to the AI provider. Calls `on_chunk` for each
/// text delta received. Returns the full accumulated text.
pub async fn send_request_streaming(
    settings: &AppSettings,
    system: &str,
    prompt: &str,
    image_paths: &[String],
    temperature: f32,
    mut on_chunk: impl FnMut(&str),
) -> Result<String> {
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
                stream: true,
            };
            serde_json::to_vec(&payload)?.into()
        }
        crate::models::ProviderType::OpenAi => {
            let messages =
                build_openai_messages(&provider.model, system, prompt, image_paths).await?;
            let max_output = resolve_max_output_tokens(&provider.model, provider.max_output_tokens);
            let body_bytes = if is_openai_reasoning_model(&provider.model) {
                let payload = OpenAiReasoningRequest {
                    model: &provider.model,
                    max_completion_tokens: max_output,
                    messages,
                    stream: true,
                };
                serde_json::to_vec(&payload)?
            } else {
                let payload = OpenAiRequest {
                    model: &provider.model,
                    max_tokens: max_output,
                    temperature,
                    messages,
                    stream: true,
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
                    warn!(attempt = attempt + 1, error = %crate::sanitize_error(&error.to_string()), "streaming transport error, retrying");
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
        if !status.is_success() {
            // Read the error body with the same size cap as the success path to
            // avoid OOM when a misconfigured endpoint returns a huge non-2xx body.
            // (Issue #2145: previously `response.text()` read to EOF with no limit.)
            let mut err_buf = BytesMut::new();
            let mut err_stream = response.bytes_stream();
            while let Some(chunk) = err_stream.next().await {
                let chunk = chunk.map_err(|e| anyhow!(format_transport_error(&e, &endpoint)))?;
                if err_buf.len() + chunk.len() > MAX_RESPONSE_SIZE {
                    return Err(anyhow!(
                        "Streaming API error response body exceeds {}MB size limit, possible misconfigured endpoint",
                        MAX_RESPONSE_SIZE / (1024 * 1024)
                    ));
                }
                err_buf.extend_from_slice(&chunk);
            }
            let text = String::from_utf8_lossy(&err_buf).to_string();

            if is_retryable_provider_error(status.as_u16(), &text) && attempt < 2 {
                warn!(
                    attempt = attempt + 1,
                    status = status.as_u16(),
                    "retryable streaming API error, retrying"
                );
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
                "Streaming API request failed ({}): {}",
                status.as_u16(),
                crate::sanitize_error(&text)
            ));
        }

        let mut accumulated = String::new();
        let mut buf = BytesMut::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| anyhow!(format_transport_error(&e, &endpoint)))?;
            if buf.len() + chunk.len() > MAX_RESPONSE_SIZE {
                return Err(anyhow!(
                    "Streaming API response exceeds {}MB size limit, possible misconfigured endpoint",
                    MAX_RESPONSE_SIZE / (1024 * 1024)
                ));
            }
            buf.extend_from_slice(&chunk);

            // Process complete lines
            while let Some(newline_pos) = buf.iter().position(|&b| b == b'\n') {
                let line_bytes = buf.split_to(newline_pos + 1);
                let line = std::str::from_utf8(&line_bytes)
                    .unwrap_or("")
                    .trim_end_matches('\r')
                    .trim_end_matches('\n');

                if line.is_empty() {
                    continue;
                }

                // Skip "event:" lines (Anthropic) and "id:" / "retry:" metadata
                if line.starts_with("event:")
                    || line.starts_with("id:")
                    || line.starts_with("retry:")
                {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        if accumulated.trim().is_empty() {
                            return Err(anyhow!("API returned an empty response"));
                        }
                        return Ok(accumulated);
                    }

                    // Parse SSE data based on provider type
                    match provider_type {
                        crate::models::ProviderType::OpenAi => {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                                if let Some(text) =
                                    parsed["choices"][0]["delta"]["content"].as_str()
                                {
                                    if !text.is_empty() {
                                        if accumulated.len() + text.len() > MAX_RESPONSE_SIZE {
                                            return Err(anyhow!(
                                                "Streaming response text exceeds {}MB size limit",
                                                MAX_RESPONSE_SIZE / (1024 * 1024)
                                            ));
                                        }
                                        accumulated.push_str(text);
                                        on_chunk(text);
                                    }
                                }
                            }
                        }
                        crate::models::ProviderType::Anthropic => {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                                let event_type = parsed["type"].as_str().unwrap_or("");
                                if event_type == "content_block_delta" {
                                    if let Some(text) = parsed["delta"]["text"].as_str() {
                                        if !text.is_empty() {
                                            if accumulated.len() + text.len() > MAX_RESPONSE_SIZE {
                                                return Err(anyhow!(
                                                    "Streaming response text exceeds {}MB size limit",
                                                    MAX_RESPONSE_SIZE / (1024 * 1024)
                                                ));
                                            }
                                            accumulated.push_str(text);
                                            on_chunk(text);
                                        }
                                    }
                                } else if event_type == "message_stop" {
                                    if accumulated.trim().is_empty() {
                                        return Err(anyhow!("API returned an empty response"));
                                    }
                                    return Ok(accumulated);
                                } else if event_type == "error" {
                                    let error_type =
                                        parsed["error"]["type"].as_str().unwrap_or("unknown");
                                    let error_message = parsed["error"]["message"]
                                        .as_str()
                                        .unwrap_or("unknown error");
                                    return Err(anyhow!(
                                        "Anthropic API error ({}): {}",
                                        error_type,
                                        error_message
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Process any remaining data in buf that wasn't terminated by a newline.
        // The stream may end without a trailing \n, leaving the final SSE frame
        // unprocessed. (Fixes #1597)
        if !buf.is_empty() {
            let line = std::str::from_utf8(&buf)
                .unwrap_or("")
                .trim_end_matches('\r')
                .trim_end_matches('\n');

            if !line.is_empty()
                && !line.starts_with("event:")
                && !line.starts_with("id:")
                && !line.starts_with("retry:")
            {
                if let Some(data) = line.strip_prefix("data: ") {
                    let data = data.trim();
                    if data != "[DONE]" {
                        match provider_type {
                            crate::models::ProviderType::OpenAi => {
                                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                                {
                                    if let Some(text) =
                                        parsed["choices"][0]["delta"]["content"].as_str()
                                    {
                                        if !text.is_empty() {
                                            accumulated.push_str(text);
                                            on_chunk(text);
                                        }
                                    }
                                }
                            }
                            crate::models::ProviderType::Anthropic => {
                                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                                {
                                    let event_type = parsed["type"].as_str().unwrap_or("");
                                    if event_type == "content_block_delta" {
                                        if let Some(text) = parsed["delta"]["text"].as_str() {
                                            if !text.is_empty() {
                                                accumulated.push_str(text);
                                                on_chunk(text);
                                            }
                                        }
                                    } else if event_type == "error" {
                                        let error_type =
                                            parsed["error"]["type"].as_str().unwrap_or("unknown");
                                        let error_message = parsed["error"]["message"]
                                            .as_str()
                                            .unwrap_or("unknown error");
                                        return Err(anyhow!(
                                            "Anthropic API error ({}): {}",
                                            error_type,
                                            error_message
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if accumulated.trim().is_empty() {
            return Err(anyhow!("API returned an empty response"));
        }
        return Ok(accumulated);
    }

    Err(anyhow!("Streaming API request failed after retries"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ProviderType;
    use std::net::{Ipv4Addr, Ipv6Addr};

    // ── detect_image_media_type ──────────────────────────────────

    #[test]
    fn detect_png() {
        assert_eq!(detect_image_media_type("photo.png").unwrap(), "image/png");
    }

    #[test]
    fn detect_jpg() {
        assert_eq!(detect_image_media_type("photo.jpg").unwrap(), "image/jpeg");
    }

    #[test]
    fn detect_jpeg() {
        assert_eq!(detect_image_media_type("photo.jpeg").unwrap(), "image/jpeg");
    }

    #[test]
    fn detect_webp() {
        assert_eq!(detect_image_media_type("photo.webp").unwrap(), "image/webp");
    }

    #[test]
    fn detect_gif() {
        assert_eq!(detect_image_media_type("photo.gif").unwrap(), "image/gif");
    }

    #[test]
    fn detect_uppercase_extension() {
        assert_eq!(detect_image_media_type("photo.PNG").unwrap(), "image/png");
    }

    #[test]
    fn detect_with_path() {
        assert_eq!(
            detect_image_media_type("/home/user/vault/images/test.jpeg").unwrap(),
            "image/jpeg"
        );
    }

    #[test]
    fn detect_unsupported_format() {
        assert!(detect_image_media_type("photo.bmp").is_err());
    }

    #[test]
    fn detect_no_extension() {
        assert!(detect_image_media_type("Makefile").is_err());
    }

    // ── is_private_ip ────────────────────────────────────────────

    #[test]
    fn private_10_x() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
    }

    #[test]
    fn private_172_16() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
    }

    #[test]
    fn private_192_168() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    }

    #[test]
    fn loopback_127() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
    }

    #[test]
    fn link_local_169_254() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
    }

    #[test]
    fn broadcast() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255))));
    }

    #[test]
    fn unspecified_0_0_0_0() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
    }

    #[test]
    fn cgnat_100_64() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
    }

    #[test]
    fn benchmarking_198_18() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1))));
    }

    #[test]
    fn reserved_240() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(240, 0, 0, 1))));
    }

    #[test]
    fn public_ip() {
        assert!(!is_private_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn public_ip_cloudflare() {
        assert!(!is_private_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    #[test]
    fn ipv6_loopback() {
        assert!(is_private_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn ipv6_unspecified() {
        assert!(is_private_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
    }

    #[test]
    fn ipv6_link_local() {
        assert!(is_private_ip(IpAddr::V6("fe80::1".parse().unwrap())));
    }

    #[test]
    fn ipv6_unique_local() {
        assert!(is_private_ip(IpAddr::V6("fc00::1".parse().unwrap())));
    }

    #[test]
    fn ipv6_public() {
        assert!(!is_private_ip(IpAddr::V6(
            "2606:4700:4700::1111".parse().unwrap()
        )));
    }

    #[test]
    fn ipv6_ipv4_mapped_private() {
        // ::ffff:10.0.0.1 maps to 10.0.0.1 which is private
        assert!(is_private_ip(IpAddr::V6(
            "::ffff:10.0.0.1".parse().unwrap()
        )));
    }

    #[test]
    fn ipv6_ipv4_mapped_public() {
        // ::ffff:8.8.8.8 maps to 8.8.8.8 which is public
        assert!(!is_private_ip(IpAddr::V6(
            "::ffff:8.8.8.8".parse().unwrap()
        )));
    }

    // ── normalize_endpoint ───────────────────────────────────────

    #[test]
    fn normalize_anthropic_full_path() {
        assert_eq!(
            normalize_endpoint(
                "https://api.anthropic.com/v1/messages",
                ProviderType::Anthropic
            ),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn normalize_anthropic_v1_suffix() {
        assert_eq!(
            normalize_endpoint("https://api.anthropic.com/v1", ProviderType::Anthropic),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn normalize_anthropic_bare_url() {
        assert_eq!(
            normalize_endpoint("https://api.anthropic.com", ProviderType::Anthropic),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn normalize_openai_full_path() {
        assert_eq!(
            normalize_endpoint(
                "https://api.openai.com/v1/chat/completions",
                ProviderType::OpenAi
            ),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn normalize_openai_v1_suffix() {
        assert_eq!(
            normalize_endpoint("https://api.openai.com/v1", ProviderType::OpenAi),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn normalize_openai_bare_url() {
        assert_eq!(
            normalize_endpoint("https://api.openai.com", ProviderType::OpenAi),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn normalize_trailing_slash() {
        assert_eq!(
            normalize_endpoint("https://api.openai.com/", ProviderType::OpenAi),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn normalize_whitespace() {
        assert_eq!(
            normalize_endpoint("  https://api.openai.com  ", ProviderType::OpenAi),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    // ── normalize_messages_endpoint ──────────────────────────────

    #[test]
    fn normalize_messages_legacy_wrapper() {
        assert_eq!(
            normalize_messages_endpoint("https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    // ── is_retryable_provider_error ──────────────────────────────

    #[test]
    fn retryable_429() {
        assert!(is_retryable_provider_error(429, "rate limited"));
    }

    #[test]
    fn retryable_500() {
        assert!(is_retryable_provider_error(500, "internal error"));
    }

    #[test]
    fn retryable_502() {
        assert!(is_retryable_provider_error(502, "bad gateway"));
    }

    #[test]
    fn retryable_503() {
        assert!(is_retryable_provider_error(503, "service unavailable"));
    }

    #[test]
    fn retryable_504() {
        assert!(is_retryable_provider_error(504, "gateway timeout"));
    }

    #[test]
    fn retryable_chinese_rate_limit() {
        assert!(is_retryable_provider_error(200, "访问量过大"));
    }

    #[test]
    fn retryable_too_many_requests() {
        assert!(is_retryable_provider_error(
            200,
            "Too many requests, please slow down"
        ));
    }

    #[test]
    fn retryable_rate_limit_in_detail() {
        assert!(is_retryable_provider_error(200, "Rate limit exceeded"));
    }

    #[test]
    fn not_retryable_400() {
        assert!(!is_retryable_provider_error(400, "bad request"));
    }

    #[test]
    fn not_retryable_401() {
        assert!(!is_retryable_provider_error(401, "unauthorized"));
    }

    #[test]
    fn not_retryable_403() {
        assert!(!is_retryable_provider_error(403, "forbidden"));
    }

    #[test]
    fn not_retryable_404() {
        assert!(!is_retryable_provider_error(404, "not found"));
    }

    #[test]
    fn not_retryable_501() {
        assert!(!is_retryable_provider_error(501, "not implemented"));
    }

    // ── format_transport_error ───────────────────────────────────

    #[tokio::test]
    async fn format_timeout_error() {
        // Make a request that will timeout
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(1))
            .build()
            .unwrap();
        let err = client
            .get("http://192.0.2.1:12345/test") // TEST-NET, unreachable
            .send()
            .await
            .unwrap_err();
        let msg = format_transport_error(&err, "http://api.example.com/v1/chat");
        // Should contain the host, not the path
        assert!(msg.contains("api.example.com"));
        assert!(!msg.contains("/v1/chat"));
    }

    #[tokio::test]
    async fn format_connect_error() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let err = client
            .get("http://127.0.0.1:1/v1/test")
            .send()
            .await
            .unwrap_err();
        let msg = format_transport_error(&err, "http://127.0.0.1:1/v1/test");
        assert!(msg.contains("127.0.0.1"));
    }

    #[tokio::test]
    async fn format_strips_userinfo() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(1))
            .build()
            .unwrap();
        let err = client
            .get("http://192.0.2.1:12345/test")
            .send()
            .await
            .unwrap_err();
        let msg = format_transport_error(&err, "https://user:pass@api.example.com/v1");
        // Should NOT contain credentials
        assert!(!msg.contains("user:pass"));
        assert!(!msg.contains("/v1"));
        assert!(msg.contains("api.example.com"));
    }

    // ── should_retry_transport_error ─────────────────────────────

    #[tokio::test]
    async fn should_retry_connect_error() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let err = client
            .get("http://127.0.0.1:1/test")
            .send()
            .await
            .unwrap_err();
        assert!(should_retry_transport_error(&err));
    }
}
