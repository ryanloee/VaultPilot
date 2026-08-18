//! Provider connection probe (#3480) + model auto-detection (#3489).
//!
//! Shared by the sidecar agent's `checkProviderConnection` RPC and the
//! desktop Tauri `test_provider_connection` command, so both surfaces get
//! identical "test connection" behaviour.

use serde::{Deserialize, Serialize};

/// Probe request — mirrors the agent RPC wire format (camelCase).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckProviderConnectionParams {
    /// API base URL (e.g. https://api.openai.com/v1)
    pub api_base: String,
    /// API key (may be masked for round-trip; if so, return an error so the
    /// caller knows to use the real key)
    pub api_key: String,
    /// Provider type string: "openai", "anthropic", "ollama"
    #[serde(default)]
    pub provider_type: String,
    /// Optional model name (not strictly needed for /models probe, but kept
    /// for future use such as a targeted chat-completion ping).
    #[serde(default)]
    #[allow(dead_code)]
    pub model: Option<String>,
    /// Optional timeout in milliseconds (default 8000)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Probe outcome — serialized back to the caller as camelCase.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionResult {
    /// True when the upstream returned HTTP 2xx for the probe request.
    pub ok: bool,
    /// HTTP status code if we got an HTTP response, else null.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Human-readable error message (Chinese) when the probe failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The URL that was probed (for diagnostics).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_url: Option<String>,
    /// Available model names discovered during the probe (#3489).
    ///
    /// Populated from Ollama `/api/tags` (`.models[].name`) or
    /// OpenAI-compatible `/models` (`data[].id`). Empty when the probe
    /// failed or the response body could not be parsed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Whether an actual chat message was sent and accepted (HTTP 2xx).
    /// `None` when no `model` was supplied (probe-only mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_ok: Option<bool>,
    /// Error detail from the chat ping when it failed (e.g. quota/balance
    /// errors that /models cannot detect).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_error: Option<String>,
    /// HTTP status of the chat ping (useful for quota/429 diagnostics).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_status: Option<u16>,
}

/// Probes the configured provider by hitting its `/models` endpoint.
///
/// OpenAI-compatible providers answer `GET /v1/models` with Bearer auth;
/// Anthropic answers `GET /v1/models` with `x-api-key`. Ollama has no auth
/// on `/api/tags`. Returns a structured result, never panics.
pub async fn check_provider_connection(
    params: &CheckProviderConnectionParams,
) -> ProviderConnectionResult {
    use std::time::Duration;

    const DEFAULT_TIMEOUT_MS: u64 = 8_000;
    const MASK_SENTINEL: &str = "********";

    // Masked key indicates the caller sent the stored (masked) settings rather
    // than the live dialog input — reject so the client knows to send the
    // freshly typed key.
    if params.api_key.is_empty() {
        return ProviderConnectionResult {
            ok: false,
            status: None,
            error: Some("API Key 未填写".to_string()),
            probe_url: None,
            models: vec![],
            ping_ok: None,
            ping_error: None,
            ping_status: None,
        };
    }
    if params.api_key == MASK_SENTINEL {
        return ProviderConnectionResult {
            ok: false,
            status: None,
            error: Some("API Key 已掩码，无法测试；请重新输入完整 Key".to_string()),
            probe_url: None,
            models: vec![],
            ping_ok: None,
            ping_error: None,
            ping_status: None,
        };
    }

    let timeout = Duration::from_millis(params.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(e) => {
            return ProviderConnectionResult {
                ok: false,
                status: None,
                error: Some(format!("构造 HTTP 客户端失败: {e}")),
                probe_url: None,
                models: vec![],
                ping_ok: None,
                ping_error: None,
                ping_status: None,
            }
        }
    };

    let ptype = params.provider_type.to_ascii_lowercase();
    let base = params.api_base.trim_end_matches('/').to_string();

    let result = match ptype.as_str() {
        "anthropic" => {
            let url = format!("{}/v1/models", base);
            let resp = client
                .get(&url)
                .header("x-api-key", &params.api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await;
            (url, resp)
        }
        "ollama" => {
            // Ollama exposes /api/tags without auth
            let url = format!("{}/api/tags", base);
            let resp = client.get(&url).send().await;
            (url, resp)
        }
        // Default: OpenAI-compatible
        _ => {
            let url = format!("{}/models", base);
            let resp = client
                .get(&url)
                .header("Authorization", format!("Bearer {}", params.api_key))
                .send()
                .await;
            (url, resp)
        }
    };

    let (probe_url, resp_result) = result;
    match resp_result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            // Read the body once and reuse it for both status reporting and
            // model-list extraction (#3489 auto-detection).
            let body = resp.text().await.unwrap_or_default();
            if (200..300).contains(&status) {
                let models = match ptype.as_str() {
                    "ollama" => parse_ollama_tags_response(&body),
                    "anthropic" => parse_openai_models_response(&body),
                    _ => parse_openai_models_response(&body),
                };
                // /models alone cannot detect quota/balance failures — send a
                // minimal chat message so the test reflects real usability.
                let ping = match params.model.as_deref() {
                    Some(m) if !m.trim().is_empty() => {
                        chat_ping(&client, &ptype, &base, &params.api_key, m).await
                    }
                    _ => None,
                };
                match ping {
                    Some((ping_status, ping_error)) if !(200..300).contains(&ping_status) => {
                        ProviderConnectionResult {
                            ok: false,
                            status: Some(ping_status),
                            error: Some(ping_error.clone()),
                            probe_url: Some(probe_url),
                            models,
                            ping_ok: Some(false),
                            ping_error: Some(ping_error),
                            ping_status: Some(ping_status),
                        }
                    }
                    Some((ping_status, _)) => ProviderConnectionResult {
                        ok: true,
                        status: Some(ping_status),
                        error: None,
                        probe_url: Some(probe_url),
                        models,
                        ping_ok: Some(true),
                        ping_error: None,
                        ping_status: Some(ping_status),
                    },
                    // No model supplied — probe-only result.
                    None => ProviderConnectionResult {
                        ok: true,
                        status: Some(status),
                        error: None,
                        probe_url: Some(probe_url),
                        models,
                        ping_ok: None,
                        ping_error: None,
                        ping_status: None,
                    },
                }
            } else {
                let snippet = body.chars().take(200).collect::<String>();
                ProviderConnectionResult {
                    ok: false,
                    status: Some(status),
                    error: Some(format!("HTTP {status}: {snippet}")),
                    probe_url: Some(probe_url),
                    models: vec![],
                    ping_ok: None,
                    ping_error: None,
                    ping_status: None,
                }
            }
        }
        Err(e) => {
            let msg = if e.is_connect() {
                "无法连接到供应商（连接被拒绝/DNS 失败）".to_string()
            } else if e.is_timeout() {
                "连接超时（请检查网络或增大超时设置）".to_string()
            } else {
                format!("{e}")
            };
            ProviderConnectionResult {
                ok: false,
                status: None,
                error: Some(msg),
                probe_url: Some(probe_url),
                models: vec![],
                ping_ok: None,
                ping_error: None,
                ping_status: None,
            }
        }
    }
}

