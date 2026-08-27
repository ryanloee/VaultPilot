# VaultPilot — Project Knowledge Base

**Generated:** 2026-08-20
**Commit:** e79e898a
**Branch:** main

## OVERVIEW
Local-first AI knowledge assistant — Rust core `vaultpilot_lib` (SQLite FTS5 + Markdown vault, grounded Q&A, agent tool loop, CJK-aware search) shared by CLI and Tauri v2 desktop/mobile (one React 19 frontend for Win/Linux/Android).

## STRUCTURE
```
VaultPilot/
├── src/                          # vaultpilot_lib — single source of truth
│   ├── ai/                       # provider abstraction, parsing, tool loop
│   ├── storage/                  # SQLite pool, FTS5, notes/collections
│   ├── orchestration/            # triggers, schedulers, event bus
│   ├── regression/               # one file per fixed issue (86 files)
│   ├── models/                   # settings, NoteDocument, SearchQuery
│   ├── agent.rs / agent_engine.rs
│   └── bin/vaultpilot-cli/       # CLI — main.rs 12k lines, 50+ subcmds
├── desktop/                      # Tauri v2 app (vaultpilot-desktop crate)
│   ├── src/                      # React 19 + TS + Tailwind + Zustand (36 files)
│   └── src-tauri/src/            # Rust backend — commands/, state.rs, lib.rs
├── crates/mcp-connector/         # MCP protocol bridge
├── contracts/vaultpilot-agent.v1.json
├── extensions/                   # browser clippers
├── docs/ / scripts/ / tests/     # build.md, TESTING.md, fixtures only
└── PROJECT_STATE.md              # bilingual dev log — check 决策记录/已知阻塞项
```

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Grounded Q&A / answer pipeline | `src/ai/parsing.rs:parse_model_answer`, `src/agent.rs` | strict — empty content = error |
| Full-text / CJK search | `src/storage/search.rs:tokenize_cjk_for_fts`, `src/storage/instant_search.rs` | bigram both ends, schema v2 |
| Trigger scheduling | `src/orchestration/trigger_executor.rs`, `desktop/src-tauri/src/lib.rs:60` | `run_forever()` spawned in Tauri setup |
| CLI commands | `src/bin/vaultpilot-cli/main.rs`, `src/bin/vaultpilot-agent.rs` | handle dispatch, 50+ subcmds |
| Tauri commands bridge | `desktop/src-tauri/src/commands/`, `desktop/src-tauri/src/lib.rs:119` | register in `invoke_handler!` |
| Frontend chat/notes/triggers | `desktop/src/components/`, `desktop/src/lib/tauri.ts` | `invoke()` → Rust, `onAgentStatus` events |
| Agent engines (external) | `src/agent_engine.rs`, `contracts/vaultpilot-agent.v1.json` | builtin + claude-code + codex |
| Regression tests | `src/regression/issue_NNN_*.rs` + `src/regression/mod.rs` | required per bugfix |
| Storage / schema | `src/storage/pool.rs:StorageContext`, `src/storage/mod.rs` | `initialize_storage_with_context` migration |

## CODE MAP
| Symbol | Type | Location | Refs | Role |
|--------|------|----------|------|------|
| `StorageContext` | struct | `src/storage/pool.rs:31` | 464 | SQLite pool, vault paths, settings cache |
| `SearchQuery` | struct | `src/models/mod.rs:501` | 64 | text+tags/filters, limit/offset |
| `TriggerRuleStatus` | struct | `src/storage/trigger_rules.rs:217` | 4 | last/next fire, run_count, last_error |
| `TriggerExecutionRecord` | struct | `src/storage/trigger_rules.rs:297` | 3 | execution log row |
| `AgentEngineRegistry` | struct | `src/agent_engine.rs:1017` | 3 | list/select engines |
| `TriggerExecutor` | struct | `src/orchestration/trigger_executor.rs` | — | cron eval + `fire_due_rules_with_dispatch` |
| `Query` | struct | `src/vault_query.rs:185` | 31 | parsed vault query (select/filter/formulas) |
| `EngineEvent` | struct | `src/agent_engine.rs:173` | — | engine run event stream |

