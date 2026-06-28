pub mod client;
pub mod context;
pub mod parsing;

pub use context::{resolve_context_window, resolve_max_output_tokens};

// Re-export public types so callers can use `ai::ChatAnswerResult` etc.
pub use client::{send_request_streaming, RequestUsage};
pub use parsing::{
    AssistantToolCall, ChatAnswerResult, RecordInteractionResult, ToolSelectionResult,
};

use std::collections::HashSet;

use anyhow::{anyhow, Context, Result};
use tracing::instrument;

use crate::models::{AppSettings, ConversationTurn, NoteDocument, NoteMeta, StructuredNoteDraft};
use crate::prompting;

use client::{send_request, send_request_with_temperature};
use parsing::{
    enrich_citations, extract_json, parse_or_fallback_answer, parse_or_fallback_note,
    parse_record_response, parse_tool_call, CompressionResponse, NoteSelectionResponse,
};

#[instrument(skip(settings, raw_input, image_paths), fields(input_len = raw_input.len()))]
pub async fn organize_note(
    settings: &AppSettings,
    raw_input: &str,
    image_paths: &[String],
) -> Result<StructuredNoteDraft> {
    let system = prompting::ingest_system_prompt();
    let prompt = prompting::ingest_user_prompt(raw_input);
    let response =
        send_request_with_temperature(settings, &system, &prompt, image_paths, 0.1).await?;
    Ok(parse_or_fallback_note(&response.text, raw_input))
}

#[instrument(skip(settings, question, image_paths, history, prior_tool_results))]
pub async fn select_tool_call(
    settings: &AppSettings,
    question: &str,
    image_paths: &[String],
    history: &[ConversationTurn],
    prior_tool_results: &[String],
) -> Result<ToolSelectionResult> {
    let system = prompting::tool_call_system_prompt();
    let prompt = prompting::tool_call_user_prompt(
        question,
        !image_paths.is_empty(),
        history,
        prior_tool_results,
    );
    let response =
        send_request_with_temperature(settings, &system, &prompt, image_paths, 0.1).await?;

    let (tool_call, usage) = match parse_tool_call(&response.text, question) {
        Ok(tool_call) => (tool_call, response.usage),
        Err(_) => {
            let retry_prompt = prompting::tool_call_retry_user_prompt(
                question,
                !image_paths.is_empty(),
                history,
                prior_tool_results,
                &response.text,
            );
            let retry_response =
                send_request_with_temperature(settings, &system, &retry_prompt, image_paths, 0.1)
                    .await?;
            let tool_call = parse_tool_call(&retry_response.text, question).with_context(|| {
                format!(
                    "model did not return a valid tool call after retry; last response: {}",
                    crate::sanitize_error(retry_response.text.trim())
                )
            })?;
            (tool_call, retry_response.usage)
        }
    };

    Ok(ToolSelectionResult { tool_call, usage })
}

#[instrument(skip(settings, question, docs, image_paths, history))]
pub async fn answer_question(
    settings: &AppSettings,
    question: &str,
    docs: &[NoteDocument],
    image_paths: &[String],
    history: &[ConversationTurn],
) -> Result<ChatAnswerResult> {
    let (system, prompt) = if docs.is_empty() {
        (
            prompting::general_chat_system_prompt(),
            prompting::general_chat_user_prompt(question, history),
        )
    } else {
        (
            prompting::answer_system_prompt(),
            prompting::answer_user_prompt(question, docs, history),
        )
    };

    let response = send_request(settings, &system, &prompt, image_paths).await?;
    let parsed = parse_or_fallback_answer(&response.text, question, docs.is_empty());

    Ok(ChatAnswerResult {
        answer: parsed.answer,
        citations: if docs.is_empty() {
            Vec::new()
        } else {
            enrich_citations(parsed.citations, docs)
        },
        usage: response.usage,
    })
}

#[instrument(skip(settings, question, tool_name, tool_result, docs, history))]
pub async fn answer_after_tool(
    settings: &AppSettings,
    question: &str,
    tool_name: &str,
    tool_result: &str,
    docs: &[NoteDocument],
    history: &[ConversationTurn],
) -> Result<ChatAnswerResult> {
    let system = prompting::tool_result_system_prompt();
    let prompt =
        prompting::tool_result_user_prompt(question, tool_name, tool_result, docs, history);
    let response = send_request(settings, &system, &prompt, &[]).await?;
    let parsed = parse_or_fallback_answer(&response.text, question, docs.is_empty());

    Ok(ChatAnswerResult {
        answer: parsed.answer,
        citations: if docs.is_empty() {
            Vec::new()
        } else {
            enrich_citations(parsed.citations, docs)
        },
        usage: response.usage,
    })
}

