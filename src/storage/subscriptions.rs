//! Subscription CRUD — AI Scheduled Research subscription persistence (#2167).
//!
//! Subscriptions are recurring AI-powered research tasks stored in the
//! `subscriptions` SQLite table.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use tracing::instrument;
use uuid::Uuid;

use crate::models::{AiSubscription, NoteDocument, NoteMeta};

use super::pool::open_connection;
use super::StorageContext;

// ────────────────────────────────────────────────────────
// Subscription CRUD
// ────────────────────────────────────────────────────────

/// Create a new subscription. Returns the created subscription.
#[instrument(skip(context))]
pub fn create_subscription_with_context(
    context: &StorageContext,
    name: &str,
    schedule: &str,
    prompt: &str,
    tools: &str,
    target_collection: &str,
) -> Result<AiSubscription> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();

    connection.execute(
        "INSERT INTO subscriptions (id, name, schedule, prompt, tools, target_collection, enabled, last_run_at, next_run_at, created_at, updated_at, run_count, last_status, last_error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, '', '', ?7, ?8, 0, '', '')",
        params![id, name, schedule, prompt, tools, target_collection, now, now],
    ).with_context(|| format!("failed to create subscription '{name}'"))?;

    Ok(AiSubscription {
        id,
        name: name.to_string(),
        schedule: schedule.to_string(),
        prompt: prompt.to_string(),
        tools: tools.to_string(),
        target_collection: target_collection.to_string(),
        enabled: true,
        last_run_at: String::new(),
        next_run_at: String::new(),
        created_at: now.clone(),
        updated_at: now,
        run_count: 0,
        last_status: String::new(),
        last_error: String::new(),
    })
}

/// Delete a subscription by ID. Returns `true` if a row was deleted.
#[instrument(skip(context))]
pub fn delete_subscription_with_context(context: &StorageContext, id: &str) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let rows = connection
        .execute("DELETE FROM subscriptions WHERE id = ?1", params![id])
        .with_context(|| format!("failed to delete subscription '{id}'"))?;
    Ok(rows > 0)
}

/// List all subscriptions.
#[instrument(skip(context))]
pub fn list_subscriptions_with_context(context: &StorageContext) -> Result<Vec<AiSubscription>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        r#"
        SELECT id, name, schedule, prompt, tools, target_collection,
               enabled, last_run_at, next_run_at, created_at, updated_at,
               run_count, last_status, last_error
        FROM subscriptions
        ORDER BY created_at DESC
        "#,
    )?;

    let subscriptions = stmt
        .query_map([], |row| {
            Ok(AiSubscription {
                id: row.get(0)?,
                name: row.get(1)?,
                schedule: row.get(2)?,
                prompt: row.get(3)?,
                tools: row.get(4)?,
                target_collection: row.get(5)?,
                enabled: row.get::<_, i64>(6)? != 0,
                last_run_at: row.get(7)?,
                next_run_at: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                run_count: row.get(11)?,
                last_status: row.get(12)?,
                last_error: row.get(13)?,
            })
        })
        .with_context(|| "failed to query subscriptions")?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| "failed to collect subscriptions")?;

    Ok(subscriptions)
}

/// Get a single subscription by ID.
#[instrument(skip(context))]
pub fn get_subscription_with_context(
    context: &StorageContext,
    id: &str,
) -> Result<Option<AiSubscription>> {
    let (connection, _) = open_connection(context)?;

    let result = connection
        .query_row(
            r#"
            SELECT id, name, schedule, prompt, tools, target_collection,
                   enabled, last_run_at, next_run_at, created_at, updated_at,
                   run_count, last_status, last_error
            FROM subscriptions
            WHERE id = ?1
            "#,
            params![id],
            |row| {
                Ok(AiSubscription {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    schedule: row.get(2)?,
                    prompt: row.get(3)?,
                    tools: row.get(4)?,
                    target_collection: row.get(5)?,
                    enabled: row.get::<_, i64>(6)? != 0,
                    last_run_at: row.get(7)?,
                    next_run_at: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                    run_count: row.get(11)?,
                    last_status: row.get(12)?,
                    last_error: row.get(13)?,
                })
            },
        )
        .optional()
        .with_context(|| "failed to query subscription")?;

    Ok(result)
}

