//! Scheduled Research — AI subscription execution engine (#2167).
//!
//! Runs AI subscription tasks: computes the next run time from cron expressions,
//! substitutes placeholder variables ({{topic}}, {{date}}, etc.), optionally
//! includes the previous run's result as cross-run context, executes the prompt,
//! saves the result as a vault note in the target collection, and updates run
//! metadata.

use anyhow::Result;
use chrono::Utc;
use std::time::Duration;
use tracing::instrument;

use crate::models::{AiSubscription, NoteDocument, NoteMeta};
use crate::storage::subscriptions::{
    compute_and_update_next_run, list_due_subscriptions_with_context,
    update_subscription_run_with_context,
};
use crate::storage::{save_note_with_context, StorageContext};

/// Per-subscription AI call timeout (3 minutes).
const _SUBSCRIPTION_AI_TIMEOUT: Duration = Duration::from_secs(180);
/// Timeout for storage-layer I/O.
const _STORAGE_IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Run all due subscriptions (enabled + next_run_at <= now).
///
/// For each subscription:
/// 1. Substitute placeholder variables in the prompt
/// 2. Optionally include the previous run's result as context
/// 3. Execute the prompt via AI
/// 4. Save the AI response as a note in the target collection
/// 5. Update subscription run metadata and compute next run time
#[instrument(skip(context))]
pub async fn run_all_due_subscriptions(context: &StorageContext) -> Vec<SubscriptionRunResult> {
    let subs = match list_due_subscriptions_with_context(context) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to list due subscriptions: {e}");
            return vec![];
        }
    };

    if subs.is_empty() {
        tracing::info!("no due subscriptions to run");
        return vec![];
    }

    let mut results = Vec::with_capacity(subs.len());
    for sub in subs {
        tracing::info!(name = %sub.name, id = %sub.id, "running subscription");
        let result = run_single_subscription(context, &sub).await;
        results.push(result);
    }
    results
}

/// Result of a single subscription run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionRunResult {
    pub subscription_id: String,
    pub subscription_name: String,
    pub status: String,
    pub note_id: Option<String>,
    pub note_title: Option<String>,
    pub error: Option<String>,
}

// ─── Placeholder substitution ──────────────────────────────────────

/// Context available for placeholder substitution in subscription prompts.
struct SubstitutionContext {
    /// The subscription's own name.
    subscription_name: String,
    /// The current date/time in a human-readable format.
    now_formatted: String,
    /// The previous run's result note title (empty string if first run).
    last_run_title: String,
    /// The previous run's result note body (truncated, empty if first run).
    last_run_body_preview: String,
}

/// Substitute {{placeholders}} in the prompt template.
///
/// Supported placeholders:
/// - `{{topic}}` — subscription name
/// - `{{date}}` — current date (YYYY-MM-DD)
/// - `{{datetime}}` — current date-time (YYYY-MM-DD HH:MM)
/// - `{{last_run_title}}` — previous result note title (cross-run context)
/// - `{{last_run_body}}` — previous result body preview (cross-run context)
fn substitute_placeholders(template: &str, ctx: &SubstitutionContext) -> String {
    template
        .replace("{{topic}}", &ctx.subscription_name)
        .replace(
            "{{date}}",
            &ctx.now_formatted[..10.min(ctx.now_formatted.len())],
        )
        .replace(
            "{{datetime}}",
            &ctx.now_formatted[..16.min(ctx.now_formatted.len())],
        )
        .replace("{{last_run_title}}", &ctx.last_run_title)
        .replace("{{last_run_body}}", &ctx.last_run_body_preview)
}