#[instrument(skip(settings, question, tool_results, docs, history))]
pub async fn answer_after_tools(
    settings: &AppSettings,
    question: &str,
    tool_results: &[String],
    docs: &[NoteDocument],
    history: &[ConversationTurn],
) -> Result<ChatAnswerResult> {
    let system = prompting::tool_result_system_prompt();
    let prompt = prompting::multi_tool_result_user_prompt(question, tool_results, docs, history);
    let response = send_request(settings, &system, &prompt, &[]).await?;
    let parsed = parse_or_fallback_answer(&response.text, question, docs.is_empty());

    Ok(ChatAnswerResult {
        answer: parsed.answer,
        citations: if docs.is_empty() {
            Vec::new()
        } else {
            enrich_citations(parsed.citations, docs)
        },
        usage: response.usage,
    })
}

#[instrument(skip(settings, raw_input, docs, image_paths))]
pub async fn record_note_interaction(
    settings: &AppSettings,
    raw_input: &str,
    docs: &[NoteDocument],
    image_paths: &[String],
) -> Result<RecordInteractionResult> {
    let system = prompting::record_system_prompt();
    let prompt = prompting::record_user_prompt(raw_input, docs);
    let response = send_request(settings, &system, &prompt, image_paths).await?;
    parse_record_response(&response.text, raw_input, response.usage)
}

#[instrument(skip(settings, prompt, docs))]
pub async fn generate_with_context(
    settings: &AppSettings,
    prompt: &str,
    docs: &[NoteDocument],
    _mode: &str,
) -> Result<String> {
    let system = prompting::write_system_prompt();
    let user_prompt = prompting::write_user_prompt(prompt, docs);
    let response = send_request(settings, &system, &user_prompt, &[]).await?;
    Ok(response.text.trim().to_string())
}

#[instrument(skip(settings, existing_summary, history))]
pub async fn compress_conversation(
    settings: &AppSettings,
    existing_summary: &str,
    history: &[ConversationTurn],
) -> Result<String> {
    let system = prompting::compression_system_prompt();
    let prompt = prompting::compression_user_prompt(existing_summary, history);
    let response = send_request(settings, &system, &prompt, &[]).await?;

    let json = extract_json(&response.text)
        .map_err(|_| anyhow!("model did not return valid JSON for conversation compression"))?;
    let parsed: CompressionResponse = serde_json::from_str(&json).map_err(|e| {
        anyhow!(
            "failed to parse conversation compression response: {}",
            crate::sanitize_error(&e.to_string())
        )
    })?;
    let summary = parsed.summary.trim();
    if summary.is_empty() {
        return Err(anyhow!("model returned an empty conversation summary"));
    }
    Ok(summary.to_string())
}