/// Sends one minimal chat message to the provider and reports whether it was
/// accepted. This is what catches quota/balance/usage-limit failures that a
/// `/models` probe passes silently.
///
/// Returns `Some((status, error_message))` on HTTP-response failure and
/// `Some((status, ""))` on success; `None` when the request could not be sent
/// (network/timeout — the probe already covered that, so degrade to probe-only
/// by skipping the ping). The error message is extracted from common
/// `{"error": {"message": ...}}` response shapes when possible.
async fn chat_ping(
    client: &reqwest::Client,
    ptype: &str,
    base: &str,
    api_key: &str,
    model: &str,
) -> Option<(u16, String)> {
    let body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": "ping" }],
        "max_tokens": 1,
    });
    let result = match ptype {
        "anthropic" => {
            let url = format!("{base}/v1/messages");
            client
                .post(&url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await
                .map(|r| (url, r))
        }
        "ollama" => {
            let url = format!("{base}/api/chat");
            let body = serde_json::json!({
                "model": model,
                "messages": [{ "role": "user", "content": "ping" }],
                "stream": false,
            });
            client.post(&url).json(&body).send().await.map(|r| (url, r))
        }
        _ => {
            let url = format!("{base}/chat/completions");
            client
                .post(&url)
                .bearer_auth(api_key)
                .json(&body)
                .send()
                .await
                .map(|r| (url, r))
        }
    };

    let (_url, resp) = result.ok()?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if (200..300).contains(&status) {
        Some((status, String::new()))
    } else {
        Some((status, extract_ping_error(&text, status)))
    }
}

/// Pulls a human-readable error out of a failed chat response body, falling
/// back to a short raw snippet. Handles OpenAI/Anthropic-style
/// `{"error": {"message": "..."}}` and plain text bodies.
fn extract_ping_error(body: &str, status: u16) -> String {
    let snippet = || {
        let s = body.trim();
        if s.is_empty() {
            format!("HTTP {status}")
        } else {
            s.chars().take(200).collect::<String>()
        }
    };
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").cloned())
        .and_then(|e| {
            if let Some(m) = e.get("message").and_then(|m| m.as_str()) {
                Some(m.to_string())
            } else if let Some(s) = e.as_str() {
                Some(s.to_string())
            } else {
                None
            }
        })
        .map(|m| m.chars().take(300).collect::<String>())
        .unwrap_or_else(snippet)
}

// ── Model auto-detection parsers (#3489) ───────────────────────