/// Update a subscription's run metadata after execution.
#[instrument(skip(context))]
pub fn update_subscription_run_with_context(
    context: &StorageContext,
    id: &str,
    status: &str,
    error_msg: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();

    let rows = connection.execute(
        r#"
        UPDATE subscriptions
        SET last_run_at = ?1,
            updated_at = ?1,
            run_count = run_count + 1,
            last_status = ?2,
            last_error = ?3
        WHERE id = ?4
        "#,
        params![now, status, error_msg, id],
    )?;

    Ok(rows > 0)
}

/// Enable or disable a subscription.
#[instrument(skip(context))]
pub fn set_subscription_enabled_with_context(
    context: &StorageContext,
    id: &str,
    enabled: bool,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let rows = connection
        .execute(
            "UPDATE subscriptions SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![enabled as i64, Utc::now().to_rfc3339(), id],
        )
        .with_context(|| format!("failed to update subscription '{id}'"))?;
    Ok(rows > 0)
}

/// Update subscription editable fields (name, schedule, prompt, tools, target_collection).
/// Update all mutable fields of an existing subscription by ID.
/// Returns `true` if a row was updated.
///
/// When the schedule changes, `last_status` and `next_run_at` are reset so the
/// scheduler re-evaluates the new cron expression on the next cycle instead of
/// permanently excluding an error-parked subscription (#2852).
#[instrument(skip(context))]
pub fn update_subscription_with_context(
    context: &StorageContext,
    id: &str,
    name: &str,
    schedule: &str,
    prompt: &str,
    tools: &str,
    target_collection: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();

    // Fetch the current schedule so we only reset error-park state when the
    // cron expression actually changed.
    let old_schedule: Option<String> = connection
        .query_row(
            "SELECT schedule FROM subscriptions WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;

    let schedule_changed = old_schedule.as_deref() != Some(schedule);

    let rows = if schedule_changed {
        connection
            .execute(
                r#"
            UPDATE subscriptions
            SET name = ?1, schedule = ?2, prompt = ?3,
                tools = ?4, target_collection = ?5,
                last_status = '', last_error = '', next_run_at = '',
                updated_at = ?6
            WHERE id = ?7
            "#,
                params![name, schedule, prompt, tools, target_collection, now, id],
            )
            .with_context(|| format!("failed to update subscription '{id}'"))?
    } else {
        connection
            .execute(
                "UPDATE subscriptions SET name = ?1, schedule = ?2, prompt = ?3, tools = ?4, target_collection = ?5, updated_at = ?6 WHERE id = ?7",
                params![name, schedule, prompt, tools, target_collection, now, id],
            )
            .with_context(|| format!("failed to update subscription '{id}'"))?
    };
    Ok(rows > 0)
}

/// List all enabled subscriptions that are due for execution.
///
/// A subscription is "due" when:
/// - It is enabled, AND
/// - Its `next_run_at` is empty (never run) OR `next_run_at <= now`, AND
/// - Its `last_status` is not `'error'` (parked subscriptions with invalid
///   cron expressions are skipped to avoid wasting LLM calls every 365 days).
///
/// This enables cron-based scheduling: after each run, `next_run_at` is
/// updated to the next future time, so the subscription won't be due again
/// until that time arrives. Error-parked subscriptions are excluded so the
/// scheduler does not re-evaluate a known-bad cron expression each cycle
/// (#2852).
#[instrument(skip(context))]
pub fn list_due_subscriptions_with_context(
    context: &StorageContext,
) -> Result<Vec<AiSubscription>> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();

    let mut stmt = connection.prepare(
        r#"
        SELECT id, name, schedule, prompt, tools, target_collection,
               enabled, last_run_at, next_run_at, created_at, updated_at,
               run_count, last_status, last_error
        FROM subscriptions
        WHERE enabled = 1
          AND (next_run_at = '' OR next_run_at <= ?1)
          AND COALESCE(last_status, '') != 'error'
        ORDER BY created_at ASC
        "#,
    )?;

    let subscriptions = stmt
        .query_map(params![now], |row| {
            Ok(AiSubscription {
                id: row.get(0)?,
                name: row.get(1)?,
                schedule: row.get(2)?,
                prompt: row.get(3)?,
                tools: row.get(4)?,
                target_collection: row.get(5)?,
                enabled: row.get::<_, i64>(6)? != 0,
                last_run_at: row.get(7)?,
                next_run_at: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                run_count: row.get(11)?,
                last_status: row.get(12)?,
                last_error: row.get(13)?,
            })
        })
        .with_context(|| "failed to query due subscriptions")?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| "failed to collect due subscriptions")?;

    Ok(subscriptions)
}

