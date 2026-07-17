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
    /// Prompt text for the `Custom` action. Only meaningful when
    /// `action == TriggerAction::Custom`; stored on the rule because the
    /// `TriggerAction` enum variants are unit-like. Omitted from serialized
    /// output when `None` for backward compatibility (#2842).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_prompt: Option<String>,
}

impl AgentTriggerRule {
    /// The prompt text that should be executed for this rule.
    ///
    /// For `TriggerAction::Custom` this is `custom_prompt` (required). For all
    /// other actions the predefined task template is used and this returns
    /// `None`. Returns `None` for a `Custom` rule that has no prompt text set,
    /// which callers should treat as a configuration error rather than running
    /// an empty action (#2842).
    pub fn effective_prompt(&self) -> Option<&str> {
        match &self.action {
            TriggerAction::Custom => self.custom_prompt.as_deref(),
            _ => None,
        }
    }
}

/// Status of the trigger-rule execution layer.
///
/// As of v0.5.62 (#3053), the cron-based executor shipped in a033c4c (#3048)
/// is fully wired up: `TriggerExecutor::spawn` reads the `trigger_rules`
/// table on a 60 s cadence, evaluates due cron rules, records an execution
/// row in `trigger_executions` per fire, and updates each rule's
/// `last_fired_at` / `run_count` / `last_status`. The `trigger fire-now` and
/// `trigger start` CLI subcommands exercise the same path.
///
/// `NotConnected` is retained for backward-compat (older clients may still
/// parse the JSON `executor_status` field) and for the future event-bus
/// dispatcher, which is not yet wired up — but cron evaluation is live.
///
/// See <https://github.com/ryanloee/VaultPilot/issues/3048> and
/// <https://github.com/ryanloee/VaultPilot/issues/3053>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorStatus {
    /// No background loop / event dispatcher is reading the trigger_rules
    /// table. Rules are persisted but never fire. CLI must warn the user.
    /// Currently only retained for backward-compat with older JSON consumers.
    NotConnected,
    /// The executor is active: due cron rules fire on schedule and an
    /// execution row is recorded per fire. Returned by [`Self::current()`]
    /// since the cron executor shipped in a033c4c (#3048 / #3053).
    Connected,
}

impl ExecutorStatus {
    /// Current executor status. See [`ExecutorStatus`] doc for context.
    ///
    /// Returns [`Self::Connected`] since a033c4c (#3048) wired up the
    /// cron evaluator + execution-log writer. The deferred agent-dispatch
    /// work (LLM daily-review generation) is independent of this flag —
    /// the flag reflects "is *any* executor reading the table", and the
    /// cron evaluator clearly is (#3053).
    pub fn current() -> Self {
        // #3053: executor shipped in a033c4c (v0.5.62). Flip from
        // NotConnected to Connected so the CLI no longer prints the stale
        // "rules will NOT fire" warning that contradicts the executor that
        // just landed.
        Self::Connected
    }

    /// Stable string id suitable for JSON / CLI display.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotConnected => "not_connected",
            Self::Connected => "connected",
        }
    }

    /// Human-readable warning shown when rules exist but will not fire.
    /// Returns `None` when the executor is connected.
    pub fn warning(&self) -> Option<&'static str> {
        match self {
            Self::NotConnected => Some(
                "Trigger rules are stored but NOT executed yet — the executor \
                 (scheduler / event dispatcher) is not connected in this build. \
                 Rules will not fire until a future version wires it up (#3048).",
            ),
            Self::Connected => None,
        }
    }
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
            custom_prompt: None,
        };
        let json = serde_json::to_string(&rule).unwrap();
        let parsed: AgentTriggerRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rule);
    }

    #[test]
    fn agent_trigger_rule_custom_prompt_roundtrip() {
        // Regression test for #2842: a `Custom` trigger must be able to carry
        // its prompt text, and the field must round-trip through JSON while
        // remaining backward compatible (absent field deserializes to None).
        let rule = AgentTriggerRule {
            id: "custom-1".into(),
            label: "Custom Task".into(),
            trigger: TriggerKind::Cron {
                expression: "0 8 * * *".into(),
            },
            action: TriggerAction::Custom,
            enabled: true,
            custom_prompt: Some("Summarize the meeting notes for {{date}}".into()),
        };

        // The prompt is reachable via effective_prompt for Custom actions.
        assert_eq!(
            rule.effective_prompt(),
            Some("Summarize the meeting notes for {{date}}")
        );

        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("Summarize the meeting notes for {{date}}"));

        let parsed: AgentTriggerRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rule);
        assert_eq!(
            parsed.effective_prompt(),
            Some("Summarize the meeting notes for {{date}}")
        );

        // Non-custom actions must not surface a custom prompt.
        let daily = AgentTriggerRule {
            id: "daily-1".into(),
            label: "Daily".into(),
            trigger: TriggerKind::Cron {
                expression: "0 8 * * *".into(),
            },
            action: TriggerAction::DailyReview,
            enabled: true,
            custom_prompt: None,
        };
        assert_eq!(daily.effective_prompt(), None);
    }

    #[test]
    fn agent_trigger_rule_backward_compat_missing_custom_prompt() {
        // A previously-persisted rule without the custom_prompt field (written
        // before #2842) must still deserialize, defaulting to None (#2842).
        let json = r#"{
            "id": "legacy-1",
            "label": "Legacy Custom",
            "trigger": {"type": "cron", "expression": "0 8 * * *"},
            "action": "custom",
            "enabled": true
        }"#;
        let parsed: AgentTriggerRule = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.action, TriggerAction::Custom);
        assert_eq!(parsed.custom_prompt, None);
        // A Custom rule without a prompt is a configuration error, not a silent no-op.
        assert_eq!(parsed.effective_prompt(), None);
    }

    // ─── #3048 / #3053: executor-status honesty contract ───────────────
    //
    // The trigger_rules storage layer shipped in v0.5.61 (#3048) and the
    // cron-based executor that actually fires rules shipped in a033c4c
    // (v0.5.62, #3053). `ExecutorStatus::current()` reflects whether *any*
    // executor is wired up — and the cron evaluator + execution-log writer
    // clearly is. These tests pin the contract:
    //   - ExecutorStatus::current() reflects whether the executor is wired up.
    //   - When NotConnected, a non-empty warning string is available.
    //   - as_str() round-trips through serde as snake_case.
    //
    // (Previously this asserted NotConnected; it was flipped to Connected
    // in #3053 when the executor shipped.)

    #[test]
    fn executor_status_current_matches_wiring() {
        // #3053: the cron executor shipped in a033c4c — current() must now
        // report Connected so the CLI no longer emits the stale "rules will
        // NOT fire" warning that contradicts the live executor.
        assert_eq!(ExecutorStatus::current(), ExecutorStatus::Connected);
    }

    #[test]
    fn executor_status_not_connected_has_warning() {
        let warning = ExecutorStatus::NotConnected.warning();
        assert!(warning.is_some(), "NotConnected must surface a warning");
        let w = warning.unwrap();
        assert!(!w.is_empty());
        // Warning must mention the executor is not active and that rules
        // won't fire — this is the whole point of #3048.
        assert!(
            w.to_lowercase().contains("not"),
            "warning should state the executor is not connected: {w}"
        );
        assert!(
            w.to_lowercase().contains("fire") || w.to_lowercase().contains("execute"),
            "warning should mention that rules won't fire: {w}"
        );
    }

    #[test]
    fn executor_status_connected_has_no_warning() {
        // When the executor ships, no warning should be shown.
        assert_eq!(ExecutorStatus::Connected.warning(), None);
    }

    #[test]
    fn executor_status_as_str_is_stable() {
        // as_str() is part of the CLI's JSON contract — must not change
        // without coordinating with downstream parsers (WinUI / mobile).
        assert_eq!(ExecutorStatus::NotConnected.as_str(), "not_connected");
        assert_eq!(ExecutorStatus::Connected.as_str(), "connected");
    }

    #[test]
    fn executor_status_serializes_snake_case() {
        let json = serde_json::to_string(&ExecutorStatus::NotConnected).unwrap();
        assert_eq!(json, "\"not_connected\"");
        let parsed: ExecutorStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ExecutorStatus::NotConnected);

        let json = serde_json::to_string(&ExecutorStatus::Connected).unwrap();
        assert_eq!(json, "\"connected\"");
    }
}
