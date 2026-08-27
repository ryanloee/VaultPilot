//! # Agent Trigger Rules (#2799)
//!
//! User-definable rules that tell the agent to run automatically on a schedule
//! or in response to vault events (note created, updated, deleted, webhook, etc.).
//!
//! ## Design
//!
//! Each rule specifies:
//! - A **trigger** (cron expression or event name + optional filter)
//! - **Conditions** (optional constraints that must be satisfied for the rule to fire)
//! - An **action** (predefined prompt-template id)
//! - Whether it is **enabled**
//!
//! Conditions are evaluated when the trigger fires. If all conditions match,
//! the action is executed. Empty conditions list means unconditional.
//!
//! Rules are stored as JSON/YAML in the vault alongside settings, but the
//! active set is parsed into this struct.

use chrono::Timelike;
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
    /// Provider name from settings.providers to use for this rule's AI call.
    /// `None` = use the currently active provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    /// Optional conditions that must be satisfied for this rule to fire.
    /// Evaluated when the trigger fires. All conditions must match.
    /// Empty (default) means unconditional — always fire when triggered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
}

/// A condition that must be satisfied for a trigger rule to fire.
///
/// All conditions in a rule's `conditions` list must match. The condition
/// is evaluated against the current vault state or event context depending
/// on the trigger type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    /// The event target (e.g., note) must have the specified tag.
    TagContains {
        /// Tag to check for (without # prefix).
        tag: String,
    },
    /// A frontmatter field must equal the specified value.
    FrontmatterEquals {
        /// Frontmatter field name.
        field: String,
        /// Expected value.
        value: String,
    },
    /// Always matches — unconditional.
    Always,
    /// Current time must fall within the specified window (#3441).
    ///
    /// Both `start` and `end` are `HH:MM` (24-hour). Overnight windows are
    /// supported (e.g. `start = "22:00"`, `end = "06:00"` matches 22:00–23:59
    /// and 00:00–06:00).
    TimeWindow {
        /// Window start in `HH:MM` (24-hour).
        start: String,
        /// Window end in `HH:MM` (24-hour, exclusive).
        end: String,
    },
    /// Note body text must contain the specified substring (#3441).
    ///
    /// Case-sensitive substring match. For regex matching, see future
    /// `ContentMatches` variant (requires `regex` crate dependency).
    ContentContains {
        /// Substring to search for in the note body.
        substring: String,
    },
}

impl Condition {
    /// Evaluate this condition in the given context.
    ///
    /// Returns `true` if the condition is satisfied.
    pub fn matches(&self, _context: &ConditionContext) -> bool {
        match self {
            // Always matches regardless of context.
            Condition::Always => true,
            // Tag conditions require context with tags.
            Condition::TagContains { tag } => _context.tags.iter().any(|t| t == tag),
            // Frontmatter conditions require context with matching field.
            Condition::FrontmatterEquals { field, value } => {
                _context.frontmatter.get(field).is_some_and(|v| v == value)
            }
            // Time-window condition: current time must fall within [start, end).
            Condition::TimeWindow { start, end } => _context
                .now
                .is_some_and(|now| time_in_window(now, start, end)),
            // Content-contains condition: note text must contain substring.
            Condition::ContentContains { substring } => _context
                .note_text
                .as_deref()
                .is_some_and(|text| text.contains(substring.as_str())),
        }
    }
}

/// Check whether `now` falls within the daily time window `[start, end)`.
///
/// `start` and `end` are `HH:MM` (24-hour). Overnight windows (start > end)
/// are supported: `22:00`–`06:00` matches 22:00–23:59 and 00:00–05:59.
/// Returns `false` if either boundary fails to parse.
fn time_in_window(now: chrono::DateTime<chrono::Utc>, start: &str, end: &str) -> bool {
    let (Ok(s), Ok(e)) = (parse_hhmm(start), parse_hhmm(end)) else {
        return false;
    };
    // Extract current time as minutes since midnight directly from the
    // DateTime, avoiding string format/parse round-trips.
    let cur_min = now.hour() * 60 + now.minute();
    if s <= e {
        // Normal window: e.g. 09:00–18:00.
        cur_min >= s && cur_min < e
    } else {
        // Overnight window: e.g. 22:00–06:00.
        cur_min >= s || cur_min < e
    }
}

