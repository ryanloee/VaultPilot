//! Deep Research mode — Agent 自主规划、多轮搜索、生成引用溯源报告 (#1961).
//!
//! This module implements a multi-round research capability where:
//! 1. The user enters a research topic
//! 2. Vault notes related to the topic are surfaced and injected into planning
//! 3. The AI internally plans a research outline (list of sub-questions)
//! 4. Multi-round searches execute — vault notes + web results combined (#1631)
//! 5. Results are synthesized into a structured report with citations
//! 6. The report is auto-saved as a vault note
//!
//! Vault-aware (#1631): the planning phase and each search round now include
//! relevant vault notes alongside web search results, giving the AI access to
//! the user's existing knowledge base.
//!
//! Two tiers are supported:
//! - **Fast**  (3–5 rounds, ~30s target)
//! - **Deep**  (10–20 rounds, 2–5min target)

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::ai::client::send_request_with_temperature;
use crate::ai::parsing::extract_json;
use crate::ai::RequestUsage;
use crate::models::{AppSettings, NoteDocument, NoteMeta};
use crate::storage::{
    find_related_notes_for_text_with_context, load_note_with_context, save_note_with_context,
    StorageContext,
};

// ── Tier configuration ───────────────────────────────────────────────────

/// Execution tier for deep research.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeepResearchTier {
    /// Fast tier: 3–5 search rounds, ~30 seconds target.
    #[default]
    Fast,
    /// Deep tier: 10–20 search rounds, 2–5 minute target.
    Deep,
}

impl DeepResearchTier {
    fn min_rounds(&self) -> usize {
        match self {
            Self::Fast => 3,
            Self::Deep => 10,
        }
    }

    fn max_rounds(&self) -> usize {
        match self {
            Self::Fast => 5,
            Self::Deep => 20,
        }
    }

    #[allow(dead_code)]
    fn timeout(&self) -> Duration {
        match self {
            Self::Fast => Duration::from_secs(60),
            Self::Deep => Duration::from_secs(300),
        }
    }
}

// ── Data types ────────────────────────────────────────────────────────────

/// A single sub-question in the research plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSubQuestion {
    /// The sub-question text.
    pub question: String,
    /// Rationale for why this question is important to answer.
    #[serde(default)]
    pub rationale: String,
    /// Search queries for this sub-question (one or more).
    #[serde(default)]
    pub search_queries: Vec<String>,
}

/// The research outline produced by the planning phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchPlan {
    /// The original research topic.
    pub topic: String,
    /// Executive summary / goal of the research.
    #[serde(default)]
    pub goal: String,
    /// Ordered list of sub-questions to investigate.
    pub sub_questions: Vec<ResearchSubQuestion>,
}

/// Result of a single search round.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRoundResult {
    /// The sub-question this round addressed.
    pub question: String,
    /// The search query used.
    pub query: String,
    /// Raw search results (snippets + URLs).
    pub raw_results: String,
    /// AI-generated summary of findings from this round.
    pub summary: String,
    /// Index of this round (1-based).
    pub round_number: usize,
}

/// A single citation with source information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchCitation {
    /// Citation number (displayed as superscript).
    pub number: usize,
    /// The source URL.
    pub url: String,
    /// The title of the source.
    pub title: String,
    /// A short snippet from the source.
    #[serde(default)]
    pub snippet: String,
}

/// The final research report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResult {
    /// The research topic.
    pub topic: String,
    /// The full report body (Markdown with citations).
    pub report: String,
    /// Collected citations with source information.
    pub citations: Vec<ResearchCitation>,
    /// Number of search rounds executed.
    pub rounds_used: usize,
    /// Token usage across all LLM calls.
    pub total_usage: RequestUsage,
    /// Note ID of the saved vault note (if saved successfully).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_note_id: Option<String>,
    /// Note title of the saved vault note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_note_title: Option<String>,
    /// Any error that occurred during execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ── Status callback ─────────────────────────────────────────────────────

/// Progress events emitted during deep research execution.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum DeepResearchEvent {
    /// Surfacing vault notes relevant to the topic (#1631).
    VaultContext {
        /// Number of vault notes found related to the topic.
        count: usize,
        detail: String,
    },
    /// Planning the research outline.
    Planning { detail: String },
    /// A sub-question is being researched.
    Searching {
        round: usize,
        total_rounds: usize,
        question: String,
        query: String,
    },
    /// Search results received.
    SearchResult {
        round: usize,
        question: String,
        result_preview: String,
    },
    /// Synthesizing the final report.
    Synthesizing,
    /// Saving the report as a vault note.
    Saving { title: String },
    /// Completed successfully.
    Completed { note_id: String, note_title: String },
    /// Error occurred.
    Error { message: String },
}

