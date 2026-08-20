# src/storage — Storage & Search

## OVERVIEW
SQLite (r2d2 pool + WAL) + FTS5 + Markdown vault. `StorageContext` (pool.rs:31, 464 refs) is the single entry point; all `*_with_context` are blocking.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| FTS / CJK bigram | `search.rs:tokenize_cjk_for_fts`, `instant_search.rs` | bigram both write & query side |
| Schema & migration | `mod.rs:initialize_storage_with_context`, `pool.rs` | v1→v2 re-index from disk |
| Notes CRUD | `notes.rs` (4k lines) | largest file after CLI main.rs |
| Trigger rules/status | `trigger_rules.rs:217 TriggerRuleStatus`, `297 TriggerExecutionRecord` | `next_fire_at` recomputed on read |
| Collections/graph | `collections.rs`, `knowledge_graph.rs` | wikilink / backlink |
| Settings cache | `pool.rs:StorageContext.cached_settings` | OS keychain for api_key (#2826) |

## CONVENTIONS
- Every public function is `*_with_context(&StorageContext, ...)` — never open a bare `Connection`.
- FTS tables use `tokenize='unicode61'` + manual `tokenize_cjk_for_fts` — any new write path must apply it.
- `busy_timeout=5000`, `foreign_keys=ON`, `journal_mode=WAL` set at pool init.
- `AppPaths` derived from `APPDATA/LOCALAPPDATA/HOME` (bridged on mobile in `desktop/src-tauri/src/lib.rs:34`).

## ANTI-PATTERNS
- No raw `rusqlite::Connection::open` outside `pool.rs` — use `open_connection(&ctx)`.
- No FTS write without `tokenize_cjk_for_fts` — CJK recall drops to zero.
- No bypass of `initialize_storage_with_context` migration check — v1 DB retries next open on failure.
- No async SQLite — wrap in `spawn_blocking` at call site (Tauri layer), not here.

## NOTES
- `instant_search_notes_with_context` scans vault files directly (no FTS) for instant search fallback.
- Hidden dirs (`.trash`) excluded from instant search.
- `list_trigger_rules_with_status_with_context` recomputes `next_fire_at` for enabled cron rules on every read.