## CONVENTIONS
- Business logic lives in `vaultpilot_lib` only — Tauri layer (`desktop/src-tauri/`) is a thin `#[tauri::command]` wrapper calling `*_with_context` directly (no IPC/subprocess).
- SQLite is blocking — wrap every `*_with_context` in `tokio::task::spawn_blocking` inside async Tauri commands (`desktop/src-tauri/src/commands/triggers.rs` pattern).
- One React frontend for all platforms — gate desktop-only (tray/updater) with `#[cfg(desktop)]` + `is_desktop()` checks; `email` feature default-on but optional (Android disables for OpenSSL).
- Shell on Windows dev machines is CMD — no `ls`/`head`; use `dir`/`rg` without Unix pipes.
- Config files UTF-8 without BOM (`tauri.conf.json` BOM broke parsing — be3fc6d8).

## ANTI-PATTERNS (THIS PROJECT)
- **No fabricated answers** — `parse_model_answer` (`src/ai/parsing.rs`) treats empty model content as error, never canned text. Do not add answer-path fallbacks (only ingest-side tolerances `parse_or_fallback_note`/`fallback_record_reply` are allowed).
- **No business logic in Tauri** — implement in `vaultpilot_lib` so CLI and desktop share it.
- **No bypass of cron validation** — `create/update_trigger_rule_with_context` validate via `next_due_time_at`; stored unparseable cron = silent never-fires trap.
- **No FTS write without `tokenize_cjk_for_fts`** — all `note_fts`/`attachment_fts` writes must bigram-tokenize CJK (`src/storage/search.rs`).
- **No `parseInt(x) || fallback` for cron fields** — `0` is falsy; use explicit nullish check.
- **`TriggerAction::ProcessWebhook` from cron** is invalid (no payload) → recorded as failed; `Custom` without `custom_prompt` is config error #2842.
- **No trailing args after `powershell -Command <script>`** — PowerShell joins them into the script text (never `$args`): a trailing path becomes a bare statement that ShellExecutes the file (opened every OCR'd image in Paint) and `$args[0]` stays null (#4068). Pass data via env var (`OCR_IMAGE_ENV` pattern in `src/storage/notes.rs`).

## COMMANDS
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --exclude vaultpilot-desktop -- -D warnings
cargo test --workspace --exclude vaultpilot-desktop
cargo test regression
cargo check -p vaultpilot-desktop   # run after pnpm build
# desktop/ (pnpm@10)
pnpm install; pnpm build            # tsc --noEmit + vite build
pnpm test                           # vitest run
pnpm tauri dev; pnpm tauri build
```

## NOTES
- Scheduled triggers: `lib.rs` spawns `TriggerExecutor::run_forever()` on Tauri async runtime; close-to-tray keeps process (and scheduler) alive. CLI `trigger fire-now` shares `fire_due_rules_with_dispatch` (prompt → `ask_with_ai_with_context` 180s → save note `source: trigger_rule` → log execution). `fire_due_rules_at` is record-only for tests. Verify via `SELECT * FROM trigger_executions ORDER BY fired_at DESC LIMIT 10`.
- Cron in UTC — `desktop/src/components/triggers/TriggerView.tsx:toCron/fromCron` converts local ↔ UTC (weekday shifts at midnight). Re-save pre-convention rules in UI to fix.
- `list_trigger_rules_with_status_with_context` recomputes `next_fire_at` on read for enabled cron rules (stored column stale after edits).
- Mobile bridge: `desktop/src-tauri/src/lib.rs:34` maps `APPDATA/LOCALAPPDATA/HOME` to Tauri app dirs — do not remove.
- Bugfix pipeline: public API → `src/regression/issue_NNN_desc.rs` + `mod.rs`; private → inline `#[cfg(test)] // REGRESSION: #NNN`; frontend → `desktop/src/*.test.ts(x)` — see `docs/BUGFIX_PIPELINE.md`.