// ── Main entry point ──────────────────────────────────────────────────────

/// Run a deep research session.
///
/// This is the main public API for the deep research feature. It:
/// 1. Plans a research outline via AI
/// 2. Executes multi-round web searches
/// 3. Synthesizes results into a structured report with citations
/// 4. Auto-saves the report as a vault note
///
/// # Arguments
/// * `settings` - App settings (provider, model, etc.)
/// * `context` - Storage context for vault operations
/// * `topic` - The research topic/question from the user
/// * `tier` - Fast (3-5 rounds) or Deep (10-20 rounds)
/// * `emit` - Callback for progress events
#[instrument(skip(settings, context, emit))]
pub async fn run_deep_research(
    settings: &AppSettings,
    context: &StorageContext,
    topic: &str,
    tier: DeepResearchTier,
    mut emit: impl FnMut(DeepResearchEvent),
) -> Result<ResearchResult> {
    let topic = topic.trim().to_string();
    if topic.is_empty() {
        return Err(anyhow!("research topic cannot be empty"));
    }

    // ── Phase 0: Search vault for existing knowledge (#1631) ────────────
    let vault_context = match vault_search(context, &topic, 8) {
        Ok(vc) => {
            let count = if vc.is_empty() {
                0
            } else {
                vc.lines()
                    .filter(|l| l.starts_with("--- vault note"))
                    .count()
            };
            emit(DeepResearchEvent::VaultContext {
                count,
                detail: if count > 0 {
                    format!("Found {} vault notes related to \"{}\"", count, topic)
                } else {
                    format!("No existing vault notes found for \"{}\"", topic)
                },
            });
            vc
        }
        Err(e) => {
            tracing::warn!("vault search failed during deep research: {}", e);
            String::new()
        }
    };

    emit(DeepResearchEvent::Planning {
        detail: format!("Planning research outline for: {}", topic),
    });

    // ── Phase 1: Plan the research outline (vault-aware) ────────────────
    let (plan, plan_usage) =
        generate_research_plan(settings, &topic, &tier, &vault_context).await?;
    let mut total_usage = plan_usage;
    let total_rounds = plan.sub_questions.len();
    if total_rounds == 0 {
        return Err(anyhow!("research plan has no sub-questions"));
    }

    // Clamp rounds to tier limits
    let total_rounds = total_rounds.min(tier.max_rounds()).max(tier.min_rounds());

    emit(DeepResearchEvent::Planning {
        detail: format!(
            "Research plan ready: {} sub-questions across {} rounds",
            plan.sub_questions.len(),
            total_rounds
        ),
    });

    // ── Phase 2: Multi-round searches (vault + web, #1631) ──────────────
    let mut round_results: Vec<SearchRoundResult> = Vec::new();
    let mut all_citations: Vec<ResearchCitation> = Vec::new();
    let mut citation_counter: usize = 0;

    for (i, sub_q) in plan.sub_questions.iter().enumerate() {
        if i >= total_rounds {
            break;
        }

        // Determine search queries for this sub-question
        let queries = if sub_q.search_queries.is_empty() {
            vec![sub_q.question.clone()]
        } else {
            sub_q.search_queries.clone()
        };

        for query in &queries {
            if round_results.len() >= total_rounds {
                break;
            }
            let round_num = round_results.len() + 1;

            emit(DeepResearchEvent::Searching {
                round: round_num,
                total_rounds,
                question: sub_q.question.clone(),
                query: query.clone(),
            });

            // Execute vault search for this sub-question (sync, #1631)
            let vault_results = match vault_search(context, query, 3) {
                Ok(vr) => vr,
                Err(e) => {
                    tracing::warn!("vault search failed for query '{}': {}", query, e);
                    String::new()
                }
            };

            // Execute web search
            let raw_web = match web_search(settings, query).await {
                Ok(results) => results,
                Err(e) => {
                    tracing::warn!("web search failed for query '{}': {}", query, e);
                    format!("[Search failed: {}]", crate::sanitize_error(&e.to_string()))
                }
            };

            // Combine vault + web results into unified raw results
            let raw_results = if !vault_results.is_empty() {
                format!(
                    "=== VAULT NOTES (existing knowledge) ===\n{}\n\n=== WEB RESULTS ===\n{}",
                    vault_results, raw_web
                )
            } else {
                raw_web
            };

            let result_preview = raw_results.chars().take(200).collect::<String>();
            emit(DeepResearchEvent::SearchResult {
                round: round_num,
                question: sub_q.question.clone(),
                result_preview: if raw_results.contains("[Search failed") {
                    format!("⚠️ {}", result_preview)
                } else {
                    format!("✓ Found results: {}", result_preview)
                },
            });

            // AI summary of this round's findings
            let summary = if raw_results.contains("[Search failed") {
                raw_results.clone()
            } else {
                match summarize_search_round(settings, &sub_q.question, query, &raw_results).await {
                    Ok((s, usage)) => {
                        total_usage = merge_usage(total_usage, usage);
                        s
                    }
                    Err(e) => {
                        tracing::warn!("summarize failed, falling back to raw results: {e}");
                        raw_results.clone()
                    }
                }
            };

            // Extract citations from raw results
            let new_citations = extract_citations_from_results(&raw_results, &mut citation_counter);
            all_citations.extend(new_citations);

            round_results.push(SearchRoundResult {
                question: sub_q.question.clone(),
                query: query.clone(),
                raw_results: raw_results.clone(),
                summary: summary.clone(),
                round_number: round_num,
            });
        }
    }

    // ── Phase 3: Synthesize final report ─────────────────────────────────
    emit(DeepResearchEvent::Synthesizing);

    let (report, report_citations, synthesis_usage) =
        synthesize_report(settings, &topic, &plan, &round_results, &all_citations).await?;
    total_usage = merge_usage(total_usage, synthesis_usage);

    // ── Phase 4: Save as vault note ─────────────────────────────────────
    let now = Utc::now();
    let note_title = format!(
        "[Deep Research] {} — {}",
        topic,
        now.format("%Y-%m-%d %H:%M")
    );
    let note_body = report.clone();
    let note_summary = report
        .chars()
        .take(300)
        .collect::<String>()
        .lines()
        .next()
        .unwrap_or(&note_title)
        .to_string();

    let meta = NoteMeta {
        title: note_title.clone(),
        summary: note_summary,
        source: "deep_research".to_string(),
        tags: vec![
            "deep-research".to_string(),
            format!(
                "tier:{}",
                if tier == DeepResearchTier::Fast {
                    "fast"
                } else {
                    "deep"
                }
            ),
        ],
        collections: vec!["Deep Research".to_string()],
        ..Default::default()
    };

    let note = NoteDocument {
        meta,
        body: note_body,
        search_snippet: None,
        search_score: None,
    };

    emit(DeepResearchEvent::Saving {
        title: note_title.clone(),
    });

    let saved_note = save_note_with_context(context, note)
        .map_err(|e| anyhow!("failed to save research report as note: {}", e))?;

    let saved_note_id = saved_note.meta.id.clone();
    let saved_note_title = saved_note.meta.title.clone();

    emit(DeepResearchEvent::Completed {
        note_id: saved_note_id.clone(),
        note_title: saved_note_title.clone(),
    });

    Ok(ResearchResult {
        topic: topic.clone(),
        report,
        citations: report_citations,
        rounds_used: round_results.len(),
        total_usage,
        saved_note_id: Some(saved_note_id),
        saved_note_title: Some(saved_note_title),
        error: None,
    })
}

// ── Phase 1: Planning ────────────────────────────────────────────────────

/// Search vault notes related to a query. Returns formatted text suitable for
/// injecting into AI prompts (title, summary, body snippet for each note).
/// This is a synchronous operation (SQLite queries).
fn vault_search(context: &StorageContext, query: &str, limit: usize) -> Result<String> {
    let related = find_related_notes_for_text_with_context(context, query, limit)?;
    if related.is_empty() {
        return Ok(String::new());
    }
    let mut parts = Vec::with_capacity(related.len());
    for rn in &related {
        let body_preview = match load_note_with_context(context, &rn.meta.id) {
            Ok(doc) => {
                let preview: String = doc.body.chars().take(300).collect();
                if preview.len() < doc.body.len() {
                    format!("{}…", preview)
                } else {
                    preview
                }
            }
            Err(_) => String::from("(body unavailable)"),
        };
        let tag_str = if rn.meta.tags.is_empty() {
            String::new()
        } else {
            format!(" [tags: {}]", rn.meta.tags.join(", "))
        };
        parts.push(format!(
            "--- vault note: {} (score: {}){} ---\\nSummary: {}\\nBody preview:\\n{}",
            rn.meta.title, rn.score, tag_str, rn.meta.summary, body_preview
        ));
    }
    Ok(parts.join("\\n\\n"))
}

/// Generate a research outline by asking the AI to decompose the topic.
/// vault_context contains formatted results from a vault search injected
/// to help the AI plan more relevant sub-questions (#1631).
async fn generate_research_plan(
    settings: &AppSettings,
    topic: &str,
    _tier: &DeepResearchTier,
    vault_context: &str,
) -> Result<(ResearchPlan, RequestUsage)> {
    let system = "\
You are a research planning assistant. Your task is to decompose a research topic \
into a structured set of sub-questions that, when answered, form a comprehensive \
research report.

Rules:
- Break the topic into 3-8 focused sub-questions
- Each sub-question should target a distinct aspect of the topic
- For each, provide 1-2 specific search queries that would find relevant information
- Include a brief rationale explaining why each sub-question matters

Return ONLY valid JSON with no markdown fences or extra text.";

    let vault_block = if vault_context.is_empty() {
        String::new()
    } else {
        format!(
            "\\nRelevant notes from the user's vault (existing knowledge):\\n{vault}\\n\\n\
             Use this vault knowledge to inform your research plan — \
             focus on areas NOT already covered by vault notes.",
            vault = vault_context
        )
    };

    let user_prompt = format!(
        r#"Research topic: "{topic}"{vault_block}

Generate a research plan. Return JSON in this exact schema (all fields required):
{{
  "topic": "<the original topic>",
  "goal": "<one-sentence research goal>",
  "subQuestions": [
    {{
      "question": "<sub-question>",
      "rationale": "<why this sub-question matters>",
      "searchQueries": ["<specific search query 1>", "<specific search query 2>"]
    }}
  ]
}}

Focus on producing sub-questions that are:
1. Answerable via web search
2. Cover different dimensions of the topic (background, current state, controversies, future outlook, etc.)
3. Logically ordered from foundational to advanced
4. Complement (don't duplicate) the vault knowledge already available"#,
        topic = topic,
        vault_block = vault_block
    );

    let response = send_request_with_temperature(settings, system, &user_prompt, &[], 0.2)
        .await
        .context("AI planning call failed")?;

    let json_text = extract_json(&response.text).context(format!(
        "model did not return valid JSON for research plan. Response: {}",
        crate::sanitize_error(&response.text.chars().take(200).collect::<String>())
    ))?;

    let plan: ResearchPlan = serde_json::from_str(&json_text).context(format!(
        "failed to parse research plan JSON: {}",
        crate::sanitize_error(&json_text.chars().take(200).collect::<String>())
    ))?;

    if plan.sub_questions.is_empty() {
        return Err(anyhow!("research plan has no sub-questions"));
    }

    Ok((plan, response.usage))
}

// ── Phase 2: Web Search ──────────────────────────────────────────────────

/// Execute a web search for the given query.
///
/// Uses DuckDuckGo's HTML-based lite search (no API key required).
/// Returns formatted search results as plain text.
async fn web_search(_settings: &AppSettings, query: &str) -> Result<String> {
    // Use DuckDuckGo's lite search which returns simple HTML
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(
            "Mozilla/5.0 (compatible; VaultPilot/1.0; +https://github.com/ryanloee/VaultPilot)",
        )
        .build()
        .context("failed to create HTTP client for web search")?;

    let response = client
        .post("https://lite.duckduckgo.com/lite/")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("q={}", urlencoding(query)))
        .send()
        .await
        .context("web search request failed")?;

    let html = response.text().await?;

    // Parse the simple HTML table from DuckDuckGo lite
    // The lite version returns results in a specific table structure
    let results = parse_ddg_lite_html(&html);

    if results.is_empty() {
        // Fallback: try html.duckduckgo.com
        let response2 = client
            .post("https://html.duckduckgo.com/html/")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(format!("q={}", urlencoding(query)))
            .send()
            .await
            .context("web search fallback request failed")?;

        let html2 = response2.text().await?;
        let results2 = parse_ddg_html_results(&html2);

        if results2.is_empty() {
            Ok(format!(
                "No search results found for query: {}\n\
                 The search engine returned an empty response.",
                query
            ))
        } else {
            Ok(results2)
        }
    } else {
        Ok(results)
    }
}

/// URL-encode a string for form submission.
fn urlencoding(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

/// Parse DuckDuckGo lite HTML search results.
fn parse_ddg_lite_html(html: &str) -> String {
    let mut results = Vec::new();

    // DDG lite returns results in a straightforward HTML table.
    // We extract text from table rows.
    for table in html.split("<table") {
        if !table.contains("result-link") && !table.contains("result-snippet") {
            continue;
        }
        // Extract rows
        for row in table.split("<tr") {
            let row_text = strip_html_tags(row);
            let row_text = row_text.trim();
            if !row_text.is_empty() && row_text.len() > 10 {
                results.push(row_text.to_string());
            }
        }
    }

    // Also try extracting any link text
    if results.is_empty() {
        for table in html.split("<table") {
            for row in table.split("<tr") {
                if let Some(link_start) = row.find("<a ") {
                    let after_link = &row[link_start..];
                    if let Some(href_start) = after_link.find("href=\"") {
                        let href_end = after_link[href_start + 6..]
                            .find('"')
                            .map(|e| href_start + 6 + e)
                            .unwrap_or(0);
                        let url = &after_link[href_start + 6..href_end];
                        let text = strip_html_tags(after_link);
                        if !url.is_empty() && !text.is_empty() && url != "#" {
                            results.push(format!("- {} ({})", text.trim(), url.trim()));
                        }
                    }
                }
            }
        }
    }

    results.join("\n")
}

/// Parse DuckDuckGo HTML search results (html.duckduckgo.com/html/).
fn parse_ddg_html_results(html: &str) -> String {
    let mut results = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    // Extract result blocks
    for block in html.split("class=\"result__body\"") {
        if block.len() < 50 {
            continue;
        }

        // Extract title
        let title = block
            .split("class=\"result__title\"")
            .nth(1)
            .and_then(|s| {
                let after_a = s.find(">").map(|p| &s[p + 1..]);
                after_a.map(|a| {
                    let end = a.find("</a>").unwrap_or(a.len());
                    strip_html_tags(&a[..end]).trim().to_string()
                })
            })
            .unwrap_or_default();

        // Extract URL
        let url = block
            .split("class=\"result__url\"")
            .nth(1)
            .and_then(|s| {
                s.split("href=\"")
                    .nth(1)
                    .and_then(|h| h.split('"').next())
                    .map(|u| u.to_string())
            })
            .unwrap_or_default();

        // Extract snippet
        let snippet = block
            .split("class=\"result__snippet\"")
            .nth(1)
            .map(|s| {
                let start = s.find(">").map(|p| p + 1).unwrap_or(0);
                let end = s.find("</a>").unwrap_or(s.len().min(start + 300));
                strip_html_tags(&s[start..end]).trim().to_string()
            })
            .unwrap_or_default();

        if !url.is_empty() && seen_urls.insert(url.clone()) {
            if !title.is_empty() {
                results.push(format!("- **{}**", title));
            }
            results.push(format!("  URL: {}", url));
            if !snippet.is_empty() {
                results.push(format!("  Snippet: {}", snippet));
            }
            results.push(String::new());
        }
    }

    results.join("\n")
}

/// Strip HTML tags from a string.
fn strip_html_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    // Clean up whitespace
    let cleaned: Vec<&str> = out.split_whitespace().collect();
    cleaned.join(" ")
}

/// AI summarization of a single search round's findings.
async fn summarize_search_round(
    settings: &AppSettings,
    question: &str,
    query: &str,
    raw_results: &str,
) -> Result<(String, RequestUsage)> {
    let system = "\
You are a research assistant analyzing web search results. \
Summarize the key findings from the search results that are relevant to the sub-question. \
Be specific and cite URLs where possible. \
Keep the summary concise (2-4 paragraphs).";

    let user_prompt = format!(
        r#"Sub-question: {question}
Search query: {query}

Raw search results:
{raw_results}

Please provide a concise summary of the key findings relevant to the sub-question.
Focus on factual information, specific data points, and named sources."#,
        question = question,
        query = query,
        raw_results = raw_results
    );

    let response = send_request_with_temperature(settings, system, &user_prompt, &[], 0.3)
        .await
        .context("AI search round summarization failed")?;

    Ok((response.text.trim().to_string(), response.usage))
}

/// Extract citations from raw search results.
fn extract_citations_from_results(raw_results: &str, counter: &mut usize) -> Vec<ResearchCitation> {
    let mut citations = Vec::new();

    for line in raw_results.lines() {
        let line = line.trim();
        if let Some(url_start) = line.find("http://").or_else(|| line.find("https://")) {
            let url_end = line[url_start..]
                .find(|c: char| c.is_whitespace() || c == ')' || c == '>' || c == '"')
                .map(|e| url_start + e)
                .unwrap_or(line.len());
            let url = &line[url_start..url_end];

            // Extract title from before the URL
            let title = if url_start > 0 {
                let before = line[..url_start].trim();
                before
                    .trim_start_matches('-')
                    .trim()
                    .trim_start_matches("**")
                    .trim_end_matches("**")
                    .trim()
                    .to_string()
            } else {
                url.to_string()
            };

            *counter += 1;
            citations.push(ResearchCitation {
                number: *counter,
                url: url.to_string(),
                title: title.clone(),
                snippet: line.chars().take(150).collect(),
            });
        }
    }

    citations
}

// ── Phase 3: Synthesis ────────────────────────────────────────────────────

/// Synthesize a final structured report from all search round results.
async fn synthesize_report(
    settings: &AppSettings,
    topic: &str,
    plan: &ResearchPlan,
    round_results: &[SearchRoundResult],
    citations: &[ResearchCitation],
) -> Result<(String, Vec<ResearchCitation>, RequestUsage)> {
    // Build the search context for the AI
    let mut context_parts = Vec::new();
    for (i, r) in round_results.iter().enumerate() {
        context_parts.push(format!(
            "## Round {}: {}\nQuery: {}\nSummary:\n{}\n",
            i + 1,
            r.question,
            r.query,
            r.summary
        ));
    }

    let search_context = context_parts.join("\n---\n");

    // Build citation reference list
    let citation_refs: Vec<String> = citations
        .iter()
        .map(|c| format!("[{}. {} - {}]({})", c.number, c.title, c.snippet, c.url))
        .collect();
    let citation_block = citation_refs.join("\n");

    let system = format!(
        "\
You are a research report writer. You have conducted multi-round research on a topic \
using both the user's vault notes (existing knowledge) and web searches. \
Your task is to synthesize the findings into a well-structured, comprehensive research report.

Rules:
1. Write in Markdown format with proper headings, paragraphs, and sections
2. Include numbered citations in the text using [^1], [^2], [^3] etc.
3. Every substantive claim should be supported by a citation
4. Organize the report logically:
   - Executive Summary
   - Background / Context
   - Key Findings (organized by sub-topic)
   - Analysis / Discussion
   - Conclusion
5. At the end, include a \"## References\" section listing all cited sources
6. Write in a professional, academic style
7. Be objective and balanced, presenting different viewpoints when they exist
8. The report should be comprehensive but readable (not overly verbose)

Tier: {}
Rounds completed: {}",
        if round_results.len() <= 5 {
            "Fast (5 rounds)"
        } else {
            "Deep (15 rounds)"
        },
        round_results.len()
    );

    let goal_str = if plan.goal.is_empty() {
        String::new()
    } else {
        format!("\nResearch goal: {}", plan.goal)
    };

    let sub_questions_str: Vec<String> = plan
        .sub_questions
        .iter()
        .map(|q| format!("- {}", q.question))
        .collect();
    let sub_questions_block = sub_questions_str.join("\n");

    let user_prompt = format!(
        r#"Topic: "{topic}"{goal}

Research sub-questions investigated:
{sub_questions}

Search Results Summary:
{search_context}

Available Citations:
{citations}

Please synthesize a comprehensive research report. Use [^1], [^2], etc. to cite sources within the text, \
and list all cited sources in the References section at the end.

IMPORTANT: Return the complete report as a Markdown string. Do NOT wrap it in JSON or markdown fences."#,
        topic = topic,
        goal = goal_str,
        sub_questions = sub_questions_block,
        search_context = search_context,
        citations = citation_block,
    );

    let response = send_request_with_temperature(settings, &system, &user_prompt, &[], 0.3)
        .await
        .context("AI report synthesis failed")?;

    let report_text = response.text.trim().to_string();

    // Re-number citations based on what's actually used in the report
    let report_citations = renumber_citations(&report_text, citations);

    Ok((report_text, report_citations, response.usage))
}

/// Re-number citations based on what's actually referenced in the report.
fn renumber_citations(report: &str, all_citations: &[ResearchCitation]) -> Vec<ResearchCitation> {
    // Find all [^N] patterns in the report
    let mut used_numbers = std::collections::BTreeSet::new();
    for cap in report.split("[^") {
        if let Some(end) = cap.find(']') {
            if let Ok(n) = cap[..end].parse::<usize>() {
                used_numbers.insert(n);
            }
        }
    }

    // Build a mapping from old number to new sequential number
    let mut old_to_new: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (new_num, old_num) in used_numbers.iter().enumerate() {
        old_to_new.insert(*old_num, new_num + 1);
    }

    // Filter and re-number citations
    let mut new_citations: Vec<ResearchCitation> = all_citations
        .iter()
        .filter_map(|c| {
            old_to_new.get(&c.number).map(|new_num| ResearchCitation {
                number: *new_num,
                url: c.url.clone(),
                title: c.title.clone(),
                snippet: c.snippet.clone(),
            })
        })
        .collect();

    // Sort by citation number
    new_citations.sort_by_key(|c| c.number);

    new_citations
}

/// Merge two RequestUsage values by summing token counts.
/// Matches the semantics of ask.rs:merge_usage.
fn merge_usage(current: RequestUsage, next: RequestUsage) -> RequestUsage {
    RequestUsage {
        input_tokens: match (current.input_tokens, next.input_tokens) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        },
        output_tokens: match (current.output_tokens, next.output_tokens) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        },
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::delete_note_with_context;

    #[test]
    fn test_deep_research_tier_default() {
        let tier = DeepResearchTier::default();
        assert_eq!(tier, DeepResearchTier::Fast);
    }

    #[test]
    fn test_fast_tier_bounds() {
        let tier = DeepResearchTier::Fast;
        assert_eq!(tier.min_rounds(), 3);
        assert_eq!(tier.max_rounds(), 5);
    }

    #[test]
    fn test_deep_tier_bounds() {
        let tier = DeepResearchTier::Deep;
        assert_eq!(tier.min_rounds(), 10);
        assert_eq!(tier.max_rounds(), 20);
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(strip_html_tags("<b>hello</b> world"), "hello world");
        assert_eq!(strip_html_tags("no tags"), "no tags");
        assert_eq!(strip_html_tags("<div><p>nested</p></div>"), "nested");
        assert_eq!(strip_html_tags(""), "");
    }

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("a/b"), "a%2Fb");
        assert_eq!(urlencoding("simple"), "simple");
    }

    #[test]
    fn test_extract_citations_from_results() {
        let raw = "- **Rust Programming** (https://www.rust-lang.org/)\n\
                    Snippet: Rust is a systems programming language.\n\n\
                    - **Rust Book** (https://doc.rust-lang.org/book/)\n\
                    Snippet: Learn Rust.\n";
        let mut counter = 0;
        let citations = extract_citations_from_results(raw, &mut counter);
        assert_eq!(citations.len(), 2, "should find 2 URLs");
        assert_eq!(citations[0].number, 1);
        assert_eq!(citations[0].url, "https://www.rust-lang.org/");
        assert_eq!(citations[1].number, 2);
        assert_eq!(citations[1].url, "https://doc.rust-lang.org/book/");
    }

    #[test]
    fn test_extract_citations_no_results() {
        let raw = "No URLs in this text";
        let mut counter = 0;
        let citations = extract_citations_from_results(raw, &mut counter);
        assert!(citations.is_empty());
    }

    #[test]
    fn test_renumber_citations_all_used() {
        let report = "This is [^1] a test [^2] with citations [^3].";
        let citations = vec![
            ResearchCitation {
                number: 1,
                url: "https://example.com/1".into(),
                title: "Source 1".into(),
                snippet: "".into(),
            },
            ResearchCitation {
                number: 2,
                url: "https://example.com/2".into(),
                title: "Source 2".into(),
                snippet: "".into(),
            },
            ResearchCitation {
                number: 3,
                url: "https://example.com/3".into(),
                title: "Source 3".into(),
                snippet: "".into(),
            },
        ];
        let result = renumber_citations(report, &citations);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].number, 1);
        assert_eq!(result[1].number, 2);
        assert_eq!(result[2].number, 3);
    }

    #[test]
    fn test_renumber_citations_partial_use() {
        let report = "This references [^1] and [^3] but not 2.";
        let citations = vec![
            ResearchCitation {
                number: 1,
                url: "https://example.com/1".into(),
                title: "Source 1".into(),
                snippet: "".into(),
            },
            ResearchCitation {
                number: 2,
                url: "https://example.com/2".into(),
                title: "Source 2".into(),
                snippet: "".into(),
            },
            ResearchCitation {
                number: 3,
                url: "https://example.com/3".into(),
                title: "Source 3".into(),
                snippet: "".into(),
            },
        ];
        let result = renumber_citations(report, &citations);
        assert_eq!(result.len(), 2, "only 2 citations used");
        assert_eq!(result[0].number, 1);
        assert_eq!(result[0].url, "https://example.com/1");
        assert_eq!(result[1].number, 2);
        assert_eq!(result[1].url, "https://example.com/3");
    }

    #[test]
    fn test_renumber_citations_no_citations_in_report() {
        let report = "This has no citations.";
        let citations = vec![ResearchCitation {
            number: 1,
            url: "https://example.com".into(),
            title: "Source".into(),
            snippet: "".into(),
        }];
        let result = renumber_citations(report, &citations);
        assert!(result.is_empty());
    }

    #[test]
    fn test_research_plan_serde_roundtrip() {
        let plan = ResearchPlan {
            topic: "Rust async".to_string(),
            goal: "Understand async Rust".to_string(),
            sub_questions: vec![ResearchSubQuestion {
                question: "What is async Rust?".to_string(),
                rationale: "Foundational understanding".to_string(),
                search_queries: vec!["Rust async explained".to_string()],
            }],
        };
        let json = serde_json::to_string(&plan).unwrap();
        let deserialized: ResearchPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.topic, "Rust async");
        assert_eq!(deserialized.sub_questions.len(), 1);
    }

    #[test]
    fn test_deep_research_event_variants() {
        let events = [
            DeepResearchEvent::VaultContext {
                count: 3,
                detail: "Found 3 vault notes".into(),
            },
            DeepResearchEvent::Planning {
                detail: "planning".into(),
            },
            DeepResearchEvent::Searching {
                round: 1,
                total_rounds: 5,
                question: "test".into(),
                query: "test query".into(),
            },
            DeepResearchEvent::SearchResult {
                round: 1,
                question: "test".into(),
                result_preview: "results...".into(),
            },
            DeepResearchEvent::Synthesizing,
            DeepResearchEvent::Saving {
                title: "report".into(),
            },
            DeepResearchEvent::Completed {
                note_id: "n1".into(),
                note_title: "Report".into(),
            },
            DeepResearchEvent::Error {
                message: "err".into(),
            },
        ];
        assert_eq!(events.len(), 8);
    }

    // ── Vault-aware deep research tests (#1631) ──────────────────

    #[test]
    fn test_vault_search_formatting() {
        // Test that vault_search returns properly formatted output
        // by mocking a context with notes
        let ctx = StorageContext::for_test(&std::env::temp_dir().join(format!(
            "vaultpilot-test-dr-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )));
        crate::storage::initialize_storage_with_context(&ctx).unwrap();

        // Create a test note first
        let note = NoteDocument {
            meta: NoteMeta {
                title: "Rust Async Patterns".to_string(),
                summary: "Overview of async patterns".to_string(),
                tags: vec!["rust".to_string(), "async".to_string()],
                ..Default::default()
            },
            body: "Tokio is the most popular async runtime for Rust. \
                   It provides a multi-threaded work-stealing scheduler."
                .to_string(),
            search_snippet: None,
            search_score: None,
        };
        let saved = save_note_with_context(&ctx, note).unwrap();
        let result = vault_search(&ctx, "Rust async", 3).unwrap();
        // Should contain our saved note
        assert!(!result.is_empty(), "vault search should return results");
        assert!(
            result.contains("Rust Async Patterns"),
            "should contain note title: {}",
            result
        );

        // Clean up
        delete_note_with_context(&ctx, &saved.meta.id, None).unwrap();
    }

    #[test]
    fn test_vault_search_empty_vault() {
        let ctx = StorageContext::for_test(&std::env::temp_dir().join(format!(
            "vaultpilot-test-dr-empty-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )));
        crate::storage::initialize_storage_with_context(&ctx).unwrap();
        let result = vault_search(&ctx, "nonexistent topic xyz", 5).unwrap();
        assert!(result.is_empty(), "empty vault should return empty result");
    }

    /// Regression test for #2724: verify that synthesize_report correctly
    /// constructs search context from round_results (the variable formerly
    /// known as accumulated_search_context was dead code — the synthesis
    /// function independently builds its own context).
    #[test]
    fn test_synthesize_report_context_from_round_results() {
        let round_results = [SearchRoundResult {
            question: "What is Rust?".to_string(),
            query: "Rust programming language".to_string(),
            raw_results: "Rust is a systems programming language".to_string(),
            summary: "Rust: a safe, concurrent, practical language".to_string(),
            round_number: 1,
        }];
        // Build context the same way synthesize_report does (lines 738-749)
        let mut context_parts = Vec::new();
        for (i, r) in round_results.iter().enumerate() {
            context_parts.push(format!(
                "## Round {}: {}\nQuery: {}\nSummary:\n{}\n",
                i + 1,
                r.question,
                r.query,
                r.summary
            ));
        }
        let search_context = context_parts.join("\n---\n");
        // Verify the context contains both the question and the summary
        assert!(
            search_context.contains("What is Rust?"),
            "context should contain question"
        );
        assert!(
            search_context.contains("safe, concurrent, practical"),
            "context should contain summary"
        );
        assert!(
            search_context.contains("## Round 1:"),
            "context should contain round header"
        );
    }
}
