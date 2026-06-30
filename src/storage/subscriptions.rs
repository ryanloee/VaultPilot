//! Subscription CRUD — AI Scheduled Research subscription persistence (#2167).
//!
//! Subscriptions are recurring AI-powered research tasks stored in the
//! `subscriptions` SQLite table.

#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use tracing::instrument;
use uuid::Uuid;

use crate::models::AiSubscription;

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
    let rows = connection.execute(
        r#"
        UPDATE subscriptions
        SET name = ?1, schedule = ?2, prompt = ?3,
            tools = ?4, target_collection = ?5,
            updated_at = ?6
        WHERE id = ?7
        "#,
        params![name, schedule, prompt, tools, target_collection, Utc::now().to_rfc3339(), id],
    )?;
    Ok(rows > 0)
}

/// List all enabled subscriptions that are due for execution.
/// Currently returns all enabled subscriptions (simplified MVP — no cron-based
/// scheduling yet, just manual `run` command).
#[instrument(skip(context))]
pub fn list_due_subscriptions_with_context(
    context: &StorageContext,
) -> Result<Vec<AiSubscription>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        r#"
        SELECT id, name, schedule, prompt, tools, target_collection,
               enabled, last_run_at, next_run_at, created_at, updated_at,
               run_count, last_status, last_error
        FROM subscriptions
        WHERE enabled = 1
        ORDER BY created_at ASC
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
        update_subscription_with_context(&ctx, &id, &name, &schedule, &prompt, &tools, &target_collection)
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
    tokio::task::spawn_blocking(move || {
        set_subscription_enabled_with_context(&ctx, &id, enabled)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
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
        assert!(updated.enabled);
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
}
