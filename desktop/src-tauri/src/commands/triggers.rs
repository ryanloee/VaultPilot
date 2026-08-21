//! Trigger rule management commands.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use vaultpilot_lib::storage::{
    create_trigger_rule_with_context, delete_trigger_rule_with_context, initialize_storage_async,
    list_recent_trigger_executions_with_context, list_trigger_rules_with_status_with_context,
    toggle_trigger_rule_with_context, update_trigger_rule_with_context,
};

#[derive(Serialize, Deserialize)]
pub struct TriggerRuleDto {
    pub id: String,
    pub label: String,
    pub trigger_type: String,
    pub trigger_config: String,
    pub filter: Option<String>,
    pub action: String,
    pub enabled: bool,
    pub custom_prompt: Option<String>,
    /// Scheduler status so the UI can answer "did it fire, and did it work?".
    #[serde(default)]
    pub last_fired_at: Option<String>,
    #[serde(default)]
    pub next_fire_at: Option<String>,
    #[serde(default)]
    pub run_count: i64,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl From<vaultpilot_lib::orchestration::trigger::AgentTriggerRule> for TriggerRuleDto {
    fn from(r: vaultpilot_lib::orchestration::trigger::AgentTriggerRule) -> Self {
        Self::with_status(
            r,
            vaultpilot_lib::storage::TriggerRuleStatus {
                last_fired_at: None,
                next_fire_at: None,
                run_count: 0,
                last_status: None,
                last_error: None,
            },
        )
    }
}

impl TriggerRuleDto {
    fn with_status(
        r: vaultpilot_lib::orchestration::trigger::AgentTriggerRule,
        s: vaultpilot_lib::storage::TriggerRuleStatus,
    ) -> Self {
        let (trigger_type, trigger_config, filter) = match &r.trigger {
            vaultpilot_lib::orchestration::trigger::TriggerKind::Cron { expression } => {
                ("cron".to_string(), expression.clone(), None)
            }
            vaultpilot_lib::orchestration::trigger::TriggerKind::Event { name, filter } => {
                ("event".to_string(), name.clone(), filter.clone())
            }
        };
        Self {
            id: r.id,
            label: r.label,
            trigger_type,
            trigger_config,
            filter,
            action: format!("{:?}", r.action).to_lowercase(),
            enabled: r.enabled,
            custom_prompt: r.custom_prompt,
            last_fired_at: s.last_fired_at,
            next_fire_at: s.next_fire_at,
            run_count: s.run_count,
            last_status: s.last_status,
            last_error: s.last_error,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct TriggerExecutionDto {
    pub id: String,
    pub rule_id: String,
    pub label: String,
    pub action: String,
    pub fired_at: String,
    pub status: String,
    pub error: String,
    pub detail: String,
    pub result_content: String,
}

impl From<vaultpilot_lib::storage::TriggerExecutionRecord> for TriggerExecutionDto {
    fn from(r: vaultpilot_lib::storage::TriggerExecutionRecord) -> Self {
        Self {
            id: r.id,
            rule_id: r.rule_id,
            label: r.label,
            action: r.action,
            fired_at: r.fired_at,
            status: r.status,
            error: r.error,
            detail: r.detail,
            result_content: r.result_content,
        }
    }
}

/// Delete a single execution-log row.
#[tauri::command]
pub async fn delete_trigger_execution(
    state: tauri::State<'_, AppState>,
    execution_id: String,
) -> Result<bool, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        vaultpilot_lib::storage::delete_trigger_execution_with_context(&ctx, &execution_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Clear ALL execution-log rows.
#[tauri::command]
pub async fn clear_trigger_executions(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        vaultpilot_lib::storage::clear_trigger_executions_with_context(&ctx)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_trigger_rules(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<TriggerRuleDto>, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        list_trigger_rules_with_status_with_context(&ctx)
            .map(|pairs| {
                pairs
                    .into_iter()
                    .map(|(r, s)| TriggerRuleDto::with_status(r, s))
                    .collect()
            })
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Recent trigger-rule fires (newest first) — the user-facing execution log
/// that answers "did my scheduled task actually run?".
#[tauri::command]
pub async fn list_trigger_executions(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<TriggerExecutionDto>, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        list_recent_trigger_executions_with_context(&ctx, limit.unwrap_or(20))
            .map(|rows| rows.into_iter().map(TriggerExecutionDto::from).collect())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn create_trigger_rule(
    state: tauri::State<'_, AppState>,
    label: String,
    trigger_type: String,
    trigger_config: String,
    action: String,
    filter: Option<String>,
    custom_prompt: Option<String>,
) -> Result<TriggerRuleDto, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        create_trigger_rule_with_context(
            &ctx,
            &label,
            &trigger_type,
            &trigger_config,
            &action,
            filter.as_deref(),
            custom_prompt.as_deref(),
        )
        .map(TriggerRuleDto::from)
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_trigger_rule(
    state: tauri::State<'_, AppState>,
    rule_id: String,
    label: String,
    trigger_type: String,
    trigger_config: String,
    action: String,
    filter: Option<String>,
    custom_prompt: Option<String>,
) -> Result<TriggerRuleDto, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        update_trigger_rule_with_context(
            &ctx,
            &rule_id,
            &label,
            &trigger_type,
            &trigger_config,
            &action,
            filter.as_deref(),
            custom_prompt.as_deref(),
        )
        .map_err(|e| e.to_string())?
        .map(TriggerRuleDto::from)
        .ok_or_else(|| format!("rule not found: {rule_id}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Manually fire a rule right now ("立即触发" button) — bypasses the cron
/// schedule and dispatches the action through the full AI pipeline.
#[derive(Serialize, Deserialize)]
pub struct FireNowResult {
    pub success: bool,
    pub error: Option<String>,
    pub detail: Option<String>,
}

#[tauri::command]
pub async fn fire_trigger_rule_now(
    state: tauri::State<'_, AppState>,
    rule_id: String,
) -> Result<FireNowResult, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    let result =
        vaultpilot_lib::orchestration::trigger_executor::fire_trigger_rule_now(&ctx, &rule_id)
            .await
            .map_err(|e| e.to_string())?;
    match result {
        None => Err(format!("rule not found: {rule_id}")),
        Some(d) => Ok(FireNowResult {
            success: d.status == "success",
            error: if d.error.is_empty() {
                None
            } else {
                Some(d.error)
            },
            detail: if d.detail.is_empty() {
                None
            } else {
                Some(d.detail)
            },
        }),
    }
}

#[tauri::command]
pub async fn toggle_trigger_rule(
    state: tauri::State<'_, AppState>,
    rule_id: String,
) -> Result<bool, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        toggle_trigger_rule_with_context(&ctx, &rule_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "rule not found".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_trigger_rule(
    state: tauri::State<'_, AppState>,
    rule_id: String,
) -> Result<bool, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        delete_trigger_rule_with_context(&ctx, &rule_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