/// Parse an Ollama `/api/tags` response body and return the list of
/// installed model names.
///
/// Ollama's response shape:
/// ```json
/// { "models": [ { "name": "llama3.2:latest", ... }, { "name": "mistral:7b", ... } ] }
/// ```
///
/// Returns an empty `Vec` on any parse failure (never panics) so that a
/// malformed upstream body degrades gracefully — the connection probe still
/// reports `ok: true` with the HTTP status, just without a model list.
pub fn parse_ollama_tags_response(body: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct TagsResponse {
        #[serde(default)]
        models: Vec<TagsModel>,
    }
    #[derive(serde::Deserialize)]
    struct TagsModel {
        name: String,
    }

    serde_json::from_str::<TagsResponse>(body)
        .map(|r| {
            r.models
                .into_iter()
                .map(|m| m.name)
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Parse an OpenAI-compatible (and Anthropic) `/models` (or `/v1/models`)
/// response body and return the list of model ids.
///
/// OpenAI shape: `{ "data": [ { "id": "gpt-4o-mini", ... } ] }`
/// Anthropic shape: `{ "data": [ { "id": "claude-3-5-sonnet-20241022", ... } ] }`
///
/// Returns an empty `Vec` on any parse failure (never panics).
pub fn parse_openai_models_response(body: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        #[serde(default)]
        data: Vec<ModelsEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ModelsEntry {
        id: String,
    }

    serde_json::from_str::<ModelsResponse>(body)
        .map(|r| {
            r.data
                .into_iter()
                .map(|m| m.id)
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn check_provider_connection_params_camel_case_roundtrip() {
        let wire = json!({
            "apiBase": "https://api.openai.com/v1",
            "apiKey": "sk-test-123",
            "providerType": "openai",
            "model": "gpt-4o-mini",
            "timeoutMs": 5000
        });
        let params: CheckProviderConnectionParams = serde_json::from_value(wire).unwrap();
        assert_eq!(params.api_base, "https://api.openai.com/v1");
        assert_eq!(params.api_key, "sk-test-123");
        assert_eq!(params.provider_type, "openai");
        assert_eq!(params.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(params.timeout_ms, Some(5000));
    }

    #[test]
    fn check_provider_connection_params_accepts_minimal_payload() {
        // Only apiBase + apiKey are required; the rest default.
        let wire = json!({
            "apiBase": "https://opencode.ai/zen/v1",
            "apiKey": "sk-min"
        });
        let params: CheckProviderConnectionParams = serde_json::from_value(wire).unwrap();
        assert_eq!(params.provider_type, ""); // serde default
        assert_eq!(params.model, None);
        assert_eq!(params.timeout_ms, None);
    }

    #[test]
    fn check_provider_connection_rejects_empty_api_key() {
        let params = CheckProviderConnectionParams {
            api_base: "https://api.openai.com/v1".into(),
            api_key: "".into(),
            provider_type: "openai".into(),
            model: None,
            timeout_ms: Some(1000),
        };
        // We can't easily await in #[test] without a runtime; exercise the
        // early-return path via a tokio runtime block_on.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(check_provider_connection(&params));
        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("API Key"));
    }

    #[test]
    fn check_provider_connection_rejects_masked_api_key() {
        let params = CheckProviderConnectionParams {
            api_base: "https://api.openai.com/v1".into(),
            api_key: "********".into(),
            provider_type: "openai".into(),
            model: None,
            timeout_ms: Some(1000),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(check_provider_connection(&params));
        assert!(!result.ok);
        assert!(
            result.error.unwrap_or_default().contains("掩码"),
            "masked key should yield a masked-error message"
        );
    }

    #[test]
    fn provider_connection_result_serializes_camel_case() {
        let r = ProviderConnectionResult {
            ok: true,
            status: Some(200),
            error: None,
            probe_url: Some("https://api.openai.com/v1/models".into()),
            models: vec![],
            ping_ok: None,
            ping_error: None,
            ping_status: None,
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["status"], 200);
        assert_eq!(v["probeUrl"], "https://api.openai.com/v1/models");
        // error should be skipped because it's None
        assert!(v.get("error").is_none() || v["error"].is_null());
        // empty models should be skipped (skip_serializing_if = "Vec::is_empty")
        assert!(v.get("models").is_none() || v["models"].is_null());
        // None ping fields should be skipped
        assert!(v.get("pingOk").is_none());
        assert!(v.get("pingError").is_none());
        assert!(v.get("pingStatus").is_none());
    }

    #[test]
    fn provider_connection_result_with_models_serializes() {
        // When models is non-empty it should appear in the JSON payload so the
        // client can render a model picker (#3489).
        let r = ProviderConnectionResult {
            ok: true,
            status: Some(200),
            error: None,
            probe_url: Some("http://localhost:11434/api/tags".into()),
            models: vec!["llama3.2:latest".into(), "mistral:7b".into()],
            ping_ok: Some(true),
            ping_error: None,
            ping_status: Some(200),
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(
            v["models"],
            serde_json::json!(["llama3.2:latest", "mistral:7b"])
        );
        assert_eq!(v["pingOk"], true);
        assert_eq!(v["pingStatus"], 200);
    }

    // ── #3489: Ollama / OpenAI model auto-detection ──────────────

    #[test]
    fn parse_ollama_tags_response_extracts_model_names() {
        let body = r#"{
            "models": [
                { "name": "llama3.2:latest", "size": 2000000000 },
                { "name": "mistral:7b", "size": 4100000000 },
                { "name": "qwen2.5:14b", "size": 9000000000 }
            ]
        }"#;
        let names = parse_ollama_tags_response(body);
        assert_eq!(names, vec!["llama3.2:latest", "mistral:7b", "qwen2.5:14b"]);
    }

    #[test]
    fn parse_ollama_tags_response_empty_models() {
        let body = r#"{ "models": [] }"#;
        assert!(parse_ollama_tags_response(body).is_empty());
    }

    #[test]
    fn parse_ollama_tags_response_missing_models_key() {
        // A server that returns an unexpected shape should not panic — we
        // degrade to an empty list so the connection probe still reports ok.
        let body = r#"{ "error": "something else" }"#;
        assert!(parse_ollama_tags_response(body).is_empty());
    }

    #[test]
    fn parse_ollama_tags_response_malformed_json() {
        assert!(parse_ollama_tags_response("not json at all").is_empty());
        assert!(parse_ollama_tags_response("").is_empty());
    }

    #[test]
    fn parse_ollama_tags_response_skips_empty_names() {
        let body = r#"{ "models": [ { "name": "" }, { "name": "real:latest" } ] }"#;
        assert_eq!(parse_ollama_tags_response(body), vec!["real:latest"]);
    }

    #[test]
    fn parse_openai_models_response_extracts_ids() {
        let body = r#"{
            "data": [
                { "id": "gpt-4o-mini", "object": "model" },
                { "id": "gpt-4o", "object": "model" }
            ]
        }"#;
        let ids = parse_openai_models_response(body);
        assert_eq!(ids, vec!["gpt-4o-mini", "gpt-4o"]);
    }

    #[test]
    fn parse_openai_models_response_anthropic_shape() {
        // Anthropic /v1/models uses the same { data: [ { id } ] } shape.
        let body = r#"{
            "data": [
                { "id": "claude-3-5-sonnet-20241022", "type": "model" },
                { "id": "claude-3-5-haiku-20241022", "type": "model" }
            ]
        }"#;
        let ids = parse_openai_models_response(body);
        assert_eq!(
            ids,
            vec!["claude-3-5-sonnet-20241022", "claude-3-5-haiku-20241022"]
        );
    }

    #[test]
    fn parse_openai_models_response_empty_and_malformed() {
        assert!(parse_openai_models_response(r#"{ "data": [] }"#).is_empty());
        assert!(parse_openai_models_response("garbage").is_empty());
        // Missing "data" key entirely
        assert!(parse_openai_models_response(r#"{ "object": "list" }"#).is_empty());
    }

    // ── chat ping (#3412) ─────────────────────────────────────────

    #[test]
    fn extract_ping_error_openai_error_object() {
        let body = r#"{
            "error": {
                "message": "You exceeded your current quota, please check your plan and billing details.",
                "type": "insufficient_quota"
            }
        }"#;
        let msg = extract_ping_error(body, 429);
        assert!(msg.contains("quota"), "got: {msg}");
    }

    #[test]
    fn extract_ping_error_anthropic_error_object() {
        let body = r#"{
            "type": "error",
            "error": {
                "type": "overloaded_error",
                "message": "Overloaded: try again later"
            }
        }"#;
        let msg = extract_ping_error(body, 529);
        assert!(msg.contains("Overloaded"), "got: {msg}");
    }

    #[test]
    fn extract_ping_error_error_is_plain_string() {
        // Some providers return { "error": "rate limit" } — a string, not object.
        let body = r#"{ "error": "rate limit exceeded" }"#;
        let msg = extract_ping_error(body, 429);
        assert!(msg.contains("rate limit"), "got: {msg}");
    }

    #[test]
    fn extract_ping_error_falls_back_to_snippet() {
        assert!(extract_ping_error("not json", 500).contains("not json"));
        assert!(extract_ping_error("", 503).contains("503"));
    }

    #[test]
    fn extract_ping_error_truncates_long_messages() {
        let long = "x".repeat(500);
        let body = serde_json::json!({ "error": { "message": long } }).to_string();
        let msg = extract_ping_error(&body, 400);
        assert!(msg.chars().count() <= 300, "message too long: {}", msg.len());
    }
}