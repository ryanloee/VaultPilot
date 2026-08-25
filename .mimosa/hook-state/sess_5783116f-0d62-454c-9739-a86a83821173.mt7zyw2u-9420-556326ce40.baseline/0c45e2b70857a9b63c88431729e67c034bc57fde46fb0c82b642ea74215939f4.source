# src/orchestration — Triggers & Scheduling

## OVERVIEW
Trigger rules, cron evaluation (UTC), executor dispatch, event bus. Desktop spawns `TriggerExecutor::run_forever()` at setup; CLI uses same dispatch path.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Executor loop | `trigger_executor.rs` (1.7k lines) | `DEFAULT_TICK_INTERVAL=60s`, `fire_due_rules_with_dispatch` |
| Cron eval | `trigger_executor.rs:next_due_time_at` | `cron` crate 6-field, `Schedule::from_str` UTC |
| Trigger types | `trigger.rs` | Cron / Event / webhook |
| Ask dispatch | `ask.rs` (1.7k lines) | called with 180s timeout per trigger fire |
| Event bus | `event_bus.rs` | internal pub/sub |

## CONVENTIONS
- Cron stored & evaluated in UTC — UI converts local↔UTC (`TriggerView.tsx:toCron/fromCron`).
- `create/update_trigger_rule_with_context` validates cron via `next_due_time_at` — reject invalid.
- Firing: prompt → `ask_with_ai_with_context` → save note (`source: trigger_rule`) → log `trigger_executions`.

## ANTI-PATTERNS
- No unvalidated cron storage — unparseable = silent never-fires.
- No `ProcessWebhook` from cron — no payload; recorded as failed.
- No `Custom` without `custom_prompt` — config error #2842, not silent no-op.
- No `parseInt(x) || fallback` for cron fields — `0` is falsy.
- No use of `fire_due_rules_at` for dispatch — record-only, for tests.

## NOTES
- `list_trigger_rules_with_status_with_context` recomputes `next_fire_at` on read (stored column stale).
- Verify liveness: `SELECT * FROM trigger_executions ORDER BY fired_at DESC LIMIT 10`.
- Close-to-tray keeps scheduler alive; quit via tray「退出」.
