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
        format!(
            "{}…",
            &snippet[..snippet
                .char_indices()
                .take_while(|(i, _)| *i < 498)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(498)]
        )
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

    citations
        .into_iter()
        .map(|mut citation| {
            if let Some(doc) = doc_map.get(citation.note_id.as_str()) {
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

pub(super) fn extract_json(text: &str) -> Result<String> {
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