// ────────────────────────────────────────────────────────
// Async wrappers
// ────────────────────────────────────────────────────────

pub async fn create_subscription_async(
    ctx: &StorageContext,
    name: String,
    schedule: String,
    prompt: String,
    tools: String,
    target_collection: String,
) -> Result<AiSubscription> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        create_subscription_with_context(
            &ctx,
            &name,
            &schedule,
            &prompt,
            &tools,
            &target_collection,
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn delete_subscription_async(ctx: &StorageContext, id: String) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || delete_subscription_with_context(&ctx, &id))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn list_subscriptions_async(ctx: &StorageContext) -> Result<Vec<AiSubscription>> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || list_subscriptions_with_context(&ctx))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn get_subscription_async(
    ctx: &StorageContext,
    id: String,
) -> Result<Option<AiSubscription>> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || get_subscription_with_context(&ctx, &id))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn update_subscription_async(
    ctx: &StorageContext,
    id: String,
    name: String,
    schedule: String,
    prompt: String,
    tools: String,
    target_collection: String,
) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        update_subscription_with_context(
            &ctx,
            &id,
            &name,
            &schedule,
            &prompt,
            &tools,
            &target_collection,
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

pub async fn set_subscription_enabled_async(
    ctx: &StorageContext,
    id: String,
    enabled: bool,
) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || set_subscription_enabled_with_context(&ctx, &id, enabled))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

// ────────────────────────────────────────────────────────
// Scheduling helpers
// ────────────────────────────────────────────────────────

/// Update a subscription's `next_run_at` field (after a run completes).
/// This stores the value; the caller is responsible for computing it.
#[instrument(skip(context))]
pub fn update_subscription_next_run_with_context(
    context: &StorageContext,
    id: &str,
    next_run_at: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();

    let rows = connection.execute(
        r#"
        UPDATE subscriptions
        SET next_run_at = ?1,
            updated_at = ?2
        WHERE id = ?3
        "#,
        params![next_run_at, now, id],
    )?;

    Ok(rows > 0)
}

/// Compute the next run time from a cron expression and update the subscription.
/// Returns the computed ISO-8601 timestamp string, or an error string if parsing fails.
///
/// This function is called after a subscription run completes to schedule the next one.
/// If the cron expression is invalid, next_run_at is left as-is (or set to empty)
/// and a warning is logged.
#[instrument(skip(context))]
pub fn compute_and_update_next_run(
    context: &StorageContext,
    id: &str,
    cron_expr: &str,
) -> Result<String> {
    use std::str::FromStr;

    let full_expr = if cron_expr.split_whitespace().count() == 5 {
        format!("0 {}", cron_expr)
    } else {
        cron_expr.to_string()
    };

    match cron::Schedule::from_str(&full_expr) {
        Ok(schedule) => match schedule.upcoming(Utc).next() {
            Some(dt) => {
                let next_iso = dt.to_rfc3339();
                update_subscription_next_run_with_context(context, id, &next_iso)?;
                Ok(next_iso)
            }
            None => {
                // No future trigger times (e.g. a past-only expression such as
                // "Feb 30"). Park the subscription far in the future and record
                // an error so `list_due_subscriptions_with_context` does not
                // re-select it on every scheduling cycle (its SQL treats an
                // empty `next_run_at` as always due). See issue #2843.
                tracing::warn!(id = %id, expr = %cron_expr, "cron schedule produced no future times");
                let parked = park_subscription_with_error(
                    context,
                    id,
                    "cron expression produces no future trigger times",
                )?;
                Ok(parked)
            }
        },
        Err(e) => {
            // Invalid cron: do NOT leave `next_run_at` empty (that would make the
            // subscription perpetually "due" and re-execute every cycle). Park it
            // and record the parse error instead. See issue #2843.
            tracing::warn!(id = %id, expr = %cron_expr, error = %e, "invalid cron expression");
            let parked = park_subscription_with_error(
                context,
                id,
                &format!("invalid cron expression: {e}"),
            )?;
            Ok(parked)
        }
    }
}

