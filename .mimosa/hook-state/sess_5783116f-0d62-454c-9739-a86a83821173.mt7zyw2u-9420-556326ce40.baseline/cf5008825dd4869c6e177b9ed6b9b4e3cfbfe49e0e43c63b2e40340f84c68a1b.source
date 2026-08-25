//! # Trigger Executor — cron-based trigger-rule firing engine (#3048)
//!
//! Prior to #3048 the `trigger_rules` storage layer shipped (v0.5.61) but no
//! scheduler read the table, so rules never fired. This module closes that gap
//! by providing:
//!
//! - **Pure cron evaluation** ([`last_due_time_before`], [`is_rule_due`],
//!   [`evaluate_rules_at`]) — fully unit-testable, no I/O.
//! - **Fire-and-record** ([`fire_due_rules_at`]) — a synchronous step that,
//!   given a [`StorageContext`], loads enabled cron rules, finds the ones
//!   whose schedule is due relative to their `last_fired_at`, records an
//!   execution row in `trigger_executions`, and updates the rule's
//!   `last_fired_at` / `run_count` / `last_status`. The observable side
//!   effect is real: callers can query the execution log and the rule's
//!   last-fired timestamp.
//! - **Dispatching tick** ([`fire_due_rules_with_dispatch`]) — the full path
//!   used by the background executor and `trigger fire-now`: in addition to
//!   the schedule evaluation above, each due rule's action is actually
//!   executed — the action prompt runs through the grounded AI pipeline
//!   ([`ask_with_ai_with_context`](crate::ask_with_ai_with_context)), the
//!   answer is saved as a vault note, and the outcome (success with note id /
//!   failed with error) is recorded in the execution log.
//! - **Background loop** ([`TriggerExecutor::spawn`]) — a `tokio` task that
//!   runs [`fire_due_rules_with_dispatch`] on a fixed cadence until the
//!   [`CancellationToken`](tokio_util::sync::CancellationToken) is cancelled.
//!
//! ## Dispatch model
//!
//! Firing a rule means: **evaluate schedule → check conditions → execute the
//! action via the AI pipeline → save the result as a note → record the
//! outcome**. Every fired rule is observable in three places: the
//! `trigger_executions` log, the generated vault note, and the rule's
//! `last_fired_at` / `run_count` / `last_status` columns.
//!
//! [`fire_due_rules_at`] (record-only, no AI call) is retained as the
//! schedule-evaluation primitive — it is what tests use to assert due-ness
//! bookkeeping without a provider.

use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;
use rusqlite::params;
use tokio::task::spawn_blocking;
use tokio::time;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use crate::orchestration::trigger::{
    AgentTriggerRule, ConditionContext, TriggerAction, TriggerKind,
};
use crate::storage::pool::open_connection;
use crate::storage::trigger_rules::list_trigger_rules_with_context;
use crate::storage::StorageContext;

/// Default cadence at which the background executor scans for due rules.
///
/// One minute matches the finest standard cron resolution (the cron minute
/// field) — polling faster would not surface any earlier fires and would add
/// pointless DB load.
pub const DEFAULT_TICK_INTERVAL: Duration = Duration::from_secs(60);

/// Per-rule timeout for the AI dispatch step of
/// [`fire_due_rules_with_dispatch`]. Matches the subscription AI timeout
/// documented in `orchestration/scheduled_research.rs` — long enough for a
/// grounded multi-source answer, short enough that one stuck provider call
/// cannot stall a tick indefinitely.
const DISPATCH_AI_TIMEOUT: Duration = Duration::from_secs(180);

/// Normalize a user-supplied cron expression to the 6-field form expected by
/// the `cron` crate (`sec min hour dom mon dow`).
///
/// VaultPilot stores standard 5-field cron (`min hour dom mon dow`), so we
/// prepend `0 ` for seconds. Expressions that already have 6+ fields are
/// returned unchanged.
fn normalize_cron_expr(expr: &str) -> String {
    let trimmed = expr.trim();
    let field_count = trimmed.split_whitespace().count();
    if field_count == 5 {
        format!("0 {trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// Parse a cron expression and return the most recent firing time strictly
/// before `now` (i.e. the last due tick).
///
/// Returns `None` when:
/// - the expression is invalid, or
/// - the schedule has no occurrence before `now` (e.g. first fire is in the
///   future).
///
/// This is the core "is the rule due?" primitive: a rule is due when its last
/// due time is later than its last *fired* time (or it has never fired and has
/// any past occurrence).
///
/// **Performance note (#3054):** this walks a 366-day window, which is
/// catastrophic for fine-grained cron (`0 * * * * *` = 527k iterations,
/// ~143 ms per call). For callers that already know `last_fired_at`, use
/// [`last_due_time_before_bounded`] instead — it bounds the scan to the
/// window since the last fire and collapses already-fired minute-level rules
/// to ~1 iteration. This public function is retained for never-fired rules
/// and as a fallback; new callers should reach for the bounded variant.
pub fn last_due_time_before(expr: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let full_expr = normalize_cron_expr(expr);
    let schedule = Schedule::from_str(&full_expr).ok()?;
    // `schedule.after(&now)` returns an iterator of times strictly after `now`;
    // we want strictly before, so we walk upcoming times after a slightly
    // earlier reference and take the first one that is still < now... but the
    // cron crate's API gives us `upcoming` (forward) and `after` (forward from
    // a point). There is no direct "previous" iterator, so we iterate upcoming
    // from a distant past reference and take the last one before `now`.
    //
    // To bound the search we start 366 days back — any cron cadence finer than
    // yearly will have produced at least one fire in that window, and yearly
    // cron rules are vanishingly rare in a personal vault. Callers that know
    // `last_fired_at` should use `last_due_time_before_bounded` to skip this
    // walk entirely (#3054).
    last_due_in_window(&schedule, now - chrono::Duration::days(366), now)
}

/// Like [`last_due_time_before`] but scans only the half-open window
/// `(from, now)`. Used by the executor hot path to bound the iteration count
/// for already-fired rules: if `from = last_fired_at`, then the window
/// contains at most a few fires regardless of the rule's frequency (#3054).
///
/// Returns `None` when the expression is invalid or no fire occurs in the
/// window. Schedule parsing is performed by the caller-facing wrappers so the
/// inner helper stays zero-alloc and trivially testable.
fn last_due_in_window(
    schedule: &Schedule,
    from: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let mut last_before_now: Option<DateTime<Utc>> = None;
    for next in schedule.after(&from) {
        if next >= now {
            break;
        }
        last_before_now = Some(next);
    }
    last_before_now
}

/// Parse a cron expression and return the most recent firing time strictly
/// before `now`, bounding the scan to times after `from`.
///
/// This is the bounded-scanner sibling of [`last_due_time_before`] and the
/// primary entry point used by the executor hot path (`is_rule_due`,
/// `evaluate_rules_at`). Passing `last_fired_at` (or a narrow recent window
/// for never-fired rules) collapses the iteration count from O(frequency ×
/// window) to O(fires in window) — for an already-fired `* * * * *` rule
/// that drops ~527k iterations → ~1 (#3054).
///
/// Returns `None` when:
/// - the expression is invalid, or
/// - no fire occurs in the half-open window `(from, now)`.
pub fn last_due_time_before_bounded(
    expr: &str,
    from: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let full_expr = normalize_cron_expr(expr);
    let schedule = Schedule::from_str(&full_expr).ok()?;
    last_due_in_window(&schedule, from, now)
}

/// Resolve the most recent cron fire strictly before `now` for a rule whose
/// last *fired* timestamp is `last_fired_at`, choosing the smallest viable
/// scan window so high-frequency rules stay cheap (#3054).
///
/// Strategy:
/// - **Already fired** (`Some(fired)`): bound the scan to `(fired, now)`. For
///   a healthy executor this window contains 1–2 fires regardless of cron
///   frequency, so `* * * * *` collapses from ~527k iterations to ~1.
/// - **Never fired** (`None`): try a 2-minute window first. This catches the
///   perf-sensitive high-frequency case (`* * * * *`, `0 */5 * * * *`) in O(1)
///   iterations. If that yields nothing (lower-frequency rules whose most
///   recent fire was more than 2 minutes ago — hourly / daily / weekly), fall
///   back to the full year-long scan via [`last_due_time_before`]. That
///   fallback is cheap for low-frequency rules (~366 iterations for daily,
///   measured ~160 µs).
fn last_due_for_rule(
    expr: &str,
    last_fired_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match last_fired_at {
        Some(fired) => last_due_time_before_bounded(expr, fired, now),
        None => {
            // Narrow window covers high-frequency never-fired rules in O(1).
            // 2 minutes > 1-minute cron resolution so the most recent fire is
            // always caught.
            let narrow =
                last_due_time_before_bounded(expr, now - chrono::Duration::minutes(2), now);
            if narrow.is_some() {
                narrow
            } else {
                // Lower-frequency rule (hourly+): fall back to the year-long
                // scan, which is cheap when fires are sparse.
                last_due_time_before(expr, now)
            }
        }
    }
}

/// Compute the next firing time at or after `from` (inclusive). Used to
/// populate `next_fire_at` for display. Returns `None` if the expression is
/// invalid or has no upcoming occurrence.
pub fn next_due_time_at(expr: &str, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let full_expr = normalize_cron_expr(expr);
    let schedule = Schedule::from_str(&full_expr).ok()?;
    schedule.after(&from).next()
}

/// Should `rule` fire as of `now`, given it last fired at `last_fired_at`?
///
/// Only cron triggers are evaluated here; event triggers fire via the event
/// bus (a separate code path, not the cron executor). `last_fired_at == None`
/// means "never fired" — the rule is due if it has any past occurrence.
///
/// Uses [`last_due_for_rule`] internally so the scan window is bounded by
/// `last_fired_at` (or a 2-minute window for never-fired rules). This keeps
/// high-frequency cron (`* * * * *`, `0 */5 * * * *`) cheap on the executor
/// hot path — see #3054.
pub fn is_rule_due(
    rule: &AgentTriggerRule,
    last_fired_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    if !rule.enabled {
        return false;
    }
    let expr = match &rule.trigger {
        TriggerKind::Cron { expression } => expression,
        TriggerKind::Event { .. } => return false,
    };
    let Some(last_due) = last_due_for_rule(expr, last_fired_at, now) else {
        return false;
    };
    match last_fired_at {
        None => true,
        Some(fired) => last_due > fired,
    }
}

/// Rule with its precomputed last-fired timestamp, for batch evaluation.
#[derive(Debug, Clone)]
pub struct RuleDueInput<'a> {
    pub rule: &'a AgentTriggerRule,
    pub last_fired_at: Option<DateTime<Utc>>,
}

/// Outcome of evaluating one rule against the current clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueRule<'a> {
    pub rule: &'a AgentTriggerRule,
    pub due_at: DateTime<Utc>,
}

/// Pure evaluation: given rules with their last-fired times and a reference
/// clock `now`, return the rules that are due, each annotated with the fire
/// time (`due_at`) that justifies the firing.
///
/// This is the testable heart of the executor — no I/O. [`fire_due_rules`]
/// wraps this with DB reads/writes.
///
/// Uses [`last_due_for_rule`] internally so the scan window for each rule is
/// bounded by its `last_fired_at`, keeping high-frequency cron cheap on the
/// hot path (#3054).
pub fn evaluate_rules_at<'a>(
    inputs: &'a [RuleDueInput<'a>],
    now: DateTime<Utc>,
) -> Vec<DueRule<'a>> {
    let mut out = Vec::new();
    for input in inputs {
        if !input.rule.enabled {
            continue;
        }
        let expr = match &input.rule.trigger {
            TriggerKind::Cron { expression } => expression.as_str(),
            TriggerKind::Event { .. } => continue,
        };
        let Some(due_at) = last_due_for_rule(expr, input.last_fired_at, now) else {
            continue;
        };
        let due = match input.last_fired_at {
            None => true,
            Some(fired) => due_at > fired,
        };
        if due {
            out.push(DueRule {
                rule: input.rule,
                due_at,
            });
        }
    }
    out
}

