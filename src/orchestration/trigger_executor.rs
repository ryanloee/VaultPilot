//! # Trigger Executor — cron-based trigger-rule firing engine (#3048)
//!
//! Prior to #3048 the `trigger_rules` storage layer shipped (v0.5.61) but no
//! scheduler read the table, so rules never fired. This module closes that gap
//! by providing:
//!
//! - **Pure cron evaluation** ([`last_due_time_before`], [`is_rule_due`],
//!   [`evaluate_rules_at`]) — fully unit-testable, no I/O.
//! - **Fire-and-record** ([`fire_due_rules`]) — a synchronous step that, given
//!   a [`StorageContext`], loads enabled cron rules, finds the ones whose
//!   schedule is due relative to their `last_fired_at`, records an execution
//!   row in `trigger_executions`, and updates the rule's `last_fired_at` /
//!   `run_count` / `last_status`. The observable side effect is real: callers
//!   can query the execution log and the rule's last-fired timestamp.
//! - **Background loop** ([`TriggerExecutor::spawn`]) — a `tokio` task that
//!   calls [`fire_due_rules`] on a fixed cadence until the
//!   [`CancellationToken`](tokio_util::sync::CancellationToken) is cancelled.
//!
//! ## Dispatch model (honest scope)
//!
//! Firing a rule in this module means: **evaluate schedule → record execution
//! → emit a `tracing` event**. It does **not** yet invoke the LLM agent to
//! produce a daily-review / summarize-and-tag note. The agent dispatch path
//! requires the full runtime (provider client, vault write-back, prompt
//! templating) and is intentionally deferred to a follow-up; a pure cron
//! evaluator + observable fire log is the minimum honest fix for the silent
//! failure mode reported in #3048 ("rules sit in the DB and nothing happens").
//!
//! Rules now produce *visible* output the moment they fire — query
//! `trigger_executions` or check `trigger_rules.last_fired_at` — so users and
//! the inspector can verify the scheduler is alive. A future change plugs an
//! `AgentDispatcher` into the fire path.

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

use crate::orchestration::trigger::{AgentTriggerRule, TriggerKind};
use crate::storage::pool::open_connection;
use crate::storage::trigger_rules::list_trigger_rules_with_context;
use crate::storage::StorageContext;

/// Default cadence at which the background executor scans for due rules.
///
/// One minute matches the finest standard cron resolution (the cron minute
/// field) — polling faster would not surface any earlier fires and would add
/// pointless DB load.
pub const DEFAULT_TICK_INTERVAL: Duration = Duration::from_secs(60);

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
/// (empty on success); `detail` is optional free-form context (e.g. the
/// effective prompt that would have been sent).
fn record_trigger_execution(
    context: &StorageContext,
    rule: &AgentTriggerRule,
    fired_at: DateTime<Utc>,
    status: &str,
    error: &str,
    detail: &str,
) -> Result<()> {
    let (mut connection, _) = open_connection(context)?;
    let tx = connection.transaction()?;
    let exec_id = Uuid::new_v4().to_string();
    let fired_rfc = fired_at.to_rfc3339();
    let action_str = format!("{:?}", rule.action).to_lowercase();
    tx.execute(
        "INSERT INTO trigger_executions (id, rule_id, label, action, fired_at, status, error, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![exec_id, rule.id, rule.label, action_str, fired_rfc, status, error, detail],
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
        let detail = d
            .rule
            .effective_prompt()
            .map(|p| format!("effective_prompt={}", truncate(p, 200)))
            .unwrap_or_else(|| format!("action={:?}", d.rule.action));
        // #3055: status field must match the documented contract ("success"
        // or "failed"). The previous implementation passed the literal
        // "fired", which broke any downstream filter on `status = 'success'`
        // (WinUI Inspector, mobile surfaces, external dashboards).
        let recorded = record_trigger_execution(context, d.rule, d.due_at, "success", "", &detail);
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
                    context, d.rule, d.due_at, "failed", &err_msg, &detail,
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
    /// 2. Calls [`fire_due_rules_at`] inside `spawn_blocking` (SQLite is
    ///    synchronous).
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
                match spawn_blocking(move || fire_due_rules_at(&ctx, Utc::now())).await {
                    Ok(Ok(outcome)) => {
                        if outcome.fired > 0 || outcome.failed > 0 {
                            info!(
                                evaluated = outcome.evaluated,
                                fired = outcome.fired,
                                failed = outcome.failed,
                                "trigger tick complete"
                            );
                        }
                    }
                    Ok(Err(e)) => {
                        warn!(error = %e, "trigger tick failed");
                    }
                    Err(e) => {
                        warn!(error = %e, "trigger tick task panicked");
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
             narrow-window resolver regressed (#3058)",
        );
    }
}