#[instrument(skip(settings, question, candidates, history))]
pub async fn select_relevant_note_ids(
    settings: &AppSettings,
    question: &str,
    candidates: &[NoteMeta],
    history: &[ConversationTurn],
) -> Result<Vec<String>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let system = prompting::note_selection_system_prompt();
    let prompt = prompting::note_selection_user_prompt(question, candidates, history);
    let response = send_request_with_temperature(settings, &system, &prompt, &[], 0.1).await?;

    if let Ok(json) = extract_json(&response.text) {
        if let Ok(parsed) = serde_json::from_str::<NoteSelectionResponse>(&json) {
            let candidate_ids = candidates
                .iter()
                .map(|note| note.id.as_str())
                .collect::<HashSet<_>>();
            let ids = parsed
                .note_ids
                .into_iter()
                .filter(|id| candidate_ids.contains(id.as_str()))
                .take(4)
                .collect::<Vec<_>>();
            if !ids.is_empty() {
                return Ok(ids);
            }
        }
    }

    Ok(candidates
        .iter()
        .take(3)
        .map(|note| note.id.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::client::{
        is_private_ip, is_retryable_provider_error, normalize_messages_endpoint, OpenAiContent,
        OpenAiMessage, OpenAiReasoningRequest, OpenAiRequest, RequestUsage,
    };
    use super::context::{
        is_openai_reasoning_model, resolve_context_window, resolve_max_output_tokens,
    };
    use super::parsing::{
        dedupe_terms, extract_json, extract_json_block, fallback_answer,
        generate_programmatic_snippet, heuristic_note_from_input, normalize_draft,
        parse_or_fallback_answer, parse_or_fallback_note, parse_record_response, parse_tool_call,
        AssistantToolCall,
    };
    use crate::models::{AppSettings, ProviderConfig, StructuredNoteDraft};

    /// Serialise tests that mutate the VAULTPILOT_ALLOW_LOCAL_ENDPOINT env-var
    /// to prevent parallel race conditions in CI.
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn extracts_json_from_code_fence_like_payload() {
        let raw = "```json\n{\"answer\":\"ok\"}\n```";
        let extracted = extract_json(raw).expect("json extracted");
        assert_eq!(extracted, "{\"answer\":\"ok\"}");
    }

    #[test]
    fn extracts_clean_json_directly() {
        let raw = r#"{"answer":"hello","citations":[]}"#;
        assert_eq!(extract_json(raw).expect("extract"), raw);
    }

    #[test]
    fn extracts_json_from_surrounding_prose() {
        let raw = r#"The result is {"answer":"ok"} great"#;
        let extracted = extract_json(raw).expect("extract");
        assert!(extracted.contains("answer"));
    }

    #[test]
    fn extract_json_returns_err_without_braces() {
        assert!(extract_json("no json here").is_err());
    }

    #[test]
    fn extract_json_handles_nested_objects() {
        let raw = r#"{"a":{"b":1},"c":2}"#;
        let extracted = extract_json(raw).expect("extract nested");
        assert!(extracted.contains("\"a\""));
        assert!(extracted.contains("\"b\""));
    }

    #[test]
    fn extract_json_handles_braces_inside_strings() {
        let raw = r#"Here is the result: {"answer": "use {var} syntax", "ok": true} done"#;
        let extracted = extract_json(raw).expect("extract with braces in string");
        assert!(extracted.contains("{var}"));
        assert!(extracted.contains("\"ok\": true"));
    }

    #[test]
    fn extract_json_block_double_escaped_backslash() {
        let text = r#"{"key": "value\\"}more"#;
        let result = extract_json_block(text, '{', '}');
        assert!(result.is_some(), "Should handle double-escaped backslash");
        let json = result.unwrap();
        assert!(json.ends_with('}'), "JSON should end with closing brace");
    }

    #[test]
    fn extract_json_block_skips_prose_braces() {
        let text = r#"Here is some text {with braces} and then {"key": "value"}"#;
        let result = extract_json_block(text, '{', '}');
        let json = result.expect("should find JSON");
        assert!(json.contains("\"key\""));
        assert!(json.contains("\"value\""));
    }

    #[test]
    fn appends_messages_path_for_provider_base_url() {
        assert_eq!(
            normalize_messages_endpoint("https://open.bigmodel.cn/api/anthropic"),
            "https://open.bigmodel.cn/api/anthropic/v1/messages"
        );
        assert_eq!(
            normalize_messages_endpoint("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn normalize_endpoint_preserves_trailing_slash_url() {
        let result = normalize_messages_endpoint("https://api.example.com/v1/messages/");
        assert!(result.contains("/v1/messages"));
    }

    #[test]
    fn normalize_endpoint_appends_for_bare_host() {
        let result = normalize_messages_endpoint("https://api.example.com");
        assert_eq!(result, "https://api.example.com/v1/messages");
    }

    #[test]
    fn fallback_mentions_direct_answer_when_no_context() {
        let text = fallback_answer("我之前怎么做的", true);
        assert!(text.contains("直接回答"));
    }

    #[test]
    fn heuristically_builds_note_for_command_records() {
        let draft =
            heuristic_note_from_input("我发送刷机命令你记录一下，wboot -w update zboot.img");
        assert!(draft.title.contains("刷机命令"));
        assert!(draft.body.contains("wboot -w update zboot.img"));
    }

    #[test]
    fn uses_plain_text_when_model_does_not_return_json() {
        let parsed = parse_or_fallback_answer("你好，我记住了。", "记录一下", true);
        assert_eq!(parsed.answer, "你好，我记住了。");
    }

    #[test]
    fn record_requires_model_note_draft() {
        assert!(parse_record_response(
            "",
            "记录一下，wboot -w update zboot.img",
            RequestUsage::default()
        )
        .is_err());
    }

    #[test]
    fn parses_list_notes_tool_call() {
        let tool = parse_tool_call(
            "{\"tool\":\"list_notes\",\"query\":\"\",\"limit\":5,\"noteDraft\":null}",
            "list notes",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::ListNotes { limit: 5 }));
    }

    #[test]
    fn parses_list_notes_tool_call_cjk() {
        let tool = parse_tool_call(
            "{\"tool\":\"list_notes\",\"query\":\"\",\"limit\":5,\"noteDraft\":null}",
            "资料库里有什么",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::ListNotes { limit: 5 }));
    }

    #[test]
    fn parses_search_notes_tool_call() {
        let tool = parse_tool_call(
            "{\"tool\":\"search_notes\",\"query\":\"mmc timeout\",\"limit\":6,\"noteDraft\":null}",
            "mmc超时",
        )
        .expect("tool");
        assert!(
            matches!(tool, AssistantToolCall::SearchNotes { query, limit } if query == "mmc timeout" && limit == 6)
        );
    }

    #[test]
    fn parses_read_file_tool_call() {
        let tool = parse_tool_call(
            "{\"tool\":\"read_file\",\"path\":\"C:\\\\\\\\Users\\\\\\\\test\\\\\\\\log.txt\",\"noteDraft\":null}",
            "看下日志",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::ReadFile { path } if path.contains("log.txt")));
    }

    #[test]
    fn parses_read_file_tool_call_with_unescaped_windows_path() {
        let tool = parse_tool_call(
            r#"{"tool":"read_file","query":"","path":"\\?\C:\Users\test\log.txt","limit":6,"noteDraft":null}"#,
            "read the file",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::ReadFile { path } if path.contains("log.txt")));
    }

    #[test]
    fn rejects_run_command_tool_call() {
        assert!(parse_tool_call(
            "{\"tool\":\"run_command\",\"command\":\"dir\",\"cwd\":\"\",\"noteDraft\":null}",
            "列出文件",
        )
        .is_err());
    }

    #[test]
    fn parses_none_tool_call() {
        let tool = parse_tool_call(
            "{\"tool\":\"none\",\"query\":\"\",\"limit\":0,\"noteDraft\":null}",
            "你好",
        )
        .expect("tool");
        assert!(matches!(tool, AssistantToolCall::None));
    }

    #[test]
    fn parse_tool_call_returns_err_for_unknown_tool() {
        assert!(parse_tool_call(
            "{\"tool\":\"fly_to_moon\",\"query\":\"\",\"limit\":0,\"noteDraft\":null}",
            "去月球",
        )
        .is_err());
    }

    #[test]
    fn parse_or_fallback_note_uses_heuristic_on_plain_text() {
        let draft = parse_or_fallback_note("这不是JSON，只是一段话", "帮我记录一下mmc超时的问题");
        assert!(!draft.body.is_empty());
    }

    #[test]
    fn parse_or_fallback_answer_extracts_citations() {
        let json = r#"{"answer":"参见笔记","citations":[{"noteId":"n1","title":"T","path":"/p.md","snippet":"s"}]}"#;
        let parsed = parse_or_fallback_answer(json, "问题", true);
        assert_eq!(parsed.answer, "参见笔记");
        assert_eq!(parsed.citations.len(), 1);
        assert_eq!(parsed.citations[0].note_id, "n1");
    }

    #[test]
    fn dedupe_terms_removes_duplicates_case_insensitive() {
        let result = dedupe_terms(vec![
            "Kernel".to_string(),
            "kernel".to_string(),
            "KERNEL".to_string(),
        ]);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn dedupe_terms_removes_empty_strings() {
        let result = dedupe_terms(vec!["a".to_string(), "".to_string(), "  ".to_string()]);
        assert_eq!(result, vec!["a"]);
    }

    #[test]
    fn normalize_draft_fills_empty_fields_from_heuristic() {
        let draft = StructuredNoteDraft {
            title: String::new(),
            body: "wboot -w update zboot.img 刷机命令".to_string(),
            ..Default::default()
        };
        let normalized = normalize_draft(draft);
        assert!(!normalized.title.is_empty());
    }

    #[test]
    fn resolve_context_window_uses_manual_override() {
        let settings = AppSettings {
            provider: ProviderConfig {
                context_window_tokens: Some(999_999),
                ..Default::default()
            },
            ..Default::default()
        };
        let (tokens, source) = resolve_context_window(&settings);
        assert_eq!(tokens, 999_999);
        assert_eq!(source, "manual_override");
    }

    #[test]
    fn resolve_context_window_recognizes_claude_models() {
        let settings = AppSettings {
            provider: ProviderConfig {
                model: "claude-3-5-sonnet-latest".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let (tokens, _) = resolve_context_window(&settings);
        assert_eq!(tokens, 200_000);
    }

    #[test]
    fn resolve_context_window_defaults_for_unknown_model() {
        let settings = AppSettings {
            provider: ProviderConfig {
                model: "unknown-model-xyz".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let (tokens, source) = resolve_context_window(&settings);
        assert_eq!(tokens, 128_000);
        assert_eq!(source, "heuristic_default");
    }

    #[test]
    fn is_retryable_detects_429_and_specific_5xx() {
        assert!(is_retryable_provider_error(429, ""));
        assert!(is_retryable_provider_error(500, ""));
        assert!(is_retryable_provider_error(502, ""));
        assert!(is_retryable_provider_error(503, ""));
        assert!(is_retryable_provider_error(504, ""));
        assert!(!is_retryable_provider_error(501, ""));
        assert!(!is_retryable_provider_error(400, ""));
        assert!(!is_retryable_provider_error(401, ""));
    }

    #[test]
    fn is_retryable_detects_500_as_transient() {
        assert!(is_retryable_provider_error(500, ""));
        assert!(is_retryable_provider_error(500, "Internal Server Error"));
        assert!(!is_retryable_provider_error(501, ""));
    }

    #[test]
    fn is_retryable_detects_rate_limit_in_detail() {
        assert!(is_retryable_provider_error(400, "rate limit exceeded"));
        assert!(is_retryable_provider_error(400, "Too Many Requests"));
        assert!(is_retryable_provider_error(400, "访问量过大"));
        assert!(!is_retryable_provider_error(400, "bad request"));
    }

    #[test]
    fn is_openai_reasoning_model_matches_exact_prefix() {
        assert!(is_openai_reasoning_model("o1"));
        assert!(is_openai_reasoning_model("o3"));
        assert!(is_openai_reasoning_model("o4"));
    }

    #[test]
    fn is_openai_reasoning_model_matches_with_suffix() {
        assert!(is_openai_reasoning_model("o1-mini"));
        assert!(is_openai_reasoning_model("o1-preview"));
        assert!(is_openai_reasoning_model("o3-mini"));
        assert!(is_openai_reasoning_model("o4-mini"));
    }

    #[test]
    fn is_openai_reasoning_model_rejects_false_positives() {
        assert!(!is_openai_reasoning_model("phi-1"));
        assert!(!is_openai_reasoning_model("co1der"));
        assert!(!is_openai_reasoning_model("pro1"));
        assert!(!is_openai_reasoning_model("some-o3thing"));
        assert!(!is_openai_reasoning_model("mo4del"));
    }

    #[test]
    fn is_openai_reasoning_model_handles_namespaced_names() {
        assert!(is_openai_reasoning_model("openai/o1-mini"));
        assert!(is_openai_reasoning_model("openai/o1-preview"));
        assert!(is_openai_reasoning_model("together/o3-mini"));
        assert!(is_openai_reasoning_model("custom-provider/o4-mini"));
        assert!(is_openai_reasoning_model("org/o1"));
        assert!(!is_openai_reasoning_model("openai/gpt-4o"));
        assert!(!is_openai_reasoning_model("together/phi-1"));
        assert!(!is_openai_reasoning_model("provider/pro1"));
    }

    #[test]
    fn resolve_max_output_tokens_reasoning_models() {
        assert_eq!(resolve_max_output_tokens("o1-mini", None), 32768);
        assert_eq!(resolve_max_output_tokens("o3-mini", None), 32768);
        assert_eq!(resolve_max_output_tokens("o4-mini", None), 32768);
        assert_eq!(resolve_max_output_tokens("openai/o1-mini", None), 32768);
        assert_eq!(resolve_max_output_tokens("o1-mini", Some(16384)), 16384);
        assert_eq!(resolve_max_output_tokens("gpt-4o", None), 16384);
        assert_eq!(resolve_max_output_tokens("unknown-model", None), 8192);
    }

    #[test]
    fn openai_reasoning_request_uses_max_completion_tokens_and_omits_temperature() {
        let payload = OpenAiReasoningRequest {
            model: "o3-mini",
            max_completion_tokens: 16384,
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: OpenAiContent::Text("hello".to_string()),
            }],
            stream: false,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["model"], "o3-mini");
        assert_eq!(json["max_completion_tokens"], 16384);
        assert!(json.get("max_tokens").is_none());
        assert!(json.get("temperature").is_none());
    }

    #[test]
    fn openai_standard_request_uses_max_tokens_and_temperature() {
        let payload = OpenAiRequest {
            model: "gpt-4o",
            max_tokens: 4096,
            temperature: 0.5,
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: OpenAiContent::Text("hello".to_string()),
            }],
            stream: false,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["model"], "gpt-4o");
        assert_eq!(json["max_tokens"], 4096);
        assert_eq!(json["temperature"], 0.5);
        assert!(json.get("max_completion_tokens").is_none());
    }

    // ── validate_base_url ─────────────────────────────────────────────

    #[tokio::test]
    async fn validate_base_url_accepts_https() {
        assert!(
            super::client::validate_base_url("https://api.anthropic.com/v1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn validate_base_url_rejects_empty() {
        assert!(super::client::validate_base_url("").await.is_err());
        assert!(super::client::validate_base_url("   ").await.is_err());
    }

    #[tokio::test]
    async fn validate_base_url_rejects_invalid_url() {
        assert!(super::client::validate_base_url("not a url").await.is_err());
    }

    #[tokio::test]
    async fn validate_base_url_rejects_non_http_scheme() {
        assert!(super::client::validate_base_url("ftp://example.com")
            .await
            .is_err());
        assert!(super::client::validate_base_url("file:///etc/passwd")
            .await
            .is_err());
    }

    #[test]
    fn validate_base_url_localhost_env_guard() {
        use super::client::validate_base_url;

        // Without env var → reject localhost
        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::remove_var("VAULTPILOT_ALLOW_LOCAL_ENDPOINT");
            let rt = tokio::runtime::Runtime::new().unwrap();
            assert!(rt
                .block_on(validate_base_url("http://localhost:8080"))
                .is_err());
        }

        // With env var → allow localhost
        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("VAULTPILOT_ALLOW_LOCAL_ENDPOINT", "1");
            let rt = tokio::runtime::Runtime::new().unwrap();
            assert!(rt
                .block_on(validate_base_url("http://localhost:8080"))
                .is_ok());
        }

        // Cleanup
        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::remove_var("VAULTPILOT_ALLOW_LOCAL_ENDPOINT");
        }
    }

    #[test]
    fn validate_base_url_rejects_private_ip() {
        use super::client::validate_base_url;

        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("VAULTPILOT_ALLOW_LOCAL_ENDPOINT");
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert!(rt
            .block_on(validate_base_url("http://192.168.1.1/api"))
            .is_err());
        assert!(rt
            .block_on(validate_base_url("http://10.0.0.1/api"))
            .is_err());
        assert!(rt
            .block_on(validate_base_url("http://172.16.0.1/api"))
            .is_err());
        assert!(rt
            .block_on(validate_base_url("http://127.0.0.1/api"))
            .is_err());
    }

    // ── is_private_ip ──────────────────────────────────────────────────

    #[test]
    fn is_private_ip_detects_loopback() {
        assert!(is_private_ip("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("::1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_detects_rfc1918() {
        assert!(is_private_ip("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_allows_public_ip() {
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_detects_ipv6_unique_local() {
        assert!(is_private_ip("fd00::1".parse().unwrap()));
        assert!(is_private_ip("fc00::1".parse().unwrap()));
        assert!(is_private_ip("fd12:3456:789a::1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_detects_ipv4_mapped_private() {
        assert!(is_private_ip("::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("::ffff:192.168.1.1".parse().unwrap()));
        assert!(is_private_ip("::ffff:172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_allows_ipv4_mapped_public() {
        assert!(!is_private_ip("::ffff:8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip("::ffff:1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_allows_global_ipv6() {
        assert!(!is_private_ip("2001:db8::1".parse().unwrap()));
        assert!(!is_private_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn generate_programmatic_snippet_handles_cjk() {
        let body = "这是一个测试文档，包含中文字符和English混合内容。\n\n第二段包含更多中文。";
        let snippet = generate_programmatic_snippet(body, "测试");
        assert!(snippet.contains("=="));
        assert!(snippet.contains("测试"));
    }

    #[test]
    fn generate_programmatic_snippet_handles_multibyte_unicode() {
        let body = "Straße und Überprüfung sind wichtig.";
        let snippet = generate_programmatic_snippet(body, "Straße");
        assert!(!snippet.is_empty());
    }
}
