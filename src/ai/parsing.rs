//! Response parsing, JSON extraction, fallback generation.

use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::collections::HashSet;

use super::client::RequestUsage;
use crate::models::{AnswerCitation, NoteDocument, StructuredNoteDraft};

pub struct ChatAnswerResult {
    pub answer: String,
    pub citations: Vec<AnswerCitation>,
    pub usage: RequestUsage,
}

pub struct RecordInteractionResult {
    pub reply: String,
    pub note_draft: StructuredNoteDraft,
    pub usage: RequestUsage,
}

pub struct ToolSelectionResult {
    pub tool_call: AssistantToolCall,
    pub usage: RequestUsage,
}

#[derive(Debug, Clone, Default)]

pub enum AssistantToolCall {
    #[default]
    None,
    SearchNotes {
        query: String,
        limit: usize,
    },
    ListNotes {
        limit: usize,
    },
    ListDirectory {
        path: String,
    },
    ReadFile {
        path: String,
    },
    SaveNote {
        draft: Box<StructuredNoteDraft>,
        note_id: String,
    },
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct AnthropicResponse {
    #[serde(default)]
    pub(super) content: Vec<AnthropicContentBlock>,
    #[serde(default)]
    pub(super) usage: AnthropicUsage,
    pub(super) error: Option<AnthropicApiError>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct AnthropicContentBlock {
    #[serde(default, rename = "type")]
    pub(super) kind: String,
    pub(super) text: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct AnthropicApiError {
    #[serde(default)]
    pub(super) message: String,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct AnthropicUsage {
    #[serde(default)]
    pub(super) input_tokens: usize,
    #[serde(default)]
    pub(super) output_tokens: usize,
}

// ---------------------------------------------------------------------------
// OpenAI-compatible response structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub(super) struct OpenAiResponse {
    #[serde(default)]
    pub(super) choices: Vec<OpenAiChoice>,
    #[serde(default)]
    pub(super) usage: OpenAiUsage,
    pub(super) error: Option<OpenAiApiError>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct OpenAiChoice {
    #[serde(default)]
    pub(super) message: OpenAiChoiceMessage,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct OpenAiChoiceMessage {
    #[serde(default)]
    pub(super) content: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct OpenAiApiError {
    #[serde(default)]
    pub(super) message: String,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct OpenAiUsage {
    #[serde(default)]
    pub(super) prompt_tokens: usize,
    #[serde(default)]
    pub(super) completion_tokens: usize,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct IngestResponse {
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    board: String,
    #[serde(default)]
    kernel: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct AskResponse {
    #[serde(default)]
    pub(super) answer: String,
    #[serde(default)]
    pub(super) citations: Vec<AnswerCitation>,
    #[serde(default)]
    pub(super) note_draft: Option<StructuredNoteDraft>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecordResponse {
    #[serde(default)]
    pub(super) reply: String,
    #[serde(default)]
    pub(super) note_draft: Option<StructuredNoteDraft>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct ToolCallResponse {
    #[serde(default)]
    tool: String,
    #[serde(default)]
    query: String,
    #[serde(default)]
    path: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    note_draft: Option<StructuredNoteDraft>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct CompressionResponse {
    #[serde(default)]
    pub(super) summary: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct NoteSelectionResponse {
    #[serde(default)]
    pub(super) note_ids: Vec<String>,
}

pub(super) fn default_limit() -> usize {
    6
}

pub(super) fn parse_or_fallback_note(text: &str, raw_input: &str) -> StructuredNoteDraft {
    let parsed = extract_json(text)
        .ok()
        .and_then(|json| serde_json::from_str::<IngestResponse>(&json).ok());

    if let Some(parsed) = parsed {
        return StructuredNoteDraft {
            title: fallback_title(&parsed.title, raw_input),
            summary: fallback_summary(&parsed.summary, raw_input),
            tags: dedupe_terms(parsed.tags),
            keywords: dedupe_terms(parsed.keywords),
            platform: parsed.platform.trim().to_string(),
            board: parsed.board.trim().to_string(),
            kernel: parsed.kernel.trim().to_string(),
            status: if parsed.status.trim().is_empty() {
                "已记录".to_string()
            } else {
                parsed.status.trim().to_string()
            },
            source: "captured".to_string(),
            body: fallback_body(&parsed.body, raw_input),
        };
    }

    heuristic_note_from_input(raw_input)
}

pub(super) fn parse_or_fallback_answer(
    text: &str,
    question: &str,
    no_context: bool,
) -> AskResponse {
    if let Ok(json) = extract_json(text) {
        if let Ok(parsed) = serde_json::from_str::<AskResponse>(&json) {
            let answer = parsed.answer.trim().to_string();
            return AskResponse {
                answer: if answer.is_empty() {
                    fallback_answer(question, no_context)
                } else {
                    answer
                },
                citations: parsed.citations,
                note_draft: parsed.note_draft,
            };
        }
    }

    AskResponse {
        answer: if text.trim().is_empty() {
            fallback_answer(question, no_context)
        } else {
            text.trim().to_string()
        },
        citations: Vec::new(),
        note_draft: None,
    }
}

/// Extract a programmatic snippet from `body` that contains the best matching
/// paragraph for the given `query`.  Returns the first paragraph that contains
/// at least one query term, with `==highlight==` markers around each match.
/// Falls back to the first 280 characters if no paragraph matches.
pub(super) fn generate_programmatic_snippet(body: &str, query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    if terms.is_empty() {
        return truncate(body, 280).to_string();
    }

    // Split body into paragraphs (separated by blank lines).
    let paragraphs: Vec<&str> = body
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    // Find the first paragraph containing at least one query term.
    let best = paragraphs
        .iter()
        .find(|p| {
            let lower = p.to_lowercase();
            terms.iter().any(|t| lower.contains(t.as_str()))
        })
        .copied()
        .unwrap_or_else(|| {
            // No paragraph matched; fall back to the first non-heading paragraph
            // or the first paragraph overall.
            paragraphs
                .iter()
                .find(|p| !p.starts_with('#'))
                .copied()
                .or_else(|| paragraphs.first().copied())
                .unwrap_or(body)
        });

    // Highlight each term in the chosen paragraph (case-insensitive).
    // We collect all highlight ranges first, merge overlapping ones,
    // then apply markers once to avoid corruption from sequential passes.
    let snippet_chars: Vec<char> = best.chars().collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for term in &terms {
        let term_len = term.chars().count();
        if term_len == 0 {
            continue;
        }
        let mut i = 0;
        while i <= snippet_chars.len().saturating_sub(term_len) {
            let candidate_lower: String = snippet_chars[i..i + term_len]
                .iter()
                .collect::<String>()
                .to_lowercase();
            if candidate_lower.as_str() == term.as_str() {
                ranges.push((i, i + term_len));
                i += term_len; // skip past this match to avoid overlapping highlights
            } else {
                i += 1;
            }
        }
    }
    // Merge overlapping ranges
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    // Apply highlight markers once
    let mut snippet = String::with_capacity(best.len() + merged.len() * 4);
    let mut prev_end = 0;
    for (start, end) in &merged {
        // Append text before this highlight
        for c in &snippet_chars[prev_end..*start] {
            snippet.push(*c);
        }
        // Append highlighted text
        snippet.push_str("==");
        for c in &snippet_chars[*start..*end] {
            snippet.push(*c);
        }
        snippet.push_str("==");
        prev_end = *end;
    }
    // Append remaining characters after last highlight
    for c in &snippet_chars[prev_end..] {
        snippet.push(*c);
    }

    // Truncate if too long.
    if snippet.len() > 500 {
        let mut end = 498;
        while end > 0 && !snippet.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &snippet[..end])
    } else {
        snippet
    }
}

/// Enrich AI-generated citations with programmatic snippets from FTS5 data.
/// For each citation, if a matching note document has an FTS5 search_snippet,
/// use that; otherwise generate a programmatic snippet from the body.
pub(super) fn enrich_citations(
    citations: Vec<AnswerCitation>,
    docs: &[NoteDocument],
) -> Vec<AnswerCitation> {
    if citations.is_empty() || docs.is_empty() {
        return citations;
    }

    let doc_map: std::collections::HashMap<&str, &NoteDocument> =
        docs.iter().map(|d| (d.meta.id.as_str(), d)).collect();

    // Compute a simple relevance score based on doc rank (0.0–1.0).
    // Higher-ranked docs (earlier in the list) get higher scores.
    let total_docs = docs.len();
    let rank_map: std::collections::HashMap<&str, f64> = docs
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let score = if total_docs > 1 {
                1.0 - (i as f64 / total_docs as f64) * 0.5 // 1.0 → 0.5 range
            } else {
                1.0
            };
            (d.meta.id.as_str(), score)
        })
        .collect();

    citations
        .into_iter()
        .map(|mut citation| {
            if let Some(doc) = doc_map.get(citation.note_id.as_str()) {
                // Populate score from rank-based calculation (#1704)
                if citation.score.is_none() {
                    citation.score = rank_map.get(citation.note_id.as_str()).copied();
                }
                if let Some(ref fts_snippet) = doc.search_snippet {
                    if !fts_snippet.trim().is_empty() && fts_snippet.contains("==") {
                        citation.snippet = fts_snippet.clone();
                        return citation;
                    }
                }
                // No FTS5 snippet; generate one programmatically if the
                // AI-generated snippet is empty or very short.
                if citation.snippet.trim().len() < 20 {
                    citation.snippet = generate_programmatic_snippet(&doc.body, &citation.title);
                }
            }
            citation
        })
        .collect()
}

pub(super) fn parse_record_response(
    text: &str,
    raw_input: &str,
    usage: RequestUsage,
) -> Result<RecordInteractionResult> {
    if let Ok(json) = extract_json(text) {
        if let Ok(parsed) = serde_json::from_str::<RecordResponse>(&json) {
            if let Some(note_draft) = parsed.note_draft {
                let draft = normalize_draft(note_draft);
                let reply = if parsed.reply.trim().is_empty() {
                    fallback_record_reply(&draft.title)
                } else {
                    parsed.reply.trim().to_string()
                };
                return Ok(RecordInteractionResult {
                    reply,
                    note_draft: draft,
                    usage,
                });
            }
        }
    }

    Err(anyhow!(
        "model did not return a valid note draft for record request: {}",
        crate::sanitize_error(&truncate(raw_input, 80))
    ))
}

pub(super) fn parse_tool_call(text: &str, question: &str) -> Result<AssistantToolCall> {
    let parsed = extract_json(text)
        .ok()
        .and_then(|json| parse_tool_call_response(&json))
        .ok_or_else(|| anyhow!("model did not return a valid tool call"))?;

    let limit = parsed.limit.clamp(3, 8);

    match parsed.tool.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(AssistantToolCall::None),
        "search_notes" => Ok(AssistantToolCall::SearchNotes {
            query: if parsed.query.trim().is_empty() {
                question.trim().to_string()
            } else {
                parsed.query.trim().to_string()
            },
            limit,
        }),
        "list_notes" => Ok(AssistantToolCall::ListNotes { limit }),
        "list_directory" => Ok(AssistantToolCall::ListDirectory {
            path: parsed.path.trim().to_string(),
        }),
        "read_file" => Ok(AssistantToolCall::ReadFile {
            path: parsed.path.trim().to_string(),
        }),
        "save_note" => {
            let draft = parsed
                .note_draft
                .map(normalize_draft)
                .ok_or_else(|| anyhow!("save_note was selected but noteDraft is missing"))?;
            Ok(AssistantToolCall::SaveNote {
                draft: Box::new(draft),
                note_id: uuid::Uuid::new_v4().to_string(),
            })
        }
        other => Err(anyhow!(
            "unknown tool selected by model: {}",
            crate::sanitize_error(other)
        )),
    }
}

pub(super) fn parse_tool_call_response(json: &str) -> Option<ToolCallResponse> {
    serde_json::from_str::<ToolCallResponse>(json)
        .ok()
        .or_else(|| {
            let repaired = repair_json_string_escapes(json)?;
            serde_json::from_str::<ToolCallResponse>(&repaired).ok()
        })
}

#[allow(clippy::while_let_on_iterator)]
pub(super) fn repair_json_string_escapes(input: &str) -> Option<String> {
    let mut repaired = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaping = false;

    while let Some(ch) = chars.next() {
        if in_string {
            if escaping {
                if matches!(ch, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u') {
                    repaired.push(ch);
                } else {
                    repaired.push('\\');
                    repaired.push(ch);
                }
                escaping = false;
                continue;
            }

            match ch {
                '\\' => {
                    repaired.push('\\');
                    escaping = true;
                }
                '"' => {
                    repaired.push('"');
                    in_string = false;
                }
                '\n' => repaired.push_str("\\n"),
                '\r' => repaired.push_str("\\r"),
                '\t' => repaired.push_str("\\t"),
                _ => repaired.push(ch),
            }
        } else {
            repaired.push(ch);
            if ch == '"' {
                in_string = true;
            }
        }
    }

    if escaping {
        repaired.push('\\');
    }

    Some(repaired)
}

pub(super) fn normalize_draft(draft: StructuredNoteDraft) -> StructuredNoteDraft {
    let fallback = heuristic_note_from_input(&draft.body);
    StructuredNoteDraft {
        title: if draft.title.trim().is_empty() {
            fallback.title
        } else {
            draft.title.trim().to_string()
        },
        summary: if draft.summary.trim().is_empty() {
            fallback.summary
        } else {
            draft.summary.trim().to_string()
        },
        tags: dedupe_terms(draft.tags),
        keywords: dedupe_terms(draft.keywords),
        platform: draft.platform.trim().to_string(),
        board: draft.board.trim().to_string(),
        kernel: draft.kernel.trim().to_string(),
        status: if draft.status.trim().is_empty() {
            "已记录".to_string()
        } else {
            draft.status.trim().to_string()
        },
        source: if draft.source.trim().is_empty() {
            "captured".to_string()
        } else {
            draft.source.trim().to_string()
        },
        body: if draft.body.trim().is_empty() {
            fallback.body
        } else {
            draft.body.trim().to_string()
        },
    }
}

pub(super) fn heuristic_note_from_input(raw_input: &str) -> StructuredNoteDraft {
    let compact = raw_input.trim();
    let (heuristic_title, heuristic_tags) =
        crate::search_rules::SearchRules::global().evaluate_heuristic(compact);

    let title = if let Some(t) = heuristic_title {
        t
    } else if let Some(first_line) = compact.lines().find(|line| !line.trim().is_empty()) {
        truncate(first_line.trim(), 40)
    } else {
        "临时记录".to_string()
    };

    let keywords = extract_command_keywords(compact);
    let mut tags = vec!["record".to_string()];
    tags.extend(heuristic_tags);

    StructuredNoteDraft {
        title,
        summary: truncate(compact, 120),
        tags: dedupe_terms(tags),
        keywords,
        platform: String::new(),
        board: String::new(),
        kernel: String::new(),
        status: "已记录".to_string(),
        source: "captured".to_string(),
        body: format!(
            "## 摘要\n\n{}\n\n## 背景/上下文\n\n待确认\n\n## 关键信息\n\n{}\n\n## 操作步骤/命令\n\n```\n{}\n```\n\n## 结果/结论\n\n待确认\n\n## 待确认事项\n\n待确认\n\n## 关键词\n\n{}",
            compact,
            compact,
            compact,
            extract_command_keywords(compact).join(", ")
        ),
    }
}

pub(super) fn extract_command_keywords(raw_input: &str) -> Vec<String> {
    dedupe_terms(
        raw_input
            .split_whitespace()
            .map(|part| {
                part.trim_matches(|ch: char| ",.;:()[]{}'\"".contains(ch))
                    .to_string()
            })
            .filter(|part| !part.is_empty())
            .filter(|part| part.len() > 1)
            .collect(),
    )
}

pub(super) fn fallback_title(title: &str, raw_input: &str) -> String {
    if title.trim().is_empty() {
        heuristic_note_from_input(raw_input).title
    } else {
        title.trim().to_string()
    }
}

pub(super) fn fallback_summary(summary: &str, raw_input: &str) -> String {
    if summary.trim().is_empty() {
        truncate(raw_input.trim(), 120)
    } else {
        summary.trim().to_string()
    }
}

pub(super) fn fallback_body(body: &str, raw_input: &str) -> String {
    if body.trim().is_empty() {
        heuristic_note_from_input(raw_input).body
    } else {
        body.trim().to_string()
    }
}

pub(super) fn fallback_answer(question: &str, no_context: bool) -> String {
    if no_context {
        format!(
            "我先直接回答这个问题：{}。这次没有检索到可用的本地笔记，所以这是基于通用模型理解给出的回答。",
            question
        )
    } else {
        "我已经拿到了知识库结果，但这次模型没有按 JSON 返回，所以我先把可读文本直接展示给你。"
            .to_string()
    }
}

pub(super) fn fallback_record_reply(title: &str) -> String {
    format!(
        "我已经理解这条内容，并按“{}”这个主题准备写入知识库。",
        title
    )
}

pub(super) fn dedupe_terms(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_lowercase()))
        .collect()
}

pub(super) fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn extract_json(text: &str) -> Result<String> {
    let trimmed = text.trim();
    // Try extracting a well-delimited, validated JSON block first.
    // This prevents returning strings like `{"a":1} prose {"b":2}` that
    // bracket-match but fail serde_json (issue #601).
    if let Some(result) = extract_json_block(trimmed, '{', '}') {
        return Ok(result);
    }
    if let Some(result) = extract_json_block(trimmed, '[', ']') {
        return Ok(result);
    }
    // Fallback: if the whole string bracket-matches, return it directly.
    // This handles inputs that are valid JSON but have unusual structure
    // that extract_json_block's depth-tracking doesn't capture.
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Ok(trimmed.to_string());
    }
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return Ok(trimmed.to_string());
    }
    Err(anyhow!("AI response does not contain JSON"))
}

pub(super) fn extract_json_block(text: &str, open: char, close: char) -> Option<String> {
    // Try every occurrence of the opening character, not just the first.
    // Prose before the real JSON may contain braces/brackets that cause
    // a false match when we only look at the first occurrence (issue #434).
    for (start, _) in text.match_indices(open) {
        let mut depth = 0;
        let mut in_string = false;
        let mut backslash_count = 0usize;
        for (i, c) in text[start..].char_indices() {
            if in_string {
                if c == '\\' {
                    backslash_count += 1;
                } else {
                    if c == '"' && backslash_count.is_multiple_of(2) {
                        in_string = false;
                    }
                    backslash_count = 0;
                }
                continue;
            }
            match c {
                '"' => in_string = true,
                c if c == open => depth += 1,
                c if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        // Validate that this substring is parseable JSON
                        let candidate = &text[start..=start + i];
                        if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                            return Some(candidate.to_string());
                        }
                        // Not valid JSON — continue searching from next open char
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::client::RequestUsage;
    use crate::models::{AnswerCitation, NoteDocument, NoteMeta, StructuredNoteDraft};

    fn sample_usage() -> RequestUsage {
        RequestUsage {
            input_tokens: Some(100),
            output_tokens: Some(50),
        }
    }

    // --- default_limit ---
    #[test]
    fn default_limit_returns_6() {
        assert_eq!(default_limit(), 6);
    }

    // --- truncate ---
    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_cut() {
        assert_eq!(truncate("hello world", 5), "hello");
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate("", 10), "");
    }

    #[test]
    fn truncate_exact_length() {
        assert_eq!(truncate("abc", 3), "abc");
    }

    #[test]
    fn truncate_cjk_characters() {
        assert_eq!(truncate("你好世界测试", 3), "你好世");
    }

    // --- dedupe_terms ---
    #[test]
    fn dedupe_terms_removes_duplicates() {
        let result = dedupe_terms(vec!["a".into(), "A".into(), "b".into()]);
        assert_eq!(result, vec!["a", "b"]);
    }

    #[test]
    fn dedupe_terms_trims_whitespace() {
        let result = dedupe_terms(vec![" a ".into(), "a".into()]);
        assert_eq!(result, vec!["a"]);
    }

    #[test]
    fn dedupe_terms_filters_empty() {
        let result = dedupe_terms(vec!["".into(), "  ".into(), "a".into()]);
        assert_eq!(result, vec!["a"]);
    }

    #[test]
    fn dedupe_terms_empty_input() {
        let result = dedupe_terms(vec![]);
        assert!(result.is_empty());
    }

    // --- extract_json ---
    #[test]
    fn extract_json_from_pure_json_object() {
        let result = extract_json(r#"{"key": "value"}"#).unwrap();
        assert_eq!(result, r#"{"key": "value"}"#);
    }

    #[test]
    fn extract_json_from_json_array() {
        let result = extract_json(r#"[1, 2, 3]"#).unwrap();
        assert_eq!(result, r#"[1, 2, 3]"#);
    }

    #[test]
    fn extract_json_from_surrounding_prose() {
        let text = r#"Here is the result: {"answer": "42"} and that's it."#;
        let result = extract_json(text).unwrap();
        assert_eq!(result, r#"{"answer": "42"}"#);
    }

    #[test]
    fn extract_json_no_json_returns_err() {
        assert!(extract_json("no json here").is_err());
    }

    #[test]
    fn extract_json_empty_string_returns_err() {
        assert!(extract_json("").is_err());
    }

    // --- extract_json_block ---
    #[test]
    fn extract_json_block_simple_object() {
        let result = extract_json_block(r#"{"a":1}"#, '{', '}').unwrap();
        assert_eq!(result, r#"{"a":1}"#);
    }

    #[test]
    fn extract_json_block_nested_object() {
        let text = r#"{"a": {"b": 2}}"#;
        let result = extract_json_block(text, '{', '}').unwrap();
        assert_eq!(result, text);
    }

    #[test]
    fn extract_json_block_skips_prose_before() {
        let text = r#"Some text {"a":1} more"#;
        let result = extract_json_block(text, '{', '}').unwrap();
        assert_eq!(result, r#"{"a":1}"#);
    }

    #[test]
    fn extract_json_block_no_match() {
        assert!(extract_json_block("no braces here", '{', '}').is_none());
    }

    #[test]
    fn extract_json_block_array() {
        let result = extract_json_block(r#"text [1,2,3] more"#, '[', ']').unwrap();
        assert_eq!(result, "[1,2,3]");
    }

    #[test]
    fn extract_json_block_invalid_json_braces_skipped() {
        // A brace that doesn't form valid JSON is skipped
        assert!(extract_json_block("{not json}", '{', '}').is_none());
    }

    // --- repair_json_string_escapes ---
    #[test]
    fn repair_json_string_escapes_valid_json() {
        let input = r#"{"tool":"search_notes"}"#;
        let result = repair_json_string_escapes(input).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn repair_json_string_escapes_with_newlines() {
        let input = r#"{"text":"line1\nline2"}"#;
        let result = repair_json_string_escapes(input).unwrap();
        assert_eq!(result, input);
    }

    // --- parse_tool_call_response ---
    #[test]
    fn parse_tool_call_response_valid() {
        let json = r#"{"tool":"search_notes","query":"test","limit":5}"#;
        let result = parse_tool_call_response(json).unwrap();
        assert_eq!(result.tool, "search_notes");
        assert_eq!(result.query, "test");
        assert_eq!(result.limit, 5);
    }

    #[test]
    fn parse_tool_call_response_invalid() {
        assert!(parse_tool_call_response("not json").is_none());
    }

    #[test]
    fn parse_tool_call_response_uses_default_limit() {
        let json = r#"{"tool":"none"}"#;
        let result = parse_tool_call_response(json).unwrap();
        assert_eq!(result.limit, 6); // default_limit()
    }

    // --- parse_tool_call ---
    #[test]
    fn parse_tool_call_none() {
        let text = r#"{"tool":"none","query":"","limit":5}"#;
        let result = parse_tool_call(text, "some question").unwrap();
        assert!(matches!(result, AssistantToolCall::None));
    }

    #[test]
    fn parse_tool_call_search_notes() {
        let text = r#"{"tool":"search_notes","query":"rust tips","limit":5}"#;
        let result = parse_tool_call(text, "fallback").unwrap();
        match result {
            AssistantToolCall::SearchNotes { query, limit } => {
                assert_eq!(query, "rust tips");
                assert_eq!(limit, 5);
            }
            _ => panic!("Expected SearchNotes"),
        }
    }

    #[test]
    fn parse_tool_call_search_notes_empty_query_uses_question() {
        let text = r#"{"tool":"search_notes","query":"","limit":5}"#;
        let result = parse_tool_call(text, "my question").unwrap();
        match result {
            AssistantToolCall::SearchNotes { query, .. } => {
                assert_eq!(query, "my question");
            }
            _ => panic!("Expected SearchNotes"),
        }
    }

    #[test]
    fn parse_tool_call_list_notes() {
        let text = r#"{"tool":"list_notes","limit":4}"#;
        let result = parse_tool_call(text, "q").unwrap();
        assert!(matches!(result, AssistantToolCall::ListNotes { limit: 4 }));
    }

    #[test]
    fn parse_tool_call_list_directory() {
        let text = r#"{"tool":"list_directory","path":"/tmp","limit":5}"#;
        let result = parse_tool_call(text, "q").unwrap();
        match result {
            AssistantToolCall::ListDirectory { path } => assert_eq!(path, "/tmp"),
            _ => panic!("Expected ListDirectory"),
        }
    }

    #[test]
    fn parse_tool_call_read_file() {
        let text = r#"{"tool":"read_file","path":"/tmp/f.txt","limit":5}"#;
        let result = parse_tool_call(text, "q").unwrap();
        match result {
            AssistantToolCall::ReadFile { path } => assert_eq!(path, "/tmp/f.txt"),
            _ => panic!("Expected ReadFile"),
        }
    }

    #[test]
    fn parse_tool_call_save_note() {
        let text = r#"{"tool":"save_note","limit":5,"noteDraft":{"title":"T","summary":"S","tags":[],"keywords":[],"platform":"","board":"","kernel":"","status":"","source":"captured","body":"B"}}"#;
        let result = parse_tool_call(text, "q").unwrap();
        match result {
            AssistantToolCall::SaveNote { draft, .. } => {
                assert_eq!(draft.title, "T");
            }
            _ => panic!("Expected SaveNote"),
        }
    }

    #[test]
    fn parse_tool_call_save_note_missing_draft_returns_err() {
        let text = r#"{"tool":"save_note","limit":5}"#;
        assert!(parse_tool_call(text, "q").is_err());
    }

    #[test]
    fn parse_tool_call_unknown_tool_returns_err() {
        let text = r#"{"tool":"unknown_tool","limit":5}"#;
        assert!(parse_tool_call(text, "q").is_err());
    }

    #[test]
    fn parse_tool_call_limit_clamped() {
        let text = r#"{"tool":"list_notes","limit":100}"#;
        let result = parse_tool_call(text, "q").unwrap();
        match result {
            AssistantToolCall::ListNotes { limit } => assert_eq!(limit, 8), // clamped to max 8
            _ => panic!(),
        }
    }

    #[test]
    fn parse_tool_call_limit_clamped_min() {
        let text = r#"{"tool":"list_notes","limit":0}"#;
        let result = parse_tool_call(text, "q").unwrap();
        match result {
            AssistantToolCall::ListNotes { limit } => assert_eq!(limit, 3), // clamped to min 3
            _ => panic!(),
        }
    }

    // --- parse_or_fallback_note ---
    #[test]
    fn parse_or_fallback_note_valid_json() {
        let text = r#"{"title":"My Note","summary":"A summary","tags":["rust"],"keywords":["code"],"platform":"Linux","board":"x86","kernel":"6.1","status":"done","body":"content here"}"#;
        let result = parse_or_fallback_note(text, "raw input");
        assert_eq!(result.title, "My Note");
        assert_eq!(result.summary, "A summary");
        assert_eq!(result.tags, vec!["rust"]);
        assert_eq!(result.source, "captured");
    }

    #[test]
    fn parse_or_fallback_note_empty_title_uses_fallback() {
        let text = r#"{"title":"","summary":"S","tags":[],"keywords":[],"platform":"","board":"","kernel":"","status":"","body":"B"}"#;
        let result = parse_or_fallback_note(text, "raw input for title");
        assert!(!result.title.is_empty());
    }

    #[test]
    fn parse_or_fallback_note_invalid_json_uses_heuristic() {
        let result = parse_or_fallback_note("not json at all", "some raw input");
        assert!(!result.title.is_empty());
        assert_eq!(result.source, "captured");
    }

    #[test]
    fn parse_or_fallback_note_default_status() {
        let text = r#"{"title":"T","summary":"S","tags":[],"keywords":[],"platform":"","board":"","kernel":"","status":"","body":"B"}"#;
        let result = parse_or_fallback_note(text, "raw");
        assert_eq!(result.status, "已记录");
    }

    // --- parse_or_fallback_answer ---
    #[test]
    fn parse_or_fallback_answer_valid_json() {
        let text = r#"{"answer":"The answer is 42","citations":[]}"#;
        let result = parse_or_fallback_answer(text, "what?", false);
        assert_eq!(result.answer, "The answer is 42");
    }

    #[test]
    fn parse_or_fallback_answer_empty_answer_uses_fallback() {
        let text = r#"{"answer":"  ","citations":[]}"#;
        let result = parse_or_fallback_answer(text, "what?", false);
        assert!(!result.answer.is_empty());
    }

    #[test]
    fn parse_or_fallback_answer_no_json_uses_text() {
        let result = parse_or_fallback_answer("plain text answer", "q", false);
        assert_eq!(result.answer, "plain text answer");
    }

    #[test]
    fn parse_or_fallback_answer_empty_text_no_context() {
        let result = parse_or_fallback_answer("", "what is X?", true);
        assert!(result.answer.contains("what is X?"));
    }

    #[test]
    fn parse_or_fallback_answer_empty_text_with_context() {
        let result = parse_or_fallback_answer("", "q", false);
        assert!(result.answer.contains("知识库"));
    }

    // --- fallback_title ---
    #[test]
    fn fallback_title_non_empty() {
        assert_eq!(fallback_title("My Title", "raw"), "My Title");
    }

    #[test]
    fn fallback_title_empty_uses_heuristic() {
        let result = fallback_title("", "some raw input");
        assert!(!result.is_empty());
    }

    // --- fallback_summary ---
    #[test]
    fn fallback_summary_non_empty() {
        assert_eq!(fallback_summary("My Summary", "raw"), "My Summary");
    }

    #[test]
    fn fallback_summary_empty_uses_truncated_input() {
        let long_input = "a".repeat(200);
        let result = fallback_summary("", &long_input);
        assert!(result.len() <= 120);
    }

    // --- fallback_body ---
    #[test]
    fn fallback_body_non_empty() {
        assert_eq!(fallback_body("content", "raw"), "content");
    }

    #[test]
    fn fallback_body_empty_uses_heuristic() {
        let result = fallback_body("", "some input");
        assert!(!result.is_empty());
    }

    // --- fallback_answer ---
    #[test]
    fn fallback_answer_no_context() {
        let result = fallback_answer("what?", true);
        assert!(result.contains("what?"));
        assert!(result.contains("通用模型"));
    }

    #[test]
    fn fallback_answer_with_context() {
        let result = fallback_answer("q", false);
        assert!(result.contains("知识库"));
    }

    // --- fallback_record_reply ---
    #[test]
    fn fallback_record_reply_contains_title() {
        let result = fallback_record_reply("My Title");
        assert!(result.contains("My Title"));
    }

    // --- extract_command_keywords ---
    #[test]
    fn extract_command_keywords_basic() {
        let result = extract_command_keywords("hello world test");
        assert_eq!(result, vec!["hello", "world", "test"]);
    }

    #[test]
    fn extract_command_keywords_filters_single_char() {
        let result = extract_command_keywords("a b cd");
        assert_eq!(result, vec!["cd"]);
    }

    #[test]
    fn extract_command_keywords_strips_punctuation() {
        let result = extract_command_keywords("hello, world!");
        assert_eq!(result, vec!["hello", "world!"]); // "!" not in strip set
    }

    #[test]
    fn extract_command_keywords_empty() {
        let result = extract_command_keywords("");
        assert!(result.is_empty());
    }

    // --- normalize_draft ---
    #[test]
    fn normalize_draft_trims_fields() {
        let draft = StructuredNoteDraft {
            title: "  Title  ".into(),
            summary: "  Summary  ".into(),
            tags: vec![" tag1 ".into(), "tag1".into()],
            keywords: vec![],
            platform: " p ".into(),
            board: " b ".into(),
            kernel: " k ".into(),
            status: " s ".into(),
            source: " captured ".into(),
            body: " body ".into(),
        };
        let result = normalize_draft(draft);
        assert_eq!(result.title, "Title");
        assert_eq!(result.summary, "Summary");
        assert_eq!(result.tags, vec!["tag1"]); // deduped
        assert_eq!(result.status, "s"); // trimmed by normalize_draft
    }

    #[test]
    fn normalize_draft_empty_fields_use_fallback() {
        let draft = StructuredNoteDraft {
            title: "".into(),
            summary: "".into(),
            tags: vec![],
            keywords: vec![],
            platform: "".into(),
            board: "".into(),
            kernel: "".into(),
            status: "".into(),
            source: "".into(),
            body: "".into(),
        };
        let result = normalize_draft(draft);
        assert!(!result.title.is_empty());
        assert_eq!(result.status, "已记录");
        assert_eq!(result.source, "captured");
    }

    // --- generate_programmatic_snippet ---
    #[test]
    fn generate_programmatic_snippet_highlight_match() {
        let body = "First paragraph.\n\nThis paragraph has rust in it.\n\nThird.";
        let result = generate_programmatic_snippet(body, "rust");
        assert!(result.contains("==rust=="));
    }

    #[test]
    fn generate_programmatic_snippet_no_match_fallback() {
        let body = "First paragraph.\n\nSecond paragraph.";
        let result = generate_programmatic_snippet(body, "xyz");
        assert!(!result.contains("=="));
    }

    #[test]
    fn generate_programmatic_snippet_empty_query() {
        let body = "Some body text here.";
        let result = generate_programmatic_snippet(body, "");
        assert_eq!(result, "Some body text here.");
    }

    #[test]
    fn generate_programmatic_snippet_case_insensitive() {
        let body = "This has RUST in it.";
        let result = generate_programmatic_snippet(body, "rust");
        assert!(result.contains("==RUST=="));
    }

    // --- enrich_citations ---
    #[test]
    fn enrich_citations_empty_citations() {
        let result = enrich_citations(vec![], &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn enrich_citations_uses_fts5_snippet() {
        let citation = AnswerCitation {
            note_id: "n1".into(),
            title: "T".into(),
            path: "p".into(),
            snippet: "old".into(),
            score: None,
        };
        let doc = NoteDocument {
            meta: NoteMeta {
                id: "n1".into(),
                ..Default::default()
            },
            body: "body text".into(),
            search_snippet: Some("FTS5 ==match== here".into()),
        };
        let result = enrich_citations(vec![citation], &[doc]);
        assert_eq!(result[0].snippet, "FTS5 ==match== here");
    }

    #[test]
    fn enrich_citations_generates_snippet_when_short() {
        let citation = AnswerCitation {
            note_id: "n1".into(),
            title: "rust tips".into(),
            path: "p".into(),
            snippet: "short".into(),
            score: None,
        };
        let doc = NoteDocument {
            meta: NoteMeta {
                id: "n1".into(),
                ..Default::default()
            },
            body: "This body contains rust tips for beginners.".into(),
            search_snippet: None,
        };
        let result = enrich_citations(vec![citation], &[doc]);
        // Should generate a programmatic snippet since original is < 20 chars
        assert!(result[0].snippet.len() > 5);
    }

    #[test]
    fn enrich_citations_no_matching_doc() {
        let citation = AnswerCitation {
            note_id: "n999".into(),
            title: "T".into(),
            path: "p".into(),
            snippet: "original".into(),
            score: None,
        };
        let result = enrich_citations(vec![citation], &[]);
        assert_eq!(result[0].snippet, "original");
    }

    // --- parse_record_response ---
    #[test]
    fn parse_record_response_valid() {
        let text = r#"{"reply":"Done","noteDraft":{"title":"T","summary":"S","tags":[],"keywords":[],"platform":"","board":"","kernel":"","status":"recorded","source":"captured","body":"B"}}"#;
        let result = parse_record_response(text, "raw", sample_usage()).unwrap();
        assert_eq!(result.reply, "Done");
        assert_eq!(result.note_draft.title, "T");
    }

    #[test]
    fn parse_record_response_empty_reply_uses_fallback() {
        let text = r#"{"reply":"  ","noteDraft":{"title":"MyTitle","summary":"S","tags":[],"keywords":[],"platform":"","board":"","kernel":"","status":"recorded","source":"captured","body":"B"}}"#;
        let result = parse_record_response(text, "raw", sample_usage()).unwrap();
        assert!(result.reply.contains("MyTitle"));
    }

    #[test]
    fn parse_record_response_no_draft_returns_err() {
        let text = r#"{"reply":"Done"}"#;
        assert!(parse_record_response(text, "raw input", sample_usage()).is_err());
    }

    #[test]
    fn parse_record_response_invalid_json_returns_err() {
        assert!(parse_record_response("not json", "raw", sample_usage()).is_err());
    }

    // --- Anthropic response parsing ---
    #[test]
    fn anthropic_response_deserialize() {
        let json = r#"{"content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":10,"output_tokens":5}}"#;
        let resp: AnthropicResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.content[0].text.as_deref(), Some("hello"));
        assert_eq!(resp.usage.input_tokens, 10);
    }

    #[test]
    fn anthropic_response_with_error() {
        let json = r#"{"error":{"message":"rate limited"}}"#;
        let resp: AnthropicResponse = serde_json::from_str(json).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().message, "rate limited");
    }

    // --- OpenAI response parsing ---
    #[test]
    fn openai_response_deserialize() {
        let json = r#"{"choices":[{"message":{"content":"hi"}}],"usage":{"prompt_tokens":20,"completion_tokens":10}}"#;
        let resp: OpenAiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("hi"));
        assert_eq!(resp.usage.prompt_tokens, 20);
    }

    #[test]
    fn openai_response_with_error() {
        let json = r#"{"error":{"message":"invalid key"}}"#;
        let resp: OpenAiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.error.is_some());
    }

    // --- SaveNote draft edge cases ---

    #[test]
    fn parse_tool_call_save_note_empty_body_uses_heuristic() {
        let text = r#"{"tool":"save_note","limit":5,"noteDraft":{"title":"T","summary":"S","tags":[],"keywords":[],"platform":"","board":"","kernel":"","status":"","source":"captured","body":""}}"#;
        let result = parse_tool_call(text, "q").unwrap();
        match result {
            AssistantToolCall::SaveNote { draft, .. } => {
                // empty body triggers heuristic fallback
                assert!(!draft.body.is_empty());
            }
            _ => panic!("Expected SaveNote"),
        }
    }

    #[test]
    fn parse_tool_call_save_note_whitespace_title_uses_fallback() {
        let text = r#"{"tool":"save_note","limit":5,"noteDraft":{"title":"   ","summary":"S","tags":[],"keywords":[],"platform":"","board":"","kernel":"","status":"","source":"captured","body":"some content"}}"#;
        let result = parse_tool_call(text, "q").unwrap();
        match result {
            AssistantToolCall::SaveNote { draft, .. } => {
                // whitespace-only title gets heuristic fallback
                assert_ne!(draft.title, "   ");
                assert!(!draft.title.is_empty());
            }
            _ => panic!("Expected SaveNote"),
        }
    }

    #[test]
    fn parse_tool_call_save_note_duplicate_tags_deduped() {
        let text = r#"{"tool":"save_note","limit":5,"noteDraft":{"title":"T","summary":"S","tags":["rust","rust"," coding ","coding"],"keywords":[],"platform":"","board":"","kernel":"","status":"","source":"captured","body":"B"}}"#;
        let result = parse_tool_call(text, "q").unwrap();
        match result {
            AssistantToolCall::SaveNote { draft, .. } => {
                // tags should be deduped and trimmed
                let rust_count = draft.tags.iter().filter(|t| t.as_str() == "rust").count();
                assert_eq!(rust_count, 1);
                assert!(draft.tags.contains(&"coding".to_string()));
            }
            _ => panic!("Expected SaveNote"),
        }
    }

    #[test]
    fn parse_tool_call_save_note_missing_optional_fields() {
        let text = r#"{"tool":"save_note","limit":5,"noteDraft":{"title":"T","summary":"","tags":[],"keywords":[],"status":"","source":"","body":"content"}}"#;
        let result = parse_tool_call(text, "q").unwrap();
        match result {
            AssistantToolCall::SaveNote { draft, .. } => {
                assert_eq!(draft.title, "T");
                assert_eq!(draft.status, "已记录");
                assert_eq!(draft.source, "captured");
                assert_eq!(draft.body, "content");
            }
            _ => panic!("Expected SaveNote"),
        }
    }

    // ── Citation scoring (#1704) ───────────────────────────────────

    #[test]
    fn enrich_citations_populates_score_from_rank() {
        let citation = AnswerCitation {
            note_id: "n1".into(),
            title: "T".into(),
            path: "p".into(),
            snippet: "s".into(),
            score: None,
        };
        let docs = vec![
            NoteDocument {
                meta: NoteMeta {
                    id: "n1".into(),
                    ..Default::default()
                },
                body: "body".into(),
                search_snippet: None,
            },
            NoteDocument {
                meta: NoteMeta {
                    id: "n2".into(),
                    ..Default::default()
                },
                body: "This is a longer body for the second document.".into(),
                search_snippet: None,
            },
        ];
        let result = enrich_citations(vec![citation], &docs);
        assert!(result[0].score.is_some());
        assert!((result[0].score.unwrap() - 1.0).abs() < 0.01);
    }

    #[test]
    fn enrich_citations_score_decreases_with_rank() {
        let cit1 = AnswerCitation {
            note_id: "n1".into(),
            title: "First".into(),
            path: "p1".into(),
            snippet: "s".into(),
            score: None,
        };
        let cit2 = AnswerCitation {
            note_id: "n2".into(),
            title: "Second".into(),
            path: "p2".into(),
            snippet: "s".into(),
            score: None,
        };
        let docs = vec![
            NoteDocument {
                meta: NoteMeta {
                    id: "n1".into(),
                    ..Default::default()
                },
                body: "This is a longer body for the first document.".into(),
                search_snippet: None,
            },
            NoteDocument {
                meta: NoteMeta {
                    id: "n2".into(),
                    ..Default::default()
                },
                body: "This is a longer body for the second document.".into(),
                search_snippet: None,
            },
        ];
        let result = enrich_citations(vec![cit1, cit2], &docs);
        let s1 = result[0].score.unwrap();
        let s2 = result[1].score.unwrap();
        assert!(
            s1 > s2,
            "rank 0 score {} should be > rank 1 score {}",
            s1,
            s2
        );
    }

    #[test]
    fn enrich_citations_preserves_existing_score() {
        let citation = AnswerCitation {
            note_id: "n1".into(),
            title: "T".into(),
            path: "p".into(),
            snippet: "s".into(),
            score: Some(0.42),
        };
        let docs = vec![NoteDocument {
            meta: NoteMeta {
                id: "n1".into(),
                ..Default::default()
            },
            body: "body".into(),
            search_snippet: None,
        }];
        let result = enrich_citations(vec![citation], &docs);
        assert!((result[0].score.unwrap() - 0.42).abs() < 0.001);
    }

    #[test]
    fn enrich_citations_single_doc_gets_full_score() {
        let citation = AnswerCitation {
            note_id: "n1".into(),
            title: "T".into(),
            path: "p".into(),
            snippet: "s".into(),
            score: None,
        };
        let docs = vec![NoteDocument {
            meta: NoteMeta {
                id: "n1".into(),
                ..Default::default()
            },
            body: "body".into(),
            search_snippet: None,
        }];
        let result = enrich_citations(vec![citation], &docs);
        assert!((result[0].score.unwrap() - 1.0).abs() < 0.01);
    }
}
