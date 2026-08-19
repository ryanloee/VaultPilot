//! Trigger rule management commands.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use vaultpilot_lib::storage::{
    create_trigger_rule_with_context, delete_trigger_rule_with_context,
    initialize_storage_async, list_trigger_rules_with_context,
    toggle_trigger_rule_with_context,
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
}

impl From<vaultpilot_lib::orchestration::trigger::AgentTriggerRule> for TriggerRuleDto {
    fn from(r: vaultpilot_lib::orchestration::trigger::AgentTriggerRule) -> Self {
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
        }
    }
}

#[tauri::command]
pub async fn list_trigger_rules(state: tauri::State<'_, AppState>) -> Result<Vec<TriggerRuleDto>, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx).await.map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        list_trigger_rules_with_context(&ctx)
            .map(|rules| rules.into_iter().map(TriggerRuleDto::from).collect())
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
    initialize_storage_async(&ctx).await.map_err(|e| e.to_string())?;
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
pub async fn toggle_trigger_rule(
    state: tauri::State<'_, AppState>,
    rule_id: String,
) -> Result<bool, String> {
    let ctx = state.storage.clone();
    initialize_storage_async(&ctx).await.map_err(|e| e.to_string())?;
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
    initialize_storage_async(&ctx).await.map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        delete_trigger_rule_with_context(&ctx, &rule_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
