# desktop — Tauri v2 App (Frontend + Backend)

## OVERVIEW
Tauri v2 desktop/mobile shell — React 19 + TS + Tailwind + Zustand frontend (one codebase for Win/Linux/Android) + Rust backend (`vaultpilot-desktop` crate) that wraps `vaultpilot_lib` in-process.

## STRUCTURE
```
desktop/
├── src/                 # React frontend (36 TS/TSX files)
│   ├── components/      # chat/, notes/, triggers/, layout/, ui/
│   ├── lib/             # tauri.ts (invoke wrappers), store.ts, mock.ts
│   └── types/           # AppSettings, TriggerRule, NoteDocument etc.
├── src-tauri/src/
│   ├── lib.rs           # setup: AppState + TriggerExecutor + tray
│   ├── state.rs         # AppState (StorageContext holder)
│   └── commands/        # system/ settings/ notes/ chat/ collections/ triggers/
├── src-tauri/icons/     # platform icons (ios/android)
└── package.json         # pnpm@10, vite, vitest
```

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Tauri setup / scheduler | `src-tauri/src/lib.rs:60` | `TriggerExecutor::run_forever()` + tray |
| Command bridge | `src-tauri/src/commands/`, `src-tauri/src/lib.rs:119` `invoke_handler!` | new cmd must register |
| Frontend API | `src/lib/tauri.ts` | `tauriApi` + `api` fallback + `onAgentStatus` |
| Chat / triggers UI | `src/components/chat/`, `src/components/triggers/TriggerView.tsx` | `toCron/fromCron` UTC conversion |
| Mobile bridge | `src-tauri/src/lib.rs:34` | APPDATA/HOME → Tauri app dirs |

## CONVENTIONS
- Tauri layer is thin — `#[tauri::command]` calls `vaultpilot_lib` directly, no business logic.
- Wrap every `*_with_context` in `tokio::task::spawn_blocking` (see `commands/triggers.rs`).
- Desktop-only gated by `#[cfg(desktop)]` (Rust) + `is_desktop()` (TS); `email` feature optional for Android.
- One React frontend for all platforms — no platform forks.

## ANTI-PATTERNS
- No business logic in `desktop/src-tauri/` — put it in `vaultpilot_lib`.
- No new `invoke` without adding to `lib.rs:invoke_handler!`.
- No `tauri.conf.json` with BOM — breaks parsing (be3fc6d8).
- No `parseInt(x) || fallback` for cron fields in TS — `0` falsy.

## COMMANDS
```bash
pnpm install; pnpm build   # tsc --noEmit + vite build
pnpm test                  # vitest run
pnpm tauri dev; pnpm tauri build
cargo check -p vaultpilot-desktop  # after pnpm build
```

## NOTES
- Close-to-tray: window hide on close, quit via tray「退出」; keeps scheduler alive.
- `onAgentStatus` (`tauri.ts:207`) — listen to `agent-status` events during `ask_with_ai`.
- Mock fallback (`mock.ts`) for browser-only testing without Tauri bridge.