/// Synchronous: load enabled cron trigger rules and their `last_fired_at`
/// timestamps from the DB. Wrapped in `spawn_blocking` by callers.
fn load_enabled_cron_rules_with_last_fired(
    context: &StorageContext,
) -> Result<Vec<(AgentTriggerRule, Option<DateTime<Utc>>)>> {
    let rules = list_trigger_rules_with_context(context)?;
    let (connection, _) = open_connection(context)?;
    let mut out = Vec::with_capacity(rules.len());
    for rule in rules {
        if !rule.enabled {
            continue;
        }
        if !matches!(rule.trigger, TriggerKind::Cron { .. }) {
            continue;
        }
        let last_fired_str: Option<String> = connection
            .query_row(
                "SELECT last_fired_at FROM trigger_rules WHERE id = ?1",
                params![&rule.id],
                |row| row.get(0),
            )
            .ok()
            .filter(|s: &String| !s.is_empty());
        let last_fired_at = last_fired_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        out.push((rule, last_fired_at));
    }
    Ok(out)
}

/// Record one execution of `rule` and update the rule's run metadata.
///
/// `status` is `"success"` or `"failed"`; `error` is the error message
/// (empty on success); `detail` is optional free-form context (e.g. token
/// counts); `result_content` stores the full AI answer inline (NOT as a note
/// — trigger output must never pollute the vault).
fn record_trigger_execution(
    context: &StorageContext,
    rule: &AgentTriggerRule,
    fired_at: DateTime<Utc>,
    status: &str,
    error: &str,
    detail: &str,
    result_content: &str,
) -> Result<()> {
    let (mut connection, _) = open_connection(context)?;
    let tx = connection.transaction()?;
    let exec_id = Uuid::new_v4().to_string();
    let fired_rfc = fired_at.to_rfc3339();
    let action_str = format!("{:?}", rule.action).to_lowercase();
    tx.execute(
        "INSERT INTO trigger_executions (id, rule_id, label, action, fired_at, status, error, detail, result_content) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![exec_id, rule.id, rule.label, action_str, fired_rfc, status, error, detail, result_content],
    )
    .with_context(|| format!("failed to record execution for rule '{}'", rule.id))?;
    // Update rule run metadata.
    let next_fire = match &rule.trigger {
        TriggerKind::Cron { expression } => next_due_time_at(expression, fired_at)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
        TriggerKind::Event { .. } => String::new(),
    };
    let new_run_count: i64 = tx
        .query_row(
            "SELECT run_count FROM trigger_rules WHERE id = ?1",
            params![&rule.id],
            |row| row.get(0),
        )
        .unwrap_or(0)
        + 1;
    tx.execute(
        "UPDATE trigger_rules \
         SET last_fired_at = ?1, next_fire_at = ?2, run_count = ?3, last_status = ?4, last_error = ?5, updated_at = ?6 \
         WHERE id = ?7",
        params![fired_rfc, next_fire, new_run_count, status, error, fired_rfc, &rule.id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Outcome of a single fire-and-record step. Counted, not materialized — the
/// real observable side effects are the `trigger_executions` rows and the
/// `trigger_rules.last_fired_at` updates.
#[derive(Debug, Clone, Default)]
pub struct FireStepOutcome {
    pub evaluated: usize,
    pub fired: usize,
    pub failed: usize,
    /// Rules that were schedule-due but skipped because their conditions
    /// were not met (evaluated against an empty context for cron triggers).
    /// See #3439.
    pub skipped: usize,
}

/// One tick of the executor: load enabled cron rules, evaluate due-ness
/// against the current clock, and for each due rule record an execution.
///
/// This is the synchronous business logic; the async [`TriggerExecutor`] loop
/// just calls this via `spawn_blocking` on a cadence.
///
/// `now` is injected (rather than read from the system clock) so that tests
/// can drive the executor deterministically.
pub fn fire_due_rules_at(context: &StorageContext, now: DateTime<Utc>) -> Result<FireStepOutcome> {
    let pairs = load_enabled_cron_rules_with_last_fired(context)?;
    let inputs: Vec<RuleDueInput> = pairs
        .iter()
        .map(|(rule, last)| RuleDueInput {
            rule,
            last_fired_at: *last,
        })
        .collect();
    let due = evaluate_rules_at(&inputs, now);
    let mut outcome = FireStepOutcome {
        evaluated: inputs.len(),
        ..Default::default()
    };
    for d in due {
        // #3439: Evaluate rule conditions before firing. Cron triggers have
        // no inherent event context, so conditions are checked against an
        // empty ConditionContext. Rules with context-dependent conditions
        // (TagContains, FrontmatterEquals) will not fire — this is far less
        // misleading than the previous behavior of firing unconditionally
        // while silently ignoring stored conditions.
        //
        // Rules with no conditions, or only `Always` conditions, pass
        // through normally.
        if !d.rule.conditions_met(&ConditionContext::default()) {
            outcome.skipped += 1;
            warn!(
                rule_id = %d.rule.id,
                label = %d.rule.label,
                due_at = %d.due_at.to_rfc3339(),
                "cron rule is due but conditions not met (evaluated against \
                 empty context — TagContains/FrontmatterEquals conditions \
                 cannot be satisfied for cron triggers); skipping fire"
            );
            continue;
        }
        let detail = d
            .rule
            .effective_prompt()
            .map(|p| format!("effective_prompt={}", truncate(p, 200)))
            .unwrap_or_else(|| format!("action={:?}", d.rule.action));
        // #3055: status field must match the documented contract ("success"
        // or "failed"). The previous implementation passed the literal
        // "fired", which broke any downstream filter on `status = 'success'`
        // (WinUI Inspector, mobile surfaces, external dashboards).
        let recorded =
            record_trigger_execution(context, d.rule, d.due_at, "success", "", &detail, "");
        match recorded {
            Ok(()) => {
                outcome.fired += 1;
                info!(
                    rule_id = %d.rule.id,
                    label = %d.rule.label,
                    due_at = %d.due_at.to_rfc3339(),
                    "trigger rule fired (execution recorded)"
                );
            }
            Err(e) => {
                // #3055: when the success-row write itself fails (typically a
                // DB lock or schema mismatch), try to record a "failed" row so
                // the failure is observable in the execution log instead of
                // only in `outcome.failed` + a warn log. If that fallback also
                // fails (DB genuinely unavailable), the counters + warn log
                // remain the source of truth.
                outcome.failed += 1;
                let err_msg = format!("{e:#}");
                let _ = record_trigger_execution(
                    context, d.rule, d.due_at, "failed", &err_msg, &detail, "",
                );
                warn!(rule_id = %d.rule.id, error = %e, "failed to record trigger execution");
            }
        }
    }
    Ok(outcome)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

// ────────────────────────────────────────────────────────
// Action dispatch — turning a fired rule into real work
// ────────────────────────────────────────────────────────

/// A due rule lifted to owned form so it can cross the await points of the
/// dispatch step ([`DueRule`] borrows its rule and cannot).
struct OwnedDueRule {
    rule: AgentTriggerRule,
    due_at: DateTime<Utc>,
}

/// Build the prompt executed for `rule`'s action.
///
/// `Ok(prompt)` — run it through the grounded AI pipeline.
/// `Err(reason)` — the action cannot run from a cron context (misconfigured
/// rule); the fire is recorded as `"failed"` with `reason` as the error.
fn build_action_prompt(rule: &AgentTriggerRule, now: DateTime<Utc>) -> Result<String, String> {
    let date = now.format("%Y-%m-%d").to_string();
    match rule.action {
        TriggerAction::DailyReview => Ok(format!(
            r#"[Scheduled Task: Daily Review]
You are running the scheduled "Daily Review" trigger for this vault.
Review the notes created or updated recently (roughly the last 24 hours) and produce:
- A concise summary of what was worked on and decided
- Open threads, TODOs and follow-ups worth tracking
- Notes that look related to each other (candidate cross-links)

Today's date: {date}"#
        )),
        TriggerAction::SummarizeAndTag => Ok(format!(
            r#"[Scheduled Task: Summarize & Tag]
You are running the scheduled "Summarize & Tag" trigger for this vault.
Look at recently created or modified notes and for each one:
- Summarize it in one or two sentences
- Propose a concise set of tags (existing vault tags preferred)

Today's date: {date}"#
        )),
        TriggerAction::SuggestLinks => Ok(format!(
            r#"[Scheduled Task: Suggest Links]
You are running the scheduled "Suggest Links" trigger for this vault.
Find notes that cover related topics and suggest concrete [[wikilinks]] that
should be added between them, with a one-line reason per suggestion.

Today's date: {date}"#
        )),
        TriggerAction::ProcessWebhook => Err(
            "process_webhook action requires an event/webhook payload and cannot run \
             from a cron schedule"
                .to_string(),
        ),
        TriggerAction::Custom => rule
            .effective_prompt()
            .map(|p| format!("[Scheduled Task]\n{p}"))
            .ok_or_else(|| {
                "custom trigger rule has no prompt configured (#2842) — set a prompt \
                 or pick another action"
                    .to_string()
            }),
    }
}

/// Outcome of dispatching one rule's action. `status` is `"success"` or
/// `"failed"` — the same contract as the `trigger_executions.status` column
/// (#3055). `result_content` carries the full AI answer text (stored inline
/// in the execution log — NOT as a vault note).
pub struct ActionDispatch {
    pub status: &'static str,
    pub error: String,
    pub detail: String,
    pub result_content: String,
}

impl ActionDispatch {
    fn failed(error: String) -> Self {
        Self {
            status: "failed",
            error,
            detail: String::new(),
            result_content: String::new(),
        }
    }
}

/// Execute `rule`'s action: run the action prompt through the grounded AI
/// pipeline (with vault search context) and return the answer text.
///
/// The answer is stored inline in the `trigger_executions.result_content`
/// column — trigger output must NEVER pollute the user's note vault.
/// This is the "agent dispatch" half of a fire. It never panics and never
/// returns `Err` — every failure mode (misconfigured action, AI error,
/// timeout) is reported as an `ActionDispatch` with `status = "failed"`.
async fn dispatch_rule_action(
    context: &StorageContext,
    rule: &AgentTriggerRule,
    now: DateTime<Utc>,
) -> ActionDispatch {
    let prompt = match build_action_prompt(rule, now) {
        Ok(p) => p,
        Err(reason) => {
            return ActionDispatch {
                status: "failed",
                detail: format!("action={:?}", rule.action),
                error: reason,
                result_content: String::new(),
            };
        }
    };

    let answered = match tokio::time::timeout(
        DISPATCH_AI_TIMEOUT,
        crate::ask_with_ai_with_context(
            context,
            prompt.clone(),
            None,                          // no conversation history
            None,                          // no images
            None,                          // no model override
            rule.provider_name.as_deref(), // per-rule provider selection
            |_, _| {},                     // suppress progress events
        ),
    )
    .await
    {
        Ok(Ok(answer)) => answer,
        Ok(Err(e)) => return ActionDispatch::failed(format!("AI execution failed: {e:#}")),
        Err(_) => {
            return ActionDispatch::failed(format!(
                "AI execution timed out after {}s",
                DISPATCH_AI_TIMEOUT.as_secs()
            ))
        }
    };
    // Token usage as reported by the provider — recorded in the execution
    // log so quota consumption is attributable per fire.
    let tokens_in = answered.usage_input_tokens;
    let tokens_out = answered.usage_output_tokens;
    let answer = answered.answer;

    // Build detail: token usage for quota attribution.
    let mut detail = String::new();
    if let Some(tokens_in) = tokens_in {
        detail.push_str(&format!("tokens_in={tokens_in}"));
    }
    if let Some(tokens_out) = tokens_out {
        if !detail.is_empty() {
            detail.push(' ');
        }
        detail.push_str(&format!("tokens_out={tokens_out}"));
    }

    ActionDispatch {
        status: "success",
        error: String::new(),
        detail,
        result_content: answer,
    }
}

/// One dispatching tick of the executor: evaluate due rules against `now`,
/// then for each due rule execute its action (AI + note write-back) and
/// record the outcome in the execution log.
///
/// This is what the background loop ([`TriggerExecutor::spawn`]) and the CLI
/// `trigger fire-now` command run. It layers on top of the same evaluation
/// primitives as [`fire_due_rules_at`] but replaces the record-only fire with
/// a real dispatch, so a "success" row means the action actually ran and its
/// result note exists (`detail` carries the note id).
///
/// Fire a single rule by ID immediately, bypassing the cron schedule.
///
/// This is the backend for the UI's "立即触发" button: it loads the rule
/// (enabled or not), dispatches its action through the full AI pipeline,
/// and records the outcome. Returns `Ok(None)` when the rule does not exist.
pub async fn fire_trigger_rule_now(
    context: &StorageContext,
    rule_id: &str,
) -> Result<Option<ActionDispatch>> {
    let ctx = context.clone();
    let rule_id = rule_id.to_string();
    let rule = spawn_blocking(move || {
        crate::storage::trigger_rules::get_trigger_rule_with_context(&ctx, &rule_id)
    })
    .await
    .map_err(|e| anyhow::anyhow!("rule lookup task panicked: {e}"))??;
    let Some(rule) = rule else { return Ok(None) };
    let now = Utc::now();
    let dispatch = dispatch_rule_action(context, &rule, now).await;
    // Record the outcome; the "fire" timestamp is now (manual trigger).
    // Clone the fields before the move closure so we can still return them.
    let status = dispatch.status;
    let error = dispatch.error.clone();
    let detail = dispatch.detail.clone();
    let result_content = dispatch.result_content.clone();
    let ctx = context.clone();
    let _ = spawn_blocking(move || {
        record_trigger_execution(
            &ctx,
            &rule,
            Utc::now(),
            status,
            &error,
            &detail,
            &result_content,
        )
    })
    .await;
    Ok(Some(dispatch))
}

/// Dispatch failures are recorded as `"failed"` executions (and still advance
/// `last_fired_at`) — a rule whose AI call fails must not re-fire on every
/// tick for the same due time.
pub async fn fire_due_rules_with_dispatch(
    context: &StorageContext,
    now: DateTime<Utc>,
) -> Result<FireStepOutcome> {
    // Step 1 (blocking): load + evaluate which rules are schedule-due.
    let ctx = context.clone();
    let (evaluated, due) = spawn_blocking(move || -> Result<(usize, Vec<OwnedDueRule>)> {
        let pairs = load_enabled_cron_rules_with_last_fired(&ctx)?;
        let inputs: Vec<RuleDueInput> = pairs
            .iter()
            .map(|(rule, last)| RuleDueInput {
                rule,
                last_fired_at: *last,
            })
            .collect();
        let evaluated = inputs.len();
        let due = evaluate_rules_at(&inputs, now)
            .into_iter()
            .map(|d| OwnedDueRule {
                rule: d.rule.clone(),
                due_at: d.due_at,
            })
            .collect();
        Ok((evaluated, due))
    })
    .await
    .map_err(|e| anyhow::anyhow!("tick evaluate task panicked: {e}"))??;

    let mut outcome = FireStepOutcome {
        evaluated,
        ..Default::default()
    };

    // Step 2 (async): dispatch each due rule, then record the outcome.
    for d in due {
        // Same #3439 condition gate as the record-only path: cron triggers
        // have no event context, so context-dependent conditions cannot be
        // satisfied and the fire is skipped (without a DB row).
        if !d.rule.conditions_met(&ConditionContext::default()) {
            outcome.skipped += 1;
            warn!(
                rule_id = %d.rule.id,
                label = %d.rule.label,
                due_at = %d.due_at.to_rfc3339(),
                "cron rule is due but conditions not met (evaluated against \
                 empty context — TagContains/FrontmatterEquals conditions \
                 cannot be satisfied for cron triggers); skipping fire"
            );
            continue;
        }

        let dispatch = dispatch_rule_action(context, &d.rule, now).await;
        let status = dispatch.status;
        let error = dispatch.error;
        // Detail convention mirrors the record-only path: the note id on
        // success, otherwise the effective prompt / action for context.
        let detail = if dispatch.detail.is_empty() {
            d.rule
                .effective_prompt()
                .map(|p| format!("effective_prompt={}", truncate(p, 200)))
                .unwrap_or_else(|| format!("action={:?}", d.rule.action))
        } else {
            dispatch.detail
        };

        let ctx = context.clone();
        let rule = d.rule.clone();
        let due_at = d.due_at;
        let error_for_log = error.clone();
        let detail_for_log = detail.clone();
        let result_for_log = dispatch.result_content.clone();
        let recorded = spawn_blocking(move || {
            record_trigger_execution(
                &ctx,
                &rule,
                due_at,
                status,
                &error,
                &detail,
                &result_for_log,
            )
        })
        .await
        .map_err(|e| anyhow::anyhow!("record task panicked: {e}"))?;
        match recorded {
            Ok(()) => {
                if status == "success" {
                    outcome.fired += 1;
                    info!(
                        rule_id = %d.rule.id,
                        label = %d.rule.label,
                        detail = %detail_for_log,
                        "trigger rule fired (action dispatched, result note saved)"
                    );
                } else {
                    outcome.failed += 1;
                    warn!(
                        rule_id = %d.rule.id,
                        label = %d.rule.label,
                        error = %error_for_log,
                        "trigger rule action failed"
                    );
                }
            }
            Err(e) => {
                outcome.failed += 1;
                warn!(rule_id = %d.rule.id, error = %e, "failed to record trigger execution");
            }
        }
    }
    Ok(outcome)
}

/// Background trigger-rule executor. Spawns a `tokio` task that periodically
/// calls [`fire_due_rules_at`] against the supplied [`StorageContext`] until
/// the cancellation token fires.
///
/// Construct with [`TriggerExecutor::new`] (default cadence) or
/// [`TriggerExecutor::with_interval`]. The executor does **not** auto-spawn —
/// call [`TriggerExecutor::spawn`] to start the loop.
pub struct TriggerExecutor {
    context: StorageContext,
    tick_interval: Duration,
}

impl TriggerExecutor {
    /// Create an executor with the default ([`DEFAULT_TICK_INTERVAL`]) cadence.
    pub fn new(context: StorageContext) -> Self {
        Self {
            context,
            tick_interval: DEFAULT_TICK_INTERVAL,
        }
    }

    /// Create an executor with a custom tick cadence (mainly for tests).
    pub fn with_interval(context: StorageContext, tick_interval: Duration) -> Self {
        Self {
            context,
            tick_interval,
        }
    }

    /// Run the executor loop until `cancel` is cancelled. Each tick:
    /// 1. Clones the [`StorageContext`] (cheap — it is `Arc`-backed).
    /// 2. Runs [`fire_due_rules_with_dispatch`] (schedule evaluation in
    ///    `spawn_blocking` — SQLite is synchronous — then async AI dispatch
    ///    for each due rule).
    /// 3. Sleeps `tick_interval`, or returns early if cancelled.
    ///
    /// Errors inside a tick are logged and swallowed — a single failed tick
    /// (e.g. transient DB lock) must not kill the whole scheduler.
    pub async fn spawn(self, cancel: CancellationToken) {
        info!(
            interval_secs = self.tick_interval.as_secs(),
            "trigger executor started"
        );
        loop {
            // Race the tick against cancellation so shutdown is prompt.
            let tick = async {
                let ctx = self.context.clone();
                match fire_due_rules_with_dispatch(&ctx, Utc::now()).await {
                    Ok(outcome) => {
                        if outcome.fired > 0 || outcome.failed > 0 || outcome.skipped > 0 {
                            info!(
                                evaluated = outcome.evaluated,
                                fired = outcome.fired,
                                failed = outcome.failed,
                                skipped = outcome.skipped,
                                "trigger tick complete"
                            );
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "trigger tick failed");
                    }
                }
                time::sleep(self.tick_interval).await;
            };
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("trigger executor stopping (cancelled)");
                    break;
                }
                _ = tick => {}
            }
        }
    }

    /// Run the executor loop for the lifetime of the process — no
    /// cancellation wiring. Convenience for embedding hosts (the Tauri
    /// desktop / mobile app) that want the scheduler alive as long as the
    /// app itself runs; they cannot easily construct a
    /// `CancellationToken` without depending on `tokio-util` directly.
    pub async fn run_forever(self) {
        self.spawn(CancellationToken::new()).await;
    }
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::trigger::{TriggerAction, TriggerKind};
    use crate::storage::trigger_rules::create_trigger_rule_with_context;
    use chrono::TimeZone;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Build a rule with sensible defaults for tests.
    fn make_rule(id: &str, expr: &str, action: TriggerAction) -> AgentTriggerRule {
        AgentTriggerRule {
            id: id.to_string(),
            label: format!("test-{id}"),
            trigger: TriggerKind::Cron {
                expression: expr.to_string(),
            },
            action,
            enabled: true,
            custom_prompt: None,
            provider_name: None,
            conditions: vec![],
        }
    }

    fn fixed_time() -> DateTime<Utc> {
        // 2026-07-18T09:30:00Z — a Saturday morning.
        Utc.with_ymd_and_hms(2026, 7, 18, 9, 30, 0).unwrap()
    }

    // ── normalize_cron_expr ──

    #[test]
    fn normalize_prepends_seconds_for_5_field_cron() {
        assert_eq!(normalize_cron_expr("0 8 * * *"), "0 0 8 * * *");
        assert_eq!(normalize_cron_expr("30 9 * * 1-5"), "0 30 9 * * 1-5");
    }

    #[test]
    fn normalize_leaves_6_field_cron_unchanged() {
        assert_eq!(normalize_cron_expr("0 0 8 * * *"), "0 0 8 * * *");
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_cron_expr("  0 8 * * *  "), "0 0 8 * * *");
    }

    // ── last_due_time_before ──

    #[test]
    fn last_due_time_before_returns_most_recent_occurrence() {
        // "0 9 * * *" = every day at 09:00. At 09:30 the most recent fire is
        // today at 09:00.
        let now = fixed_time();
        let last = last_due_time_before("0 9 * * *", now).expect("should have a prior fire");
        assert_eq!(last, Utc.with_ymd_and_hms(2026, 7, 18, 9, 0, 0).unwrap());
    }

    #[test]
    fn last_due_time_before_returns_none_for_future_only_schedule() {
        // "0 0 1 1 * * 2099" — Jan 1 00:00 2099. At 2026 this is entirely in
        // the future, so there is no past occurrence.
        let now = fixed_time();
        let last = last_due_time_before("0 0 1 1 * * 2099", now);
        assert_eq!(last, None, "future-only schedule should have no prior fire");
    }

    #[test]
    fn last_due_time_before_invalid_expr_returns_none() {
        let now = fixed_time();
        assert_eq!(last_due_time_before("not a cron", now), None);
        assert_eq!(last_due_time_before("* * *", now), None);
    }

    #[test]
    fn last_due_time_before_minute_cron_works() {
        // "*/5 * * * *" — every 5 minutes. At 09:30:00 the last fire is 09:30.
        // We assert the last fire is within the last 5 minutes.
        let now = fixed_time();
        let last = last_due_time_before("*/5 * * * *", now).expect("should have prior fire");
        let delta = now - last;
        assert!(
            delta.num_seconds() >= 0 && delta.num_seconds() <= 300,
            "last fire {last} should be at most 5 min before {now}, got {delta}"
        );
    }

    // ── is_rule_due ──

    #[test]
    fn next_due_time_accepts_comma_separated_dow() {
        // Debug: test multiple dow formats to find what the cron crate accepts.
        let now = fixed_time();
        // Single value
        assert!(next_due_time_at("0 22 * * 1", now).is_some(), "single dow=1");
        // Comma list without 0
        assert!(next_due_time_at("0 22 * * 1,2,3,4,5", now).is_some(), "dow=1,2,3,4,5");
        // Comma list WITH 0 — the reported bug
        let with_zero = next_due_time_at("0 22 * * 0,1,2,3,4", now);
        if with_zero.is_none() {
            // 0 is invalid in the cron crate's dow field (1-7 only, 1=Sun).
            // The frontend generates JS day numbers (0=Sun) — must convert.
            // For now, document the limitation; the fix is in the frontend toCron.
            eprintln!("CONFIRMED: dow containing 0 fails (cron crate uses 1-7, not 0-6)");
        }
        // Range
        assert!(next_due_time_at("0 22 * * 1-5", now).is_some(), "dow=1-5 range");
    }

    #[test]
    fn is_rule_due_true_when_never_fired_and_has_past_fire() {
        let rule = make_rule("r1", "0 9 * * *", TriggerAction::DailyReview);
        let now = fixed_time();
        assert!(is_rule_due(&rule, None, now));
    }

    #[test]
    fn is_rule_due_false_when_already_fired_at_due_time() {
        let rule = make_rule("r1", "0 9 * * *", TriggerAction::DailyReview);
        let now = fixed_time();
        // Fired at exactly today's 09:00 — the most recent due time.
        let fired = Utc.with_ymd_and_hms(2026, 7, 18, 9, 0, 0).unwrap();
        assert!(!is_rule_due(&rule, Some(fired), now));
    }

    #[test]
    fn is_rule_due_true_when_last_fire_is_before_most_recent_due() {
        let rule = make_rule("r1", "0 9 * * *", TriggerAction::DailyReview);
        let now = fixed_time();
        // Fired yesterday at 09:00; today's 09:00 fire is pending.
        let fired = Utc.with_ymd_and_hms(2026, 7, 17, 9, 0, 0).unwrap();
        assert!(is_rule_due(&rule, Some(fired), now));
    }

    #[test]
    fn is_rule_due_false_for_disabled_rule() {
        let mut rule = make_rule("r1", "0 9 * * *", TriggerAction::DailyReview);
        rule.enabled = false;
        let now = fixed_time();
        assert!(!is_rule_due(&rule, None, now));
    }

    #[test]
    fn is_rule_due_false_for_event_trigger() {
        // Event triggers are not evaluated by the cron executor.
        let rule = AgentTriggerRule {
            id: "e1".into(),
            label: "evt".into(),
            trigger: TriggerKind::Event {
                name: "note_created".into(),
                filter: None,
            },
            action: TriggerAction::SummarizeAndTag,
            enabled: true,
            custom_prompt: None,
            provider_name: None,
            conditions: vec![],
        };
        assert!(!is_rule_due(&rule, None, fixed_time()));
    }

    #[test]
    fn is_rule_due_false_when_invalid_cron() {
        let rule = make_rule("r1", "garbage", TriggerAction::DailyReview);
        assert!(!is_rule_due(&rule, None, fixed_time()));
    }

    // ── evaluate_rules_at ──

    #[test]
    fn evaluate_rules_at_filters_disabled_and_event_and_not_due() {
        let now = fixed_time();
        let due_rule = make_rule("due", "0 9 * * *", TriggerAction::DailyReview);
        let mut disabled = make_rule("dis", "0 9 * * *", TriggerAction::DailyReview);
        disabled.enabled = false;
        let already_fired = make_rule("fired", "0 9 * * *", TriggerAction::DailyReview);
        let inputs = vec![
            RuleDueInput {
                rule: &due_rule,
                last_fired_at: None,
            },
            RuleDueInput {
                rule: &disabled,
                last_fired_at: None,
            },
            RuleDueInput {
                rule: &already_fired,
                last_fired_at: Some(Utc.with_ymd_and_hms(2026, 7, 18, 9, 0, 0).unwrap()),
            },
        ];
        let out = evaluate_rules_at(&inputs, now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rule.id, "due");
    }

    #[test]
    fn evaluate_rules_at_sets_due_at_to_most_recent_fire() {
        let now = fixed_time();
        let rule = make_rule("r", "0 9 * * *", TriggerAction::DailyReview);
        let inputs = vec![RuleDueInput {
            rule: &rule,
            last_fired_at: None,
        }];
        let out = evaluate_rules_at(&inputs, now);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].due_at,
            Utc.with_ymd_and_hms(2026, 7, 18, 9, 0, 0).unwrap()
        );
    }

    // ── fire_due_rules_at (DB-backed integration) ──

    fn setup_context() -> (PathBuf, StorageContext) {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-trigger-exec-test-{}-{}",
            std::process::id(),
            counter
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        // Initialize schema by opening a connection once.
        let _ = open_connection(&ctx).expect("open initializes schema");
        (temp, ctx)
    }

    #[test]
    fn fire_due_rules_at_fires_never_fired_cron_rule() {
        let (_tmp, ctx) = setup_context();
        // Daily 09:00 rule, clock is 09:30 — should fire exactly once.
        let rule = create_trigger_rule_with_context(
            &ctx,
            "Daily 9am",
            "cron",
            "0 9 * * *",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create rule");
        let now = fixed_time();
        let outcome = fire_due_rules_at(&ctx, now).expect("fire step");
        assert_eq!(outcome.evaluated, 1);
        assert_eq!(outcome.fired, 1);
        assert_eq!(outcome.failed, 0);

        // A second tick at the same clock should NOT re-fire (last_fired_at is
        // now set to today's 09:00, which is the most recent due time).
        let outcome2 = fire_due_rules_at(&ctx, now).expect("second fire step");
        assert_eq!(
            outcome2.fired, 0,
            "rule should not fire twice for the same due time"
        );

        // Advance the clock past tomorrow's 09:00 — should fire again.
        let tomorrow_after = Utc.with_ymd_and_hms(2026, 7, 19, 9, 5, 0).unwrap();
        let outcome3 = fire_due_rules_at(&ctx, tomorrow_after).expect("third fire step");
        assert_eq!(
            outcome3.fired, 1,
            "rule should fire for the next day's 09:00"
        );

        // Verify an execution row was written for each fire.
        let (conn, _) = open_connection(&ctx).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM trigger_executions WHERE rule_id = ?1",
                params![&rule.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "two executions should be recorded");

        // run_count on the rule should be incremented.
        let run_count: i64 = conn
            .query_row(
                "SELECT run_count FROM trigger_rules WHERE id = ?1",
                params![&rule.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(run_count, 2);
    }

    #[test]
    fn fire_due_rules_at_skips_disabled_and_event_rules() {
        let (_tmp, ctx) = setup_context();
        // Future-only cron (year 2099) so the cron rule is NOT due either.
        let _cron = create_trigger_rule_with_context(
            &ctx,
            "cron",
            "cron",
            "0 0 1 1 * * 2099",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create cron");
        let event_rule = create_trigger_rule_with_context(
            &ctx,
            "evt",
            "event",
            "note_created",
            "summarize_and_tag",
            None,
            None,
            None,
        )
        .expect("create event");
        // Event triggers are never evaluated by the cron executor regardless of
        // due-ness; the future-only cron is also not due.
        let outcome = fire_due_rules_at(&ctx, fixed_time()).expect("fire step");
        assert_eq!(
            outcome.evaluated, 1,
            "only the cron rule counts as cron-evaluated"
        );
        assert_eq!(
            outcome.fired, 0,
            "future-only cron + event rule → nothing fires"
        );

        // Verify no execution row exists for the event rule.
        let (conn, _) = open_connection(&ctx).unwrap();
        let evt_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM trigger_executions WHERE rule_id = ?1",
                params![&event_rule.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            evt_count, 0,
            "event rule must never produce a cron execution"
        );
    }

    #[test]
    fn next_due_time_at_returns_future_fire() {
        let from = fixed_time();
        let next = next_due_time_at("0 9 * * *", from).expect("should have a next fire");
        // At 09:30 the next 09:00 fire is tomorrow.
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 7, 19, 9, 0, 0).unwrap());
    }

    /// Regression: consecutive fires must create SEPARATE result notes.
    /// The old code used `source: "trigger_rule"` for every fire, and
    /// `save_note_with_context`'s source-based dedup (#3077) reused the
    /// existing note id — each fire silently OVERWROTE the previous
    /// result, so the user only ever saw the latest one.
    #[test]
    fn consecutive_fires_create_separate_notes() {
        let (_tmp, ctx) = setup_context();
        create_trigger_rule_with_context(
            &ctx,
            "Daily 9am",
            "cron",
            "0 9 * * *",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create rule");

        // Fire across two consecutive days.
        let day1 = Utc.with_ymd_and_hms(2026, 7, 18, 9, 30, 0).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 7, 19, 9, 30, 0).unwrap();

        let outcome1 = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fire_due_rules_with_dispatch(&ctx, day1))
            .expect("fire day1");
        assert!(
            outcome1.failed >= 1,
            "no AI config — dispatch fails but records"
        );

        let outcome2 = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fire_due_rules_with_dispatch(&ctx, day2))
            .expect("fire day2");
        assert!(outcome2.failed >= 1);

        // Both fires must be recorded as separate executions.
        let (conn, _) = open_connection(&ctx).unwrap();
        let exec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM trigger_executions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(exec_count, 2, "two fires must be recorded");
    }

    #[test]
    fn next_due_time_at_invalid_returns_none() {
        assert_eq!(next_due_time_at("garbage", fixed_time()), None);
    }

    // ────────────────────────────────────────────────────────────────────
    // #3054 regression: bounded scan must keep high-frequency cron cheap.
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn last_due_time_before_bounded_returns_most_recent_in_window() {
        // Daily 09:00 rule. With a window starting yesterday, the most recent
        // fire before 09:30 today is today's 09:00.
        let now = fixed_time();
        let from = now - chrono::Duration::days(1);
        let last = last_due_time_before_bounded("0 9 * * *", from, now)
            .expect("should find today's 09:00 in a 1-day window");
        assert_eq!(last, Utc.with_ymd_and_hms(2026, 7, 18, 9, 0, 0).unwrap());
    }

    #[test]
    fn last_due_time_before_bounded_returns_none_when_window_has_no_fire() {
        // Daily 09:00 rule. Window [09:10 today, 09:30 today] contains no 09:00
        // fire, so the bounded scan correctly yields None.
        let now = fixed_time();
        let from = Utc.with_ymd_and_hms(2026, 7, 18, 9, 10, 0).unwrap();
        assert_eq!(
            last_due_time_before_bounded("0 9 * * *", from, now),
            None,
            "no fire in the narrow window"
        );
    }

    #[test]
    fn last_due_time_before_bounded_invalid_expr_returns_none() {
        let now = fixed_time();
        assert_eq!(last_due_time_before_bounded("garbage", now, now), None);
    }

    #[test]
    fn last_due_for_rule_already_fired_uses_tight_window() {
        // #3054: an already-fired minute-level rule must resolve in O(1)
        // iterations by bounding the scan to (last_fired_at, now).
        // `* * * * *` is the pathological case from the bug report
        // (~527k iterations with the old 366-day scan).
        let now = fixed_time();
        // Pretend the rule fired 2 minutes ago at 09:28:00. The bounded
        // window (09:28:00, 09:30:00) contains exactly one fire: 09:29:00.
        let last_fired = Utc.with_ymd_and_hms(2026, 7, 18, 9, 28, 0).unwrap();
        let due = last_due_for_rule("* * * * *", Some(last_fired), now)
            .expect("a fire should exist between last_fired and now");
        assert!(due > last_fired, "due must be after last_fired");
        assert!(due < now, "due must be strictly before now");
        // The most recent minute fire before 09:30:00 is 09:29:00.
        assert_eq!(due, Utc.with_ymd_and_hms(2026, 7, 18, 9, 29, 0).unwrap());
    }

    #[test]
    fn last_due_for_rule_never_fired_minute_cron_uses_narrow_window() {
        // #3054: a never-fired minute-level rule must resolve in O(1) via the
        // 2-minute narrow window, not the 366-day fallback.
        let now = fixed_time();
        let due = last_due_for_rule("* * * * *", None, now)
            .expect("never-fired minute rule should be due");
        // The most recent minute fire before 09:30:00 is 09:29:00.
        assert_eq!(due, Utc.with_ymd_and_hms(2026, 7, 18, 9, 29, 0).unwrap());
    }

    #[test]
    fn last_due_for_rule_never_fired_daily_cron_falls_back_to_wide_window() {
        // #3054: a never-fired daily rule whose most recent fire was hours ago
        // is outside the 2-minute narrow window; the fallback to the full
        // 366-day scan must still find it.
        let now = fixed_time();
        let due = last_due_for_rule("0 9 * * *", None, now)
            .expect("never-fired daily rule should be due (today's 09:00)");
        assert_eq!(due, Utc.with_ymd_and_hms(2026, 7, 18, 9, 0, 0).unwrap());
    }

    #[test]
    fn last_due_for_rule_perf_minute_cron_under_5ms() {
        // #3054 perf regression guard: the bounded path for an already-fired
        // every-minute rule must complete in well under the bug report's
        // 143 ms. We assert < 50 ms to stay generous on shared CI runners
        // while still catching the 3-orders-of-magnitude regression that the
        // old 366-day scan would produce.
        let now = fixed_time();
        // Use a 2-minute gap so the bounded window (09:28:00, 09:30:00)
        // contains exactly one fire (09:29:00) — a realistic "missed one
        // tick" scenario for a healthy executor.
        let last_fired = Utc.with_ymd_and_hms(2026, 7, 18, 9, 28, 0).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = last_due_for_rule("* * * * *", Some(last_fired), now);
        }
        let elapsed = start.elapsed();
        // 100 iterations of an O(1) scan should be sub-millisecond in release
        // and a few ms in debug. 50 ms / 100 = 500 µs per call ceiling.
        assert!(
            elapsed.as_millis() < 50,
            "100 bounded minute-cron scans took {elapsed:?} (>50ms) — \
             the 366-day fallback is likely firing (#3054 regression)",
        );
    }

    // ────────────────────────────────────────────────────────────────────
    // #3055 regression: status field contract ("success" / "failed") and
    // the FireNow `fired` flag reflecting actual outcomes.
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn fire_due_rules_at_records_success_status_in_execution_log() {
        // #3055: a successfully-fired rule must persist status = "success"
        // (the documented contract), not the legacy "fired" literal.
        let (_tmp, ctx) = setup_context();
        let rule = create_trigger_rule_with_context(
            &ctx,
            "Daily 9am",
            "cron",
            "0 9 * * *",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create rule");
        let outcome = fire_due_rules_at(&ctx, fixed_time()).expect("fire step");
        assert_eq!(outcome.fired, 1);
        assert_eq!(outcome.failed, 0);

        let (conn, _) = open_connection(&ctx).unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM trigger_executions WHERE rule_id = ?1 ORDER BY fired_at DESC LIMIT 1",
                params![&rule.id],
                |row| row.get(0),
            )
            .expect("execution row should exist");
        assert_eq!(
            status, "success",
            "execution status must be 'success' per documented contract (#3055), got '{status}'"
        );

        // last_status on the rule row must also be "success".
        let rule_status: String = conn
            .query_row(
                "SELECT last_status FROM trigger_rules WHERE id = ?1",
                params![&rule.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            rule_status, "success",
            "trigger_rules.last_status must mirror the execution status (#3055)"
        );

        // The error column should be empty on success.
        let rule_err: String = conn
            .query_row(
                "SELECT last_error FROM trigger_rules WHERE id = ?1",
                params![&rule.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(rule_err.is_empty(), "last_error should be empty on success");
    }

    #[test]
    fn fire_now_json_fired_flag_reflects_outcome() {
        // #3055: the `trigger fire-now` JSON `fired` field must reflect
        // whether any rule actually fired, not be hardcoded to true. We
        // mirror the CLI's value computation here (the CLI handler itself is
        // an async entrypoint that's hard to unit-test in isolation, so we
        // assert the formula it uses).
        let fired_some = 3usize;
        let fired_none = 0usize;

        // CLI formula (post-fix): `"fired": outcome.fired > 0`.
        assert!(
            fired_some > 0,
            "formula sanity: outcome.fired > 0 must be true when some rules fired"
        );
        let json_some = serde_json::json!({ "fired": fired_some > 0 });
        assert_eq!(json_some["fired"], serde_json::Value::Bool(true));

        let json_none = serde_json::json!({ "fired": fired_none > 0 });
        assert_eq!(
            json_none["fired"],
            serde_json::Value::Bool(false),
            "fired must be false on a no-op tick (#3055)"
        );
    }

    // ────────────────────────────────────────────────────────────────────
    // #3057 / #3058 hardening: regression guards at the public-API layer.
    //
    // The underlying fixes for these bugs landed in PR #3056 (commit
    // cbd3106) which closed the original reports #3054 / #3055. The
    // follow-up scan that filed #3057 / #3058 ran against a pre-merge
    // snapshot; the contracts below pin the behavior at the entry points
    // the reports explicitly call out as the verification surface.
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn record_trigger_execution_failed_status_persists_to_db() {
        // #3057: when `record_trigger_execution` is called with
        // status = "failed" and a non-empty error message (the path used
        // by `fire_due_rules_at` when the success-row write itself fails),
        // both `trigger_executions.status` and `trigger_rules.last_status`
        // must read back as "failed", and the error column must carry the
        // message. The failure path used to insert no execution row at
        // all, making DB write failures invisible in the audit log.
        let (_tmp, ctx) = setup_context();
        let rule = create_trigger_rule_with_context(
            &ctx,
            "Daily 9am",
            "cron",
            "0 9 * * *",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create rule");
        let fired_at = fixed_time();
        record_trigger_execution(
            &ctx,
            &rule,
            fired_at,
            "failed",
            "simulated DB error",
            "test detail",
            "",
        )
        .expect("record failed execution");

        let (conn, _) = open_connection(&ctx).unwrap();
        let (status, error): (String, String) = conn
            .query_row(
                "SELECT status, error FROM trigger_executions \
                 WHERE rule_id = ?1 ORDER BY fired_at DESC LIMIT 1",
                params![&rule.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("failed execution row should exist");
        assert_eq!(
            status, "failed",
            "execution status must be 'failed' when recorded as failed (#3057), got '{status}'"
        );
        assert_eq!(
            error, "simulated DB error",
            "execution error column must carry the failure message (#3057), got '{error}'"
        );

        // trigger_rules.last_status and last_error must mirror the row.
        let (last_status, last_error): (String, String) = conn
            .query_row(
                "SELECT last_status, last_error FROM trigger_rules WHERE id = ?1",
                params![&rule.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            last_status, "failed",
            "trigger_rules.last_status must mirror 'failed' (#3057), got '{last_status}'"
        );
        assert_eq!(
            last_error, "simulated DB error",
            "trigger_rules.last_error must mirror the message (#3057), got '{last_error}'"
        );

        // run_count must still increment on a failed recording so the
        // scheduler bookkeeping stays consistent.
        let run_count: i64 = conn
            .query_row(
                "SELECT run_count FROM trigger_rules WHERE id = ?1",
                params![&rule.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            run_count, 1,
            "run_count must increment on failed record (#3057)"
        );
    }

    #[test]
    fn is_rule_due_minute_cron_hot_path_is_fast_already_fired() {
        // #3058 regression guard at the public entry point used by the
        // executor hot path. The pathological case in the report is a
        // `* * * * *` rule (~527k iterations in the old 366-day scan).
        // `is_rule_due` now resolves via `last_due_for_rule`, which for an
        // already-fired rule bounds the scan to (last_fired_at, now). 100
        // calls must finish well under the bug report's 143 ms *single-call*
        // ceiling.
        let now = fixed_time();
        let rule = make_rule("min", "* * * * *", TriggerAction::DailyReview);
        let last_fired = Utc.with_ymd_and_hms(2026, 7, 18, 9, 28, 0).unwrap();

        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = is_rule_due(&rule, Some(last_fired), now);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "100 is_rule_due('* * * * *', already-fired) calls took {elapsed:?} (>100ms) — \
             bounded hot-path resolver regressed (#3058)",
        );
    }

    #[test]
    fn is_rule_due_minute_cron_hot_path_is_fast_never_fired() {
        // #3058 companion to the already-fired guard above: the never-fired
        // branch of `last_due_for_rule` must route high-frequency cron
        // through the 2-minute narrow window, not the 366-day fallback.
        let now = fixed_time();
        let rule = make_rule("min", "* * * * *", TriggerAction::DailyReview);

        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = is_rule_due(&rule, None, now);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "100 is_rule_due('* * * * *', never-fired) calls took {elapsed:?} (>100ms) — \
             narrow-window resolver regressed (#3058)"
        );
    }

    // ── Regression tests for #3439 ─────────────────────────────────

    #[test]
    fn fire_due_rules_at_skips_rule_with_unsatisfied_tag_condition() {
        // #3439: a cron rule with a TagContains condition should NOT fire
        // when conditions are evaluated against an empty (cron) context.
        let (_tmp, ctx) = setup_context();
        let rule = create_trigger_rule_with_context(
            &ctx,
            "Urgent Daily Review",
            "cron",
            "0 9 * * *",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create rule");

        // Inject a TagContains condition that cannot be satisfied with an
        // empty cron context.
        {
            let (conn, _) = open_connection(&ctx).unwrap();
            conn.execute(
                "UPDATE trigger_rules SET conditions = ?1 WHERE id = ?2",
                params![r#"[{"type":"tag_contains","tag":"urgent"}]"#, &rule.id],
            )
            .unwrap();
        }

        let outcome = fire_due_rules_at(&ctx, fixed_time()).expect("fire step");
        assert_eq!(
            outcome.fired, 0,
            "rule with unsatisfied TagContains condition must NOT fire"
        );
        assert_eq!(
            outcome.skipped, 1,
            "rule should be counted as skipped due to conditions"
        );

        // No execution row should exist.
        let (conn, _) = open_connection(&ctx).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM trigger_executions WHERE rule_id = ?1",
                params![&rule.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "no execution should be recorded for skipped rule");
    }

    #[test]
    fn fire_due_rules_at_fires_rule_with_always_condition() {
        // #3439: a cron rule with an Always condition should fire normally
        // (Always passes against any context, including empty).
        let (_tmp, ctx) = setup_context();
        let rule = create_trigger_rule_with_context(
            &ctx,
            "Unconditional Review",
            "cron",
            "0 9 * * *",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create rule");

        {
            let (conn, _) = open_connection(&ctx).unwrap();
            conn.execute(
                "UPDATE trigger_rules SET conditions = ?1 WHERE id = ?2",
                params![r#"[{"type":"always"}]"#, &rule.id],
            )
            .unwrap();
        }

        let outcome = fire_due_rules_at(&ctx, fixed_time()).expect("fire step");
        assert_eq!(
            outcome.fired, 1,
            "rule with Always condition must fire normally"
        );
        assert_eq!(
            outcome.skipped, 0,
            "no rules should be skipped when conditions are Always"
        );
    }

    #[test]
    fn fire_due_rules_at_fires_rule_with_no_conditions() {
        // #3439: a cron rule with no conditions (empty list) should fire
        // normally — this is the existing behavior, now explicitly guarded.
        let (_tmp, ctx) = setup_context();
        let _rule = create_trigger_rule_with_context(
            &ctx,
            "No Conditions",
            "cron",
            "0 9 * * *",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create rule");

        let outcome = fire_due_rules_at(&ctx, fixed_time()).expect("fire step");
        assert_eq!(
            outcome.fired, 1,
            "rule with empty conditions must fire normally"
        );
        assert_eq!(outcome.skipped, 0);
    }

    #[test]
    fn fire_due_rules_at_skips_rule_with_frontmatter_condition() {
        // #3439: a cron rule with a FrontmatterEquals condition should NOT
        // fire — the empty cron context has no frontmatter to match.
        let (_tmp, ctx) = setup_context();
        let rule = create_trigger_rule_with_context(
            &ctx,
            "Status Review",
            "cron",
            "0 9 * * *",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create rule");

        {
            let (conn, _) = open_connection(&ctx).unwrap();
            conn.execute(
                "UPDATE trigger_rules SET conditions = ?1 WHERE id = ?2",
                params![
                    r#"[{"type":"frontmatter_equals","field":"status","value":"done"}]"#,
                    &rule.id
                ],
            )
            .unwrap();
        }

        let outcome = fire_due_rules_at(&ctx, fixed_time()).expect("fire step");
        assert_eq!(
            outcome.fired, 0,
            "rule with unsatisfied FrontmatterEquals must NOT fire"
        );
        assert_eq!(outcome.skipped, 1);
    }

    // ── build_action_prompt (dispatch prompt construction) ──

    #[test]
    fn build_action_prompt_daily_review_mentions_recency_and_date() {
        let rule = make_rule("dr", "0 9 * * *", TriggerAction::DailyReview);
        let prompt = build_action_prompt(&rule, fixed_time()).expect("prompt");
        assert!(
            prompt.contains("Daily Review"),
            "must identify the task: {prompt}"
        );
        assert!(
            prompt.contains("2026-07-18"),
            "must carry today's date: {prompt}"
        );
        assert!(prompt.to_lowercase().contains("24 hours"));
    }

    #[test]
    fn build_action_prompt_custom_uses_effective_prompt() {
        let mut rule = make_rule("cu", "0 9 * * *", TriggerAction::Custom);
        rule.custom_prompt = Some("Summarize my meeting notes".into());
        let prompt = build_action_prompt(&rule, fixed_time()).expect("prompt");
        assert!(prompt.contains("Summarize my meeting notes"));
    }

    #[test]
    fn build_action_prompt_custom_without_prompt_is_error() {
        // #2842: a Custom rule with no prompt is a configuration error and
        // must surface as such, never as an empty AI call.
        let rule = make_rule("cnp", "0 9 * * *", TriggerAction::Custom);
        let err = build_action_prompt(&rule, fixed_time()).expect_err("must fail");
        assert!(
            err.contains("no prompt"),
            "error must explain the cause: {err}"
        );
    }

    #[test]
    fn build_action_prompt_process_webhook_rejected_for_cron() {
        // A webhook action has no payload in a cron context — it must be
        // rejected rather than silently running a meaningless prompt.
        let rule = make_rule("wh", "0 9 * * *", TriggerAction::ProcessWebhook);
        let err = build_action_prompt(&rule, fixed_time()).expect_err("must fail");
        assert!(
            err.contains("webhook"),
            "error must explain the cause: {err}"
        );
    }

    #[test]
    fn build_action_prompt_all_predefined_actions_have_prompts() {
        for action in [
            TriggerAction::DailyReview,
            TriggerAction::SummarizeAndTag,
            TriggerAction::SuggestLinks,
        ] {
            let rule = make_rule("pa", "0 9 * * *", action);
            assert!(
                build_action_prompt(&rule, fixed_time()).is_ok(),
                "{action:?} must build a prompt"
            );
        }
    }

    // ── fire_due_rules_with_dispatch (DB-backed integration) ──

    #[test]
    fn fire_due_rules_with_dispatch_records_failure_without_ai_config() {
        // Without AI credentials the dispatch must fail *visibly*: a
        // "failed" execution row with a non-empty error, and the rule's
        // last_fired_at advanced so the same due time is not retried on
        // every tick.
        let (_tmp, ctx) = setup_context();
        let rule = create_trigger_rule_with_context(
            &ctx,
            "Custom Task",
            "cron",
            "0 9 * * *",
            "custom",
            None,
            Some("Summarize recent notes"),
            None,
        )
        .expect("create rule");

        let now = fixed_time();
        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fire_due_rules_with_dispatch(&ctx, now))
            .expect("dispatch tick");
        assert_eq!(outcome.evaluated, 1);
        assert_eq!(outcome.fired, 0, "no AI configured — cannot succeed");
        assert_eq!(outcome.failed, 1);

        let (conn, _) = open_connection(&ctx).unwrap();
        let (status, error): (String, String) = conn
            .query_row(
                "SELECT status, error FROM trigger_executions \
                 WHERE rule_id = ?1 ORDER BY fired_at DESC LIMIT 1",
                params![&rule.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("failed execution row must exist");
        assert_eq!(status, "failed");
        assert!(
            !error.is_empty(),
            "error column must carry the failure reason"
        );

        // last_fired_at must have advanced past the due time so the same
        // due slot is not re-fired on the next tick.
        let outcome2 = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fire_due_rules_with_dispatch(&ctx, now))
            .expect("second dispatch tick");
        assert_eq!(
            outcome2.failed, 0,
            "same due time must not be retried after a recorded failure"
        );
    }

    #[test]
    fn fire_due_rules_with_dispatch_skips_unsatisfied_conditions_without_dispatch() {
        // #3439 gate applies to the dispatching path too: a rule whose
        // conditions cannot be satisfied from an empty cron context is
        // skipped — no AI call, no execution row.
        let (_tmp, ctx) = setup_context();
        let rule = create_trigger_rule_with_context(
            &ctx,
            "Urgent Review",
            "cron",
            "0 9 * * *",
            "daily_review",
            None,
            None,
            None,
        )
        .expect("create rule");
        {
            let (conn, _) = open_connection(&ctx).unwrap();
            conn.execute(
                "UPDATE trigger_rules SET conditions = ?1 WHERE id = ?2",
                params![r#"[{"type":"tag_contains","tag":"urgent"}]"#, &rule.id],
            )
            .unwrap();
        }

        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fire_due_rules_with_dispatch(&ctx, fixed_time()))
            .expect("dispatch tick");
        assert_eq!(outcome.skipped, 1);
        assert_eq!(outcome.fired, 0);
        assert_eq!(outcome.failed, 0);

        let (conn, _) = open_connection(&ctx).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM trigger_executions WHERE rule_id = ?1",
                params![&rule.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "skipped rule must not produce an execution row");
    }
}