/// Park a subscription that can no longer be scheduled (invalid or exhausted
/// cron expression) by setting `next_run_at` to a far-future timestamp and
/// recording an error status. This prevents
/// `list_due_subscriptions_with_context` from re-selecting it every cycle:
/// its SQL condition `next_run_at = '' OR next_run_at <= ?1` would otherwise
/// treat an empty `next_run_at` as always due, causing infinite re-execution
/// (#2843). A non-empty far-future value defeats that condition until the user
/// fixes the expression.
fn park_subscription_with_error(
    context: &StorageContext,
    id: &str,
    error_msg: &str,
) -> Result<String> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now();
    let parked_iso = (now + chrono::Duration::days(365)).to_rfc3339();
    connection
        .execute(
            r#"
            UPDATE subscriptions
            SET next_run_at = ?1,
                last_status = 'error',
                last_error = ?2,
                updated_at = ?3
            WHERE id = ?4
            "#,
            params![parked_iso, error_msg, now.to_rfc3339(), id],
        )
        .with_context(|| format!("failed to park subscription '{id}'"))?;
    Ok(parked_iso)
}

/// Get the last successful run's result note for cross-run context.
///
/// Searches for the most recent note in the subscription's target collection
/// with source = "ai_subscription" and tags containing "scheduled".
/// Returns the note document, or None if no previous run exists.
#[instrument(skip(context))]
pub fn get_last_successful_run_note(
    context: &StorageContext,
    subscription_id: &str,
) -> Result<Option<NoteDocument>> {
    let (connection, _) = open_connection(context)?;

    // Find the most recent note generated by this subscription.
    // We match on source="ai_subscription" and a body containing the subscription ID
    // as a reliable way to associate notes with their subscription.
    let result = connection
        .query_row(
            r#"
            SELECT n.id, n.path, n.title, n.summary, n.source,
                   n.created_at, n.updated_at, n.body,
                   n.tags, n.keywords, n.platform, n.board, n.kernel, n.status
            FROM notes n
            WHERE n.source = 'ai_subscription'
              AND n.body LIKE ?1
            ORDER BY n.created_at DESC
            LIMIT 1
            "#,
            params![format!("%{}%", subscription_id)],
            |row| {
                let tags_str: String = row.get(8).unwrap_or_default();
                let keywords_str: String = row.get(9).unwrap_or_default();
                let _collections_str: String = String::new(); // not stored per-note in this query

                Ok(NoteDocument {
                    meta: NoteMeta {
                        id: row.get(0)?,
                        path: row.get(1)?,
                        title: row.get(2)?,
                        summary: row.get(3)?,
                        source: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                        tags: tags_str
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect(),
                        keywords: keywords_str
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect(),
                        ..Default::default()
                    },
                    body: row.get::<_, String>(7)?,
                    search_snippet: None,
                    search_score: None,
                })
            },
        )
        .optional()?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{initialize_storage_with_context, StorageContext};
    use chrono::Utc;
    use std::path::PathBuf;

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-subscriptions-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    #[test]
    fn test_create_and_list_subscriptions() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub = create_subscription_with_context(
            &ctx,
            "Weekly News",
            "0 9 * * 1",
            "Summarize {{topic}} key dynamics",
            "web_search",
            "Market Monitor",
        )
        .unwrap();
        assert_eq!(sub.name, "Weekly News");
        assert!(!sub.id.is_empty());
        assert!(sub.enabled);

        let subs = list_subscriptions_with_context(&ctx).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "Weekly News");
        assert_eq!(subs[0].target_collection, "Market Monitor");
    }

    #[test]
    fn test_delete_subscription() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub = create_subscription_with_context(&ctx, "Delete Me", "0 0 * * *", "test", "", "")
            .unwrap();
        assert!(delete_subscription_with_context(&ctx, &sub.id).unwrap());
        assert!(!delete_subscription_with_context(&ctx, "nonexistent").unwrap());
        assert!(list_subscriptions_with_context(&ctx).unwrap().is_empty());
    }

    #[test]
    fn test_get_subscription() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub = create_subscription_with_context(
            &ctx,
            "Get Test",
            "*/30 * * * *",
            "Find latest on {{topic}}",
            "web_search",
            "Research",
        )
        .unwrap();

        let found = get_subscription_with_context(&ctx, &sub.id).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().prompt, "Find latest on {{topic}}");

        let not_found = get_subscription_with_context(&ctx, "bogus-id").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_update_subscription_run() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub =
            create_subscription_with_context(&ctx, "Run Test", "0 0 * * *", "test prompt", "", "")
                .unwrap();

        assert!(update_subscription_run_with_context(&ctx, &sub.id, "success", "").unwrap());
        let updated = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.run_count, 1);
        assert_eq!(updated.last_status, "success");

        assert!(update_subscription_run_with_context(&ctx, &sub.id, "failed", "timeout").unwrap());
        let updated2 = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated2.run_count, 2);
        assert_eq!(updated2.last_status, "failed");
        assert_eq!(updated2.last_error, "timeout");
    }

    #[test]
    fn test_compute_next_run_invalid_cron_parked_not_due() {
        // Regression test for #2843: an invalid cron expression must NOT leave
        // `next_run_at` empty (which the `list_due` SQL treats as always due,
        // causing infinite re-execution). Instead it should be parked with a
        // far-future timestamp and an error status.
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub =
            create_subscription_with_context(&ctx, "Bad Cron", "0 9 * * 1", "test prompt", "", "")
                .unwrap();

        let next = compute_and_update_next_run(&ctx, &sub.id, "not a cron").unwrap();
        assert!(!next.is_empty(), "next_run_at must not be empty");
        // Parked value must be strictly in the future so list_due won't select it.
        assert!(
            next > Utc::now().to_rfc3339(),
            "parked next_run_at must be in the future, got {next}"
        );

        let updated = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.last_status, "error");
        assert!(
            updated.last_error.contains("invalid cron expression"),
            "last_error should record the cron parse failure, got '{}'",
            updated.last_error
        );

        // list_due must NOT return the parked subscription.
        let due = list_due_subscriptions_with_context(&ctx).unwrap();
        assert!(
            !due.iter().any(|s| s.id == sub.id),
            "parked subscription must not be returned as due"
        );
    }

    #[test]
    fn test_compute_next_run_no_future_times_parked() {
        // Regression test for #2843: a cron expression that parses but yields
        // no future trigger times (e.g. Feb 30) must also be parked, not left
        // empty.
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub = create_subscription_with_context(
            &ctx,
            "Exhausted Cron",
            "0 0 30 2 *",
            "test prompt",
            "",
            "",
        )
        .unwrap();

        let next = compute_and_update_next_run(&ctx, &sub.id, "0 0 30 2 *").unwrap();
        assert!(!next.is_empty());
        assert!(next > Utc::now().to_rfc3339());

        let updated = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.last_status, "error");
        assert!(
            updated.last_error.contains("no future trigger times"),
            "got '{}'",
            updated.last_error
        );
    }

    #[test]
    fn test_set_subscription_enabled() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub =
            create_subscription_with_context(&ctx, "Toggle Test", "0 0 * * *", "test", "", "")
                .unwrap();
        assert!(sub.enabled);

        assert!(set_subscription_enabled_with_context(&ctx, &sub.id, false).unwrap());
        let updated = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert!(!updated.enabled);

        assert!(set_subscription_enabled_with_context(&ctx, &sub.id, true).unwrap());
        let updated2 = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert!(updated2.enabled);
    }

    #[test]
    fn test_list_due_subscriptions_only_returns_enabled() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub1 = create_subscription_with_context(&ctx, "Enabled1", "0 0 * * *", "test1", "", "")
            .unwrap();
        let sub2 = create_subscription_with_context(&ctx, "Disabled", "0 0 * * *", "test2", "", "")
            .unwrap();
        set_subscription_enabled_with_context(&ctx, &sub2.id, false).unwrap();
        let _sub3 =
            create_subscription_with_context(&ctx, "Enabled2", "0 0 * * *", "test3", "", "")
                .unwrap();

        let due = list_due_subscriptions_with_context(&ctx).unwrap();
        assert_eq!(due.len(), 2);
        assert!(due.iter().any(|s| s.id == sub1.id));
        assert!(due.iter().all(|s| s.enabled));
    }

    #[test]
    fn test_update_subscription_fields() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub = create_subscription_with_context(
            &ctx,
            "Original Name",
            "0 0 * * *",
            "original prompt",
            "web_search",
            "Research",
        )
        .unwrap();

        // Update all editable fields
        assert!(update_subscription_with_context(
            &ctx,
            &sub.id,
            "Updated Name",
            "0 9 * * 1",
            "updated prompt with {{topic}}",
            "web_search,read_note",
            "Market Monitor",
        )
        .unwrap());

        let updated = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "Updated Name");
        assert_eq!(updated.schedule, "0 9 * * 1");
        assert_eq!(updated.prompt, "updated prompt with {{topic}}");
        assert_eq!(updated.tools, "web_search,read_note");
        assert_eq!(updated.target_collection, "Market Monitor");
        // Fields not updated should remain
        assert_eq!(updated.run_count, 0);
    }

    #[test]
    fn test_update_subscription_nonexistent() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let result = update_subscription_with_context(
            &ctx,
            "nonexistent-id",
            "Test",
            "0 0 * * *",
            "test",
            "",
            "",
        )
        .unwrap();
        assert!(!result);
    }

    // ── Regression tests for #2852 ────────────────────────────────

    #[test]
    fn test_parked_error_subscription_not_due_after_365_days() {
        // Regression test for #2852: even when the parked subscription's 365-day
        // timer expires, it must NOT be re-selected by list_due because the
        // `COALESCE(last_status, '') != 'error'` filter prevents it.
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub = create_subscription_with_context(&ctx, "Bad Cron", "0 9 * * 1", "test", "", "")
            .unwrap();
        // Park it with an invalid cron expression.
        compute_and_update_next_run(&ctx, &sub.id, "not a cron").unwrap();

        // Verify it's parked.
        let updated = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.last_status, "error");

        // Simulate 365 days passing: set next_run_at to a past timestamp.
        {
            let (conn, _) = open_connection(&ctx).unwrap();
            conn.execute(
                "UPDATE subscriptions SET next_run_at = ?1 WHERE id = ?2",
                params!["2020-01-01T00:00:00Z", &sub.id],
            )
            .unwrap();
        }

        // list_due must NOT return it despite next_run_at being in the past.
        let due = list_due_subscriptions_with_context(&ctx).unwrap();
        assert!(
            !due.iter().any(|s| s.id == sub.id),
            "error-parked subscription must not be re-selected even after 365-day expiry (#2852)"
        );
    }

    #[test]
    fn test_update_schedule_resets_error_status() {
        // Regression test for #2852: fixing the cron expression via
        // `update_subscription_with_context` should reset `last_status` and
        // `next_run_at` so the subscription is picked up by the scheduler again.
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub = create_subscription_with_context(&ctx, "Bad Cron", "0 9 * * 1", "test", "", "")
            .unwrap();
        // Park it.
        compute_and_update_next_run(&ctx, &sub.id, "not a cron").unwrap();

        let parked = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(parked.last_status, "error");
        assert!(!parked.next_run_at.is_empty());

        // Fix the cron via update (change the schedule).
        update_subscription_with_context(&ctx, &sub.id, "Fixed Cron", "0 0 * * *", "test", "", "")
            .unwrap();

        let fixed = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            fixed.last_status, "",
            "last_status should be reset when schedule changes"
        );
        assert_eq!(
            fixed.next_run_at, "",
            "next_run_at should be reset when schedule changes"
        );

        // Now it should be picked up by list_due again.
        let due = list_due_subscriptions_with_context(&ctx).unwrap();
        assert!(
            due.iter().any(|s| s.id == sub.id),
            "fixed subscription should be due again after schedule change"
        );
    }

    #[test]
    fn test_update_same_schedule_does_not_reset_status() {
        // Regression test for #2852: updating with the same schedule must NOT
        // reset last_status — only an actual schedule change should.
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let sub =
            create_subscription_with_context(&ctx, "Sub", "0 9 * * 1", "test", "", "").unwrap();
        // Park it.
        compute_and_update_next_run(&ctx, &sub.id, "not a cron").unwrap();
        assert_eq!(
            get_subscription_with_context(&ctx, &sub.id)
                .unwrap()
                .unwrap()
                .last_status,
            "error"
        );

        // Update with the SAME schedule but different name.
        update_subscription_with_context(&ctx, &sub.id, "New Name", "0 9 * * 1", "test", "", "")
            .unwrap();

        let updated = get_subscription_with_context(&ctx, &sub.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            updated.last_status, "error",
            "same schedule should NOT reset error status"
        );
        assert_eq!(
            updated.name, "New Name",
            "non-schedule fields should be updated"
        );
    }
}
