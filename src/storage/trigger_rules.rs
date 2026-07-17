//! Trigger Rule CRUD — Agent trigger rule persistence (#2984).
//!
//! Trigger rules are user-definable rules stored in the `trigger_rules` SQLite
//! table. Each rule specifies a trigger (cron schedule or vault event) and an
//! action for the agent to perform when the trigger fires.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use tracing::instrument;
use uuid::Uuid;

use crate::orchestration::trigger::{AgentTriggerRule, TriggerAction, TriggerKind};

use super::pool::open_connection;
use super::StorageContext;

// ────────────────────────────────────────────────────────
// Trigger Rule CRUD
// ────────────────────────────────────────────────────────

/// Create a new trigger rule. Returns the created rule.
#[instrument(skip(context))]
pub fn create_trigger_rule_with_context(
    context: &StorageContext,
    label: &str,
    trigger_type: &str,   // "cron" or "event"
    trigger_config: &str, // cron expression or event name
    action: &str,         // TriggerAction variant name
    filter: Option<&str>, // optional tag/content filter for event triggers
    custom_prompt: Option<&str>,
) -> Result<AgentTriggerRule> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    let enabled: i32 = 1;

    connection.execute(
        "INSERT INTO trigger_rules (id, label, trigger_type, trigger_config, filter, action, enabled, custom_prompt, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![id, label, trigger_type, trigger_config, filter, action, enabled, custom_prompt, now, now],
    ).with_context(|| format!("failed to create trigger rule '{label}'"))?;

    Ok(build_rule(
        &id,
        label,
        trigger_type,
        trigger_config,
        filter,
        action,
        enabled != 0,
        custom_prompt,
        &now,
        &now,
    ))
}

/// List all trigger rules.
#[instrument(skip(context))]
pub fn list_trigger_rules_with_context(context: &StorageContext) -> Result<Vec<AgentTriggerRule>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        r#"
        SELECT id, label, trigger_type, trigger_config, filter, action, enabled, custom_prompt, created_at, updated_at
        FROM trigger_rules
        ORDER BY created_at DESC
        "#,
    )?;

    let rules = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let label: String = row.get(1)?;
            let trigger_type: String = row.get(2)?;
            let trigger_config: String = row.get(3)?;
            let filter: Option<String> = row.get(4)?;
            let action: String = row.get(5)?;
            let enabled: i32 = row.get(6)?;
            let custom_prompt: Option<String> = row.get(7)?;
            let created_at: String = row.get(8)?;
            let updated_at: String = row.get(9)?;
            Ok(build_rule(
                &id,
                &label,
                &trigger_type,
                &trigger_config,
                filter.as_deref(),
                &action,
                enabled != 0,
                custom_prompt.as_deref(),
                &created_at,
                &updated_at,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to collect trigger rules")?;

    Ok(rules)
}

/// Get a single trigger rule by ID.
#[instrument(skip(context))]
pub fn get_trigger_rule_with_context(
    context: &StorageContext,
    id: &str,
) -> Result<Option<AgentTriggerRule>> {
    let (connection, _) = open_connection(context)?;

    let mut stmt = connection.prepare(
        r#"
        SELECT id, label, trigger_type, trigger_config, filter, action, enabled, custom_prompt, created_at, updated_at
        FROM trigger_rules
        WHERE id = ?1
        "#,
    )?;

    let rule = stmt
        .query_row(params![id], |row| {
            let id: String = row.get(0)?;
            let label: String = row.get(1)?;
            let trigger_type: String = row.get(2)?;
            let trigger_config: String = row.get(3)?;
            let filter: Option<String> = row.get(4)?;
            let action: String = row.get(5)?;
            let enabled: i32 = row.get(6)?;
            let custom_prompt: Option<String> = row.get(7)?;
            let created_at: String = row.get(8)?;
            let updated_at: String = row.get(9)?;
            Ok(build_rule(
                &id,
                &label,
                &trigger_type,
                &trigger_config,
                filter.as_deref(),
                &action,
                enabled != 0,
                custom_prompt.as_deref(),
                &created_at,
                &updated_at,
            ))
        })
        .optional()?;

    Ok(rule)
}

/// Delete a trigger rule by ID. Returns `true` if a row was deleted.
#[instrument(skip(context))]
pub fn delete_trigger_rule_with_context(context: &StorageContext, id: &str) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let rows = connection
        .execute("DELETE FROM trigger_rules WHERE id = ?1", params![id])
        .with_context(|| format!("failed to delete trigger rule '{id}'"))?;
    Ok(rows > 0)
}