/// Compute the next run time from a cron expression.
/// Returns an ISO-8601 formatted string, or None if the expression is invalid.
///
/// The `cron` crate expects a 6 or 7-field expression:
/// `sec min hour dayOfMonth month dayOfWeek [year]`
/// VaultPilot stores standard 5-field cron `min hour dayOfMonth month dayOfWeek`,
/// so we prepend "0 " (seconds=0) to form the 6-field equivalent.
#[cfg(test)]
fn compute_next_run_time_iso(cron_expr: &str) -> Option<String> {
    use std::str::FromStr;

    // Prepend "0 " to convert 5-field standard cron to 6-field cron crate format.
    let full_expr = if cron_expr.split_whitespace().count() == 5 {
        format!("0 {}", cron_expr)
    } else {
        cron_expr.to_string()
    };

    let schedule = cron::Schedule::from_str(&full_expr).ok()?;
    let next = schedule.upcoming(Utc).next()?;
    Some(next.to_rfc3339())
}

// ─── Subscription execution ────────────────────────────────────────

/// Execute a single subscription: prompt AI, save result as note.
///
/// Synchronous storage I/O (SQLite) is wrapped in `tokio::task::spawn_blocking`
/// to avoid blocking the tokio runtime thread pool (#2257).
#[instrument(skip(context, subscription))]
pub async fn run_single_subscription(
    context: &StorageContext,
    subscription: &AiSubscription,
) -> SubscriptionRunResult {
    let id = subscription.id.clone();
    let name = subscription.name.clone();
    let ctx = context.clone();
    let schedule = subscription.schedule.clone();

    // Build substitution context
    let now = Utc::now();
    let now_formatted = now.format("%Y-%m-%d %H:%M").to_string();

    // Load last run note for cross-run context (if any) — spawn_blocking
    let (last_title, last_body) = {
        let ctx = ctx.clone();
        let id = id.clone();
        tokio::task::spawn_blocking(move || load_last_run_context(&ctx, &id))
            .await
            .unwrap_or_default()
    };

    let sub_ctx = SubstitutionContext {
        subscription_name: name.clone(),
        now_formatted,
        last_run_title: last_title,
        last_run_body_preview: last_body,
    };

    // Substitute placeholders in the prompt
    let prompt = substitute_placeholders(&subscription.prompt, &sub_ctx);
    let tools = &subscription.tools;

    // Step 1: Run the prompt via AI (async, already non-blocking)
    let result = execute_subscription_prompt(&ctx, &prompt, tools).await;

    let (status, note_id, note_title, error_msg) = match result {
        Ok(answer) => {
            // Step 2: Save the result as a vault note
            let title = format!("[Scheduled] {} — {}", name, now.format("%Y-%m-%d %H:%M"));

            let meta = NoteMeta {
                title: title.clone(),
                summary: answer.chars().take(300).collect::<String>(),
                source: "ai_subscription".to_string(),
                tags: vec!["scheduled".to_string(), "ai".to_string()],
                collections: vec![subscription.target_collection.clone()],
                ..Default::default()
            };

            let body = format!(
                "# {}\n\n{}\n\n---\n*Generated by AI Scheduled Research — {}*\n*Subscription: {} — {}({})*\n*Prompt: {}*",
                title,
                answer,
                now.to_rfc3339(),
                name,
                id,
                crates_io_display_id(&id),
                prompt
            );

            let note = NoteDocument {
                meta,
                body,
                search_snippet: None,
            };

            // Step 3+4: Synchronous storage I/O wrapped in spawn_blocking
            let save_result = {
                let ctx = ctx.clone();
                let note = note;
                tokio::task::spawn_blocking(move || save_note_with_context(&ctx, note)).await
            };

            match save_result {
                Ok(Ok(saved_doc)) => {
                    // Update subscription run metadata (spawn_blocking)
                    let ctx = ctx.clone();
                    let id = id.clone();
                    let schedule = schedule.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        let _ = update_subscription_run_with_context(&ctx, &id, "success", "");
                        let _ = compute_and_update_next_run(&ctx, &id, &schedule);
                    })
                    .await;
                    (
                        "success".to_string(),
                        Some(saved_doc.meta.id.clone()),
                        Some(saved_doc.meta.title.clone()),
                        None,
                    )
                }
                Ok(Err(e)) => {
                    let err = format!("failed to save note: {e}");
                    tracing::error!(id = %id, error = %err, "subscription save failed");
                    let ctx = ctx.clone();
                    let id = id.clone();
                    let err_clone = err.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        let _ =
                            update_subscription_run_with_context(&ctx, &id, "failed", &err_clone);
                    })
                    .await;
                    ("failed".to_string(), None, None, Some(err))
                }
                Err(e) => {
                    let err = format!("spawn_blocking join error: {e}");
                    tracing::error!(id = %id, error = %err, "subscription spawn_blocking failed");
                    let ctx = ctx.clone();
                    let id = id.clone();
                    let err_clone = err.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        let _ =
                            update_subscription_run_with_context(&ctx, &id, "failed", &err_clone);
                    })
                    .await;
                    ("failed".to_string(), None, None, Some(err))
                }
            }
        }
        Err(e) => {
            let err = format!("{e:#}");
            tracing::error!(id = %id, error = %err, "subscription execution failed");
            let ctx = ctx.clone();
            let id = id.clone();
            let err_clone = err.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = update_subscription_run_with_context(&ctx, &id, "failed", &err_clone);
            })
            .await;
            ("failed".to_string(), None, None, Some(err))
        }
    };

    SubscriptionRunResult {
        subscription_id: id,
        subscription_name: name,
        status,
        note_id,
        note_title,
        error: error_msg,
    }
}

/// Load the last successful run's note title and body preview for cross-run context.
fn load_last_run_context(context: &StorageContext, sub_id: &str) -> (String, String) {
    match crate::storage::subscriptions::get_last_successful_run_note(context, sub_id) {
        Ok(Some(note)) => {
            let preview: String = note.body.chars().take(500).collect();
            (note.meta.title, preview)
        }
        _ => (String::new(), String::new()),
    }
}

/// Execute the AI prompt for a subscription.
///
/// Uses ask_with_ai_with_context for rich grounding with vault context.
/// For tool-allowed subscriptions, we inject the available tools info.
async fn execute_subscription_prompt(
    context: &StorageContext,
    prompt: &str,
    tools: &str,
) -> Result<String, anyhow::Error> {
    // Build a system-like instruction that wraps the subscription prompt.
    let mut effective_prompt = format!(
        r#"[Scheduled Research Task]
You are running a recurring research subscription.
Execute the following prompt thoroughly and provide a well-structured response.

RESEARCH PROMPT:
{}

Please provide a comprehensive response with:
- Key findings
- Sources/references where applicable
- Timestamp of this research"#,
        prompt
    );

    // If tools are allowed, add a note about available capabilities
    if !tools.is_empty() {
        effective_prompt.push_str(&format!(
            "\n\nAvailable tools: {}
(These tools will be used automatically if needed.)",
            tools
        ));
    }

    // Use the existing ask API for grounded answers
    let answer = crate::ask_with_ai_with_context(
        context,
        effective_prompt,
        None,      // no history
        None,      // no images
        None,      // no model override
        |_, _| {}, // suppress status events
    )
    .await?;

    Ok(answer.answer)
}

