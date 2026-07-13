//! # Agent Trigger Rules (#2799)
//!
//! User-definable rules that tell the agent to run automatically on a schedule
//! or in response to vault events (note created, updated, deleted, webhook, etc.).
//!
//! ## Design
//!
//! Each rule specifies:
//! - A **trigger** (cron expression or event name + optional filter)
//! - An **action** (predefined prompt-template id)
//! - Whether it is **enabled**
//!
//! Rules are stored as JSON/YAML in the vault alongside settings, but the
//! active set is parsed into this struct.

use serde::{Deserialize, Serialize};

/// A single user-defined trigger rule for the agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTriggerRule {
    /// Unique id for the rule (uuid v4).
    pub id: String,
    /// Human-readable label shown in UI / CLI.
    pub label: String,
    /// What makes the agent run.
    pub trigger: TriggerKind,
    /// Which predefined task to execute.
    pub action: TriggerAction,
    /// Whether this rule is active. Disabled rules are kept but skipped.
    pub enabled: bool,
}

/// Trigger source: either a cron schedule or a vault event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum TriggerKind {
    /// Run on a cron schedule (Unix cron syntax, 5-field).
    #[serde(rename = "cron")]
    Cron {
        /// Cron expression, e.g. "0 8 * * *".
        expression: String,
    },
    /// Run when a specific vault event fires.
    #[serde(rename = "event")]
    Event {
        /// Event name (e.g. "note_created", "note_updated").
        name: String,
        /// Optional tag-based or content filter.
        #[serde(skip_serializing_if = "Option::is_none")]
        filter: Option<String>,
    },
}

/// Predefined action the agent should perform when the trigger fires.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerAction {
    /// Generate a daily review of recent notes.
    DailyReview,
    /// Summarize and tag new/modified notes.
    SummarizeAndTag,
    /// Find and suggest links between notes.
    SuggestLinks,
    /// Process an external webhook payload.
    ProcessWebhook,
    /// Run a custom prompt (prompt text is stored in the rule).
    #[serde(rename = "custom")]
    Custom,
}

impl TriggerAction {
    /// Human-readable label for CLI / UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::DailyReview => "Daily Review",
            Self::SummarizeAndTag => "Summarize & Tag",
            Self::SuggestLinks => "Suggest Links",
            Self::ProcessWebhook => "Process Webhook",
            Self::Custom => "Custom Prompt",
        }
    }

    /// The provider-agnostic "task" id used to select the system prompt.
    pub fn task_id(&self) -> &'static str {
        match self {
            Self::DailyReview => "agent/daily_review",
            Self::SummarizeAndTag => "agent/summarize_and_tag",
            Self::SuggestLinks => "agent/suggest_links",
            Self::ProcessWebhook => "agent/process_webhook",
            Self::Custom => "agent/custom",
        }
    }

    /// All actions available to the user.
    pub fn all() -> &'static [TriggerAction] {
        &[
            Self::DailyReview,
            Self::SummarizeAndTag,
            Self::SuggestLinks,
            Self::ProcessWebhook,
            Self::Custom,
        ]
    }

    /// Parse from CLI string (case-insensitive, snake_case or kebab-case).
    pub fn from_arg(s: &str) -> Option<Self> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "daily_review" | "dailyreview" => Some(Self::DailyReview),
            "summarize_and_tag" | "summarize_tag" | "summarizetag" => Some(Self::SummarizeAndTag),
            "suggest_links" | "suggestlinks" => Some(Self::SuggestLinks),
            "process_webhook" | "processwebhook" | "webhook" => Some(Self::ProcessWebhook),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_action_from_arg_case_insensitive() {
        assert_eq!(
            TriggerAction::from_arg("daily_review"),
            Some(TriggerAction::DailyReview)
        );
        assert_eq!(
            TriggerAction::from_arg("DAILY_REVIEW"),
            Some(TriggerAction::DailyReview)
        );
        assert_eq!(
            TriggerAction::from_arg("summarize-and-tag"),
            Some(TriggerAction::SummarizeAndTag)
        );
    }

    #[test]
    fn trigger_action_from_arg_unknown_returns_none() {
        assert_eq!(TriggerAction::from_arg("nonexistent"), None);
    }

    #[test]
    fn trigger_kind_serialization_roundtrip() {
        let cron = TriggerKind::Cron {
            expression: "0 8 * * *".into(),
        };
        let json = serde_json::to_string(&cron).unwrap();
        let parsed: TriggerKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cron);

        let event = TriggerKind::Event {
            name: "note_created".into(),
            filter: Some("tags CONTAINS meeting".into()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: TriggerKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event);
    }

    #[test]
    fn agent_trigger_rule_serialization_roundtrip() {
        let rule = AgentTriggerRule {
            id: "abc-123".into(),
            label: "Morning Review".into(),
            trigger: TriggerKind::Cron {
                expression: "0 8 * * *".into(),
            },
            action: TriggerAction::DailyReview,
            enabled: true,
        };
        let json = serde_json::to_string(&rule).unwrap();
        let parsed: AgentTriggerRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rule);
    }
}