/// Toggle a trigger rule's enabled state. Returns the new enabled state.
#[instrument(skip(context))]
pub fn toggle_trigger_rule_with_context(
    context: &StorageContext,
    id: &str,
) -> Result<Option<bool>> {
    let (connection, _) = open_connection(context)?;

    // Get current enabled state
    let current: Option<i32> = connection
        .query_row(
            "SELECT enabled FROM trigger_rules WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;

    match current {
        None => Ok(None),
        Some(val) => {
            let new_enabled = if val == 0 { 1 } else { 0 };
            let now = Utc::now().to_rfc3339();
            connection.execute(
                "UPDATE trigger_rules SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
                params![new_enabled, now, id],
            )?;
            Ok(Some(new_enabled != 0))
        }
    }
}

// ────────────────────────────────────────────────────────
// Internal helpers
// ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn build_rule(
    id: &str,
    label: &str,
    trigger_type: &str,
    trigger_config: &str,
    filter: Option<&str>,
    action: &str,
    enabled: bool,
    custom_prompt: Option<&str>,
    _created_at: &str,
    _updated_at: &str,
) -> AgentTriggerRule {
    let trigger = match trigger_type {
        "event" => TriggerKind::Event {
            name: trigger_config.to_string(),
            filter: filter.map(|s| s.to_string()),
        },
        _ => TriggerKind::Cron {
            expression: trigger_config.to_string(),
        },
    };

    let parsed_action = TriggerAction::from_arg(action).unwrap_or(TriggerAction::Custom);

    AgentTriggerRule {
        id: id.to_string(),
        label: label.to_string(),
        trigger,
        action: parsed_action,
        enabled,
        custom_prompt: custom_prompt.map(|s| s.to_string()),
    }
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn setup_context() -> (PathBuf, StorageContext) {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-trigger-test-{}-{}",
            std::process::id(),
            counter
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    #[test]
    fn test_create_and_list_trigger_rules() {
        let (_tmp, ctx) = setup_context();

        let rule = create_trigger_rule_with_context(
            &ctx,
            "Morning Review",
            "cron",
            "0 8 * * *",
            "daily_review",
            None,
            None,
        )
        .expect("create rule");
        assert_eq!(rule.label, "Morning Review");
        assert!(rule.enabled);

        let rules = list_trigger_rules_with_context(&ctx).expect("list rules");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].label, "Morning Review");
    }

    #[test]
    fn test_create_event_trigger_rule() {
        let (_tmp, ctx) = setup_context();

        let rule = create_trigger_rule_with_context(
            &ctx,
            "Tag New Notes",
            "event",
            "note_created",
            "summarize_and_tag",
            Some("tags CONTAINS meeting"),
            None,
        )
        .expect("create event rule");

        match &rule.trigger {
            TriggerKind::Event { name, filter } => {
                assert_eq!(name, "note_created");
                assert_eq!(filter.as_deref(), Some("tags CONTAINS meeting"));
            }
            _ => panic!("expected event trigger"),
        }
        assert_eq!(rule.action, TriggerAction::SummarizeAndTag);
    }

    #[test]
    fn test_create_custom_prompt_rule() {
        let (_tmp, ctx) = setup_context();

        let rule = create_trigger_rule_with_context(
            &ctx,
            "Custom Task",
            "cron",
            "0 9 * * 1",
            "custom",
            None,
            Some("Summarize last week's notes"),
        )
        .expect("create custom rule");

        assert_eq!(rule.action, TriggerAction::Custom);
        assert_eq!(rule.effective_prompt(), Some("Summarize last week's notes"));
    }

    #[test]
    fn test_delete_trigger_rule() {
        let (_tmp, ctx) = setup_context();

        let rule = create_trigger_rule_with_context(
            &ctx,
            "Temp Rule",
            "cron",
            "0 0 * * *",
            "daily_review",
            None,
            None,
        )
        .expect("create rule");

        let deleted = delete_trigger_rule_with_context(&ctx, &rule.id).expect("delete rule");
        assert!(deleted);

        let rules = list_trigger_rules_with_context(&ctx).expect("list rules");
        assert_eq!(rules.len(), 0);
    }

    #[test]
    fn test_toggle_trigger_rule() {
        let (_tmp, ctx) = setup_context();

        let rule = create_trigger_rule_with_context(
            &ctx,
            "Toggle Test",
            "cron",
            "0 0 * * *",
            "daily_review",
            None,
            None,
        )
        .expect("create rule");
        assert!(rule.enabled);

        let toggled = toggle_trigger_rule_with_context(&ctx, &rule.id)
            .expect("toggle")
            .expect("rule exists");
        assert!(!toggled);

        let toggled_again = toggle_trigger_rule_with_context(&ctx, &rule.id)
            .expect("toggle again")
            .expect("rule exists");
        assert!(toggled_again);
    }

    #[test]
    fn test_get_nonexistent_rule() {
        let (_tmp, ctx) = setup_context();
        let result = get_trigger_rule_with_context(&ctx, "nonexistent-id").expect("get rule");
        assert!(result.is_none());
    }

    #[test]
    fn test_toggle_nonexistent_rule() {
        let (_tmp, ctx) = setup_context();
        let result = toggle_trigger_rule_with_context(&ctx, "nonexistent-id").expect("toggle");
        assert!(result.is_none());
    }
}