/// Parse a `HH:MM` string into total minutes since midnight.
fn parse_hhmm(s: &str) -> Result<u32, std::num::ParseIntError> {
    let (h, m) = s.split_once(':').unwrap_or((s, "0"));
    let hours: u32 = h.parse()?;
    let minutes: u32 = m.parse()?;
    Ok(hours * 60 + minutes)
}

/// Context available when evaluating trigger rule conditions.
///
/// Populated from the event payload (for event triggers) or current vault
/// state (for cron triggers).
#[derive(Debug, Clone, Default)]
pub struct ConditionContext {
    /// Tags associated with the triggering event / current context.
    pub tags: Vec<String>,
    /// Frontmatter fields and values associated with the context.
    pub frontmatter: std::collections::HashMap<String, String>,
    /// Current timestamp — populated by the cron executor from its `now`
    /// clock parameter. Used by [`Condition::TimeWindow`]. `None` means
    /// "not available" (e.g. tests that don't exercise time conditions).
    pub now: Option<chrono::DateTime<chrono::Utc>>,
    /// Body text of the triggering note (for event triggers) or `None`
    /// (for cron triggers with no note context). Used by
    /// [`Condition::ContentContains`].
    pub note_text: Option<String>,
}

impl AgentTriggerRule {
    /// Check whether all conditions on this rule are satisfied.
    ///
    /// Returns `true` if the rule has no conditions (unconditional) or all
    /// conditions match. Returns `false` if any condition fails.
    pub fn conditions_met(&self, context: &ConditionContext) -> bool {
        if self.conditions.is_empty() {
            return true;
        }
        self.conditions.iter().all(|c| c.matches(context))
    }
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
            provider_name: None,
            conditions: vec![],
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
            provider_name: None,
            conditions: vec![],
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
            provider_name: None,
            conditions: vec![],
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

    // ─── Condition matching tests ────────────────────────────

    #[test]
    fn condition_always_matches() {
        let ctx = ConditionContext::default();
        assert!(Condition::Always.matches(&ctx));
    }

    #[test]
    fn condition_tag_contains_matches() {
        let ctx = ConditionContext {
            tags: vec!["urgent".into(), "meeting".into()],
            ..Default::default()
        };
        assert!(Condition::TagContains {
            tag: "urgent".into()
        }
        .matches(&ctx));
        assert!(Condition::TagContains {
            tag: "meeting".into()
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_tag_contains_no_match() {
        let ctx = ConditionContext {
            tags: vec!["meeting".into()],
            ..Default::default()
        };
        assert!(!Condition::TagContains {
            tag: "urgent".into()
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_tag_contains_empty_context() {
        let ctx = ConditionContext::default();
        assert!(!Condition::TagContains {
            tag: "anything".into()
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_frontmatter_equals_matches() {
        let mut ctx = ConditionContext::default();
        ctx.frontmatter.insert("status".into(), "done".into());
        assert!(Condition::FrontmatterEquals {
            field: "status".into(),
            value: "done".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_frontmatter_equals_no_match() {
        let mut ctx = ConditionContext::default();
        ctx.frontmatter.insert("status".into(), "todo".into());
        assert!(!Condition::FrontmatterEquals {
            field: "status".into(),
            value: "done".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_frontmatter_equals_missing_field() {
        let ctx = ConditionContext::default();
        assert!(!Condition::FrontmatterEquals {
            field: "status".into(),
            value: "done".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn rule_conditions_met_empty_list() {
        let rule = AgentTriggerRule {
            id: "test-1".into(),
            label: "Test".into(),
            trigger: TriggerKind::Cron {
                expression: "0 8 * * *".into(),
            },
            action: TriggerAction::DailyReview,
            enabled: true,
            custom_prompt: None,
            provider_name: None,
            conditions: vec![],
        };
        // Empty conditions means unconditional.
        assert!(rule.conditions_met(&ConditionContext::default()));
    }

    #[test]
    fn rule_conditions_met_all_pass() {
        let rule = AgentTriggerRule {
            id: "test-2".into(),
            label: "Urgent Review".into(),
            trigger: TriggerKind::Cron {
                expression: "0 8 * * *".into(),
            },
            action: TriggerAction::SummarizeAndTag,
            enabled: true,
            custom_prompt: None,
            provider_name: None,
            conditions: vec![Condition::TagContains {
                tag: "urgent".into(),
            }],
        };
        let ctx = ConditionContext {
            tags: vec!["urgent".into()],
            ..Default::default()
        };
        assert!(rule.conditions_met(&ctx));
    }

    #[test]
    fn rule_conditions_met_one_fails() {
        let rule = AgentTriggerRule {
            id: "test-3".into(),
            label: "Tagged Review".into(),
            trigger: TriggerKind::Cron {
                expression: "0 8 * * *".into(),
            },
            action: TriggerAction::SummarizeAndTag,
            enabled: true,
            custom_prompt: None,
            provider_name: None,
            conditions: vec![
                Condition::TagContains {
                    tag: "urgent".into(),
                },
                Condition::FrontmatterEquals {
                    field: "status".into(),
                    value: "done".into(),
                },
            ],
        };
        // Only tag matches, not frontmatter
        let ctx = ConditionContext {
            tags: vec!["urgent".into()],
            ..Default::default()
        };
        assert!(!rule.conditions_met(&ctx));
    }

    #[test]
    fn condition_serialization_roundtrip() {
        let conditions = vec![
            Condition::Always,
            Condition::TagContains {
                tag: "urgent".into(),
            },
            Condition::FrontmatterEquals {
                field: "status".into(),
                value: "done".into(),
            },
            // #3441: new variants
            Condition::TimeWindow {
                start: "09:00".into(),
                end: "18:00".into(),
            },
            Condition::ContentContains {
                substring: "TODO".into(),
            },
        ];
        for cond in &conditions {
            let json = serde_json::to_string(cond).unwrap();
            let parsed: Condition = serde_json::from_str(&json).unwrap();
            assert_eq!(*cond, parsed, "round-trip failed for {cond:?} -> {json}");
        }
    }

    // ── TimeWindow condition tests (#3441) ──

    #[test]
    fn condition_time_window_within_window_matches() {
        let ctx = ConditionContext {
            now: Some(
                chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 7, 25, 10, 30, 0).unwrap(),
            ),
            ..Default::default()
        };
        assert!(Condition::TimeWindow {
            start: "09:00".into(),
            end: "18:00".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_time_window_outside_window_no_match() {
        let ctx = ConditionContext {
            now: Some(
                chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 7, 25, 20, 0, 0).unwrap(),
            ),
            ..Default::default()
        };
        assert!(!Condition::TimeWindow {
            start: "09:00".into(),
            end: "18:00".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_time_window_overnight_matches() {
        // 22:00–06:00 overnight window. 23:00 should match.
        let late = ConditionContext {
            now: Some(
                chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 7, 25, 23, 0, 0).unwrap(),
            ),
            ..Default::default()
        };
        assert!(Condition::TimeWindow {
            start: "22:00".into(),
            end: "06:00".into(),
        }
        .matches(&late));

        // 03:00 should also match (early morning).
        let early = ConditionContext {
            now: Some(
                chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 7, 25, 3, 0, 0).unwrap(),
            ),
            ..Default::default()
        };
        assert!(Condition::TimeWindow {
            start: "22:00".into(),
            end: "06:00".into(),
        }
        .matches(&early));

        // 12:00 should NOT match (middle of day).
        let noon = ConditionContext {
            now: Some(
                chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 7, 25, 12, 0, 0).unwrap(),
            ),
            ..Default::default()
        };
        assert!(!Condition::TimeWindow {
            start: "22:00".into(),
            end: "06:00".into(),
        }
        .matches(&noon));
    }

    #[test]
    fn condition_time_window_no_now_returns_false() {
        let ctx = ConditionContext::default();
        assert!(!Condition::TimeWindow {
            start: "09:00".into(),
            end: "18:00".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_time_window_invalid_format_returns_false() {
        let ctx = ConditionContext {
            now: Some(
                chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 7, 25, 10, 0, 0).unwrap(),
            ),
            ..Default::default()
        };
        assert!(!Condition::TimeWindow {
            start: "bad".into(),
            end: "18:00".into(),
        }
        .matches(&ctx));
    }

    // ── ContentContains condition tests (#3441) ──

    #[test]
    fn condition_content_contains_match() {
        let ctx = ConditionContext {
            note_text: Some("This note has a TODO item".into()),
            ..Default::default()
        };
        assert!(Condition::ContentContains {
            substring: "TODO".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_content_contains_no_match() {
        let ctx = ConditionContext {
            note_text: Some("Nothing to see here".into()),
            ..Default::default()
        };
        assert!(!Condition::ContentContains {
            substring: "TODO".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_content_contains_empty_note_text() {
        let ctx = ConditionContext::default();
        assert!(!Condition::ContentContains {
            substring: "anything".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn condition_content_contains_case_sensitive() {
        let ctx = ConditionContext {
            note_text: Some("todo item".into()),
            ..Default::default()
        };
        // Case-sensitive: "TODO" != "todo"
        assert!(!Condition::ContentContains {
            substring: "TODO".into(),
        }
        .matches(&ctx));
    }

    #[test]
    fn rule_conditions_serialized_with_default_skip() {
        // A rule with no conditions should not include "conditions" in JSON
        // (backward-compatible with older consumers).
        let rule = AgentTriggerRule {
            id: "no-conds".into(),
            label: "No Conds".into(),
            trigger: TriggerKind::Cron {
                expression: "0 8 * * *".into(),
            },
            action: TriggerAction::DailyReview,
            enabled: true,
            custom_prompt: None,
            provider_name: None,
            conditions: vec![],
        };
        let json = serde_json::to_string(&rule).unwrap();
        assert!(
            !json.contains("conditions"),
            "empty conditions should be skipped: {json}"
        );
    }

    #[test]
    fn rule_with_conditions_serializes_and_deserializes() {
        let rule = AgentTriggerRule {
            id: "with-conds".into(),
            label: "With Conds".into(),
            trigger: TriggerKind::Event {
                name: "note_created".into(),
                filter: None,
            },
            action: TriggerAction::Custom,
            enabled: true,
            custom_prompt: Some("Process urgent notes".into()),
            provider_name: None,
            conditions: vec![Condition::TagContains {
                tag: "urgent".into(),
            }],
        };
        let json = serde_json::to_string(&rule).unwrap();
        assert!(
            json.contains("conditions"),
            "rule with conditions must include field"
        );
        assert!(
            json.contains("tag_contains"),
            "condition type must be in JSON"
        );

        let parsed: AgentTriggerRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rule);
        assert_eq!(parsed.conditions.len(), 1);
    }

    #[test]
    fn backward_compat_missing_conditions_field() {
        // Older persisted rules had no "conditions" field — must deserialize
        // to an empty vec (serde default).
        let json = r#"{
            "id": "legacy-2",
            "label": "Legacy Rule",
            "trigger": {"type": "cron", "expression": "0 8 * * *"},
            "action": "daily_review",
            "enabled": true
        }"#;
        let parsed: AgentTriggerRule = serde_json::from_str(json).unwrap();
        assert!(
            parsed.conditions.is_empty(),
            "legacy rules must deserialize with empty conditions"
        );
    }
}