/// Helper: truncate a UUID to its first 8 chars for display.
fn crates_io_display_id(id: &str) -> String {
    if id.len() > 8 {
        id[..8].to_string()
    } else {
        id.to_string()
    }
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{
        initialize_storage_with_context, subscriptions::create_subscription_with_context,
        StorageContext,
    };
    use chrono::{DateTime, Timelike, Utc};
    use std::path::PathBuf;

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-sr-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    #[test]
    fn test_compute_next_run_time_iso_valid() {
        // "0 9 * * 1" = every Monday at 09:00 UTC
        let result = compute_next_run_time_iso("0 9 * * 1").unwrap();
        // Should be a valid RFC3339 timestamp
        assert!(result.contains('T'), "expected RFC3339 date, got: {result}");
        // Parse it
        let parsed: DateTime<Utc> = result.parse().expect("valid RFC3339 timestamp");
        assert_eq!(parsed.hour(), 9);
        assert_eq!(parsed.minute(), 0);
        // Should be in the future
        assert!(parsed > Utc::now() - chrono::Duration::minutes(5));
    }

    #[test]
    fn test_compute_next_run_time_iso_every_hour() {
        let result = compute_next_run_time_iso("0 * * * *").unwrap();
        let parsed: DateTime<Utc> = result.parse().unwrap();
        assert_eq!(parsed.minute(), 0);
        assert!(parsed > Utc::now() - chrono::Duration::minutes(5));
    }

    #[test]
    fn test_compute_next_run_time_iso_invalid() {
        assert!(compute_next_run_time_iso("invalid cron").is_none());
        assert!(compute_next_run_time_iso("").is_none());
    }

    #[test]
    fn test_substitute_placeholders_all() {
        let template = "Research {{topic}} on {{date}} at {{datetime}}.
Last time we found: {{last_run_title}} — {{last_run_body}}";

        let ctx = SubstitutionContext {
            subscription_name: "AI News".to_string(),
            now_formatted: "2026-06-30 14:30".to_string(),
            last_run_title: "Previous Results".to_string(),
            last_run_body_preview: "Key insights from last week...".to_string(),
        };

        let result = substitute_placeholders(template, &ctx);
        assert!(result.contains("AI News"), "topic not substituted");
        assert!(result.contains("2026-06-30"), "date not substituted");
        assert!(result.contains("14:30"), "datetime not substituted");
        assert!(
            result.contains("Previous Results"),
            "last_run_title not substituted"
        );
        assert!(
            result.contains("Key insights from last week"),
            "last_run_body not substituted"
        );
    }

    #[test]
    fn test_substitute_placeholders_empty_context() {
        let template = "Research {{topic}}";
        let ctx = SubstitutionContext {
            subscription_name: "Test".to_string(),
            now_formatted: "2026-06-30 12:00".to_string(),
            last_run_title: String::new(),
            last_run_body_preview: String::new(),
        };

        let result = substitute_placeholders(template, &ctx);
        assert_eq!(result, "Research Test");
    }

    #[test]
    fn test_substitute_placeholders_no_placeholders() {
        let template = "Just a plain prompt without variables";
        let ctx = SubstitutionContext {
            subscription_name: "Test".to_string(),
            now_formatted: "2026-06-30 12:00".to_string(),
            last_run_title: String::new(),
            last_run_body_preview: String::new(),
        };

        let result = substitute_placeholders(template, &ctx);
        assert_eq!(result, template);
    }

    #[test]
    fn test_run_subscription_updates_metadata() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        // Create a subscription
        let sub = create_subscription_with_context(
            &ctx,
            "Test Run",
            "0 0 * * *",
            "test prompt that will fail without AI config",
            "web_search",
            "Test Collection",
        )
        .unwrap();

        // Run it — without valid AI config, it will fail, but metadata should update
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_single_subscription(&ctx, &sub));

        assert_eq!(result.subscription_id, sub.id);
        assert_eq!(result.subscription_name, "Test Run");
        // Without AI configured, it should fail gracefully
        assert_eq!(result.status, "failed");
        assert!(result.error.is_some());

        // Verify metadata was updated
        let updated = crate::storage::subscriptions::get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.run_count, 1);
        assert_eq!(updated.last_status, "failed");
        assert!(!updated.last_error.is_empty());
    }

    #[test]
    fn test_run_all_due_empty() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let results = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_all_due_subscriptions(&ctx));
        assert!(results.is_empty());
    }

    #[test]
    fn test_run_all_due_skips_disabled() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        // Create a disabled subscription
        let sub =
            create_subscription_with_context(&ctx, "Disabled Test", "0 0 * * *", "test", "", "")
                .unwrap();
        crate::storage::subscriptions::set_subscription_enabled_with_context(&ctx, &sub.id, false)
            .unwrap();

        let results = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_all_due_subscriptions(&ctx));
        // Disabled subscriptions are not returned by list_due
        assert!(results.is_empty());
    }
}
