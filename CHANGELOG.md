# Changelog

All notable changes to VaultPilot will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.3.53] - 2026-06-24

### Fixed
- Mobile attachments, send button, suggestions list, progress indicator, voice input, scroll behavior (#1453-#1458)
- Mobile APK update: use `Directory` for `downloadFileAsync` + `INSTALL_PACKAGE` intent on Android 13+
- Mobile `globalSearch` FTS fallback now searches session titles, not just message content (#1478, #1480)
- Mobile `SettingsScreen` testConnection passes model parameter to `checkApi` (#1479, #1481)
- Cargo fmt indentation in `chat.rs` resolve_* tests

### Tests
- `search.rs` 17 boundary tests — search core functions (#1482)
- `mcp_server.rs` 13 boundary tests — MCP tool processing (#1483)
- `http_bridge.rs` 8 boundary tests — pure functions (#1486)
- `prompting.rs` 14 boundary tests — render, escape, system prompt completeness (#1490)
- `orchestration/chat.rs` 22 pure function + 19 session management boundary tests (#1488, #1489)

## [0.3.52] - 2026-06-24

### Fixed
- `globalSearch` FTS→LIKE fallback when FTS returns empty (common with CJK text) (#1471)
- `globalSearch` LIKE fallback now also searches session titles, not just message content (#1476)
- Replace remaining `any` types with proper TypeScript types in mobile (#1474)

### Tests
- 3 regression tests for `globalSearch` FTS→LIKE fallback (#1471)
- 1 regression test for session title LIKE search (#1476)
- 13 unit tests for `isNoteRelatedQuery` CJK/English detection (#1473)

## [0.3.51] - 2026-06-24

### Fixed
- APK in-app update intent not launching on Android 13+ (#54b140f)
- Mobile keyboard-aware input + voice continuous recording (#5648870)
- Mobile RAG force-inject notes when user asks about notes (#b158a0e)
- `ProviderEditor.tsx` replace `require()` with proper ES import (#1465)
- Sync error body logging + dead code cleanup + uuid export (#1461, #1462, #1463)

### Changed
- Extract `inferMime` from `ChatScreen.tsx` to `chatHelpers.ts` (#1467)
- Export `stripMarkdown` from `autoTag.ts` for direct testing (#1466)
- Mobile `@types/react` bump to 19.2.17

### Tests
- 15 tests for `inferMime` pure function (#1467)
- 14 tests for `stripMarkdown` pure function (#1466)
- 6 tests for `uuid()` format/uniqueness/fallback (#1463)

## [0.3.50] - 2026-06-24

### Added
- WriteApprovalDialog human-readable diff preview (#1453)

### Tests
- 12 integration tests for `run_agent()` exception paths — malformed JSON, network drop, timeout, approval rejection (#1454)
- 6 edge case tests for mobile `sync.ts` + `settingsSync.ts` service layer (#1455)

## [0.3.49] - 2026-06-24

### Fixed
- `pending_syncs` UNIQUE index on `note_id` for INSERT OR REPLACE dedup (#1447)
- `useNetworkState` calls `checkConnection` on mount (#1448)

### Changed
- Split `models.rs` (1752 lines) into `provider.rs` + `settings.rs` + `mod.rs` (#1442)
- Extract `storage/pool.rs` for connection pool management (#1443)

### Tests
- 26 unit tests for `clientPure.ts` — 5 Anthropic adapter functions (#1441)
- 5 edge case tests for `flushPendingSyncs` (#1449)

## [0.3.48] - 2026-06-24

### Added
- Agent Mode first-use onboarding guide — WinUI tooltip + CLI help (#1432)

### Fixed
- CLI agent prints FinalAnswer text instead of discarding it (#1436)
- OfflineBanner dark theme support (#1437)

### Changed
- Extract `store.ts` pure functions + 31 unit tests (#1429)
- Extract `NoteEditorScreen` pure functions + 29 unit tests (#1430)
- Create CHANGELOG.md with v0.3.x release history (#1431)

## [0.3.47] - 2026-06-23

### Changed
- Export pure functions from `rag.ts` and `db.ts` for direct unit testing (#1425, #1426)
- `updateChecker.ts` console.log → console.warn for consistency (#1421)

### Tests
- 19 unit tests for `rag.ts` parseToolCalls + buildSystemPrompt (#1422)
- 14 unit tests for `extractKeywords` (#1425)
- 17 unit tests for `buildFtsQuery` + `escapeLikePattern` (#1426)

## [0.3.46] - 2026-06-23

### Fixed
- SSE reconnection no longer sends duplicate content after content delivery (#1417)
- `settingsSync.ts` JSON.parse wrapped with try/catch for corrupt data (#1415)
- `ChatScreen` JSON.parse(m.attachments) protected with safeParseAttachments (#1414)

### Tests
- 21 unit tests for `messageV2.ts` + `clientUtils.ts` pure functions (#1410)
- 30 unit tests for `latex.ts` pure functions (#1409)

## [0.3.45] - 2026-06-23

### Fixed
- `globalSearch()` buildFtsQuery called once — prevent discarding note results (#1403)
- Add console.warn to SettingsScreen + UpdateModal silent catch blocks (#1402)

### Tests
- 7 unit tests for `fmtTime()` pure function (#1404)

## [0.3.44] - 2026-06-23

### Added
- InputBar '+' expand button + emoji picker
- In-app APK download update mechanism

### Fixed
- APK install uses content:// URI instead of jumping to browser
- Pin react to 19.2.3 + React version mismatch prevention

## [0.3.43] - 2026-06-23

### Fixed
- Sync timestamp unit mismatch — seconds vs milliseconds caused re-downloads (#1390)
- Extract text from ContentPart[] in system messages — fixed [object Object] (#1396)
- Paginate syncNotesFromServer for large vaults (#1398)

### Changed
- Extract `buildFtsQuery` helper — deduplicate 4 inline FTS5 query constructions (#1392)

### Tests
- 10 edge case tests for `checkForUpdate` (#1394)
- 26 unit tests for `looksLikeSmallTalk` + `extractCJKNgrams` + `isCJK` (#1397)

## [0.3.42] - 2026-06-23

### Added
- Agent Mode WinUI UI — tool call panel + write approval dialog (#1348)
- Agent Mode CLI usage guide in README (#1350)

### Fixed
- Replace `.expect()` with graceful error handling in `agent.rs` (#1354)
- Replace `.expect()` in `stable_term_hash` with `copy_from_slice` (#1355)

### Tests
- 28 unit tests for `agent.rs` pure functions (#1349)

## [0.3.41] - 2026-06-23

### Changed
- Split `ChatScreen.tsx` into 5 sub-components (#1336)
- Split `SettingsScreen.tsx` into 5 sub-components (#1338)
- Extract `client.ts` pure functions + 27 unit tests (#1337)

## [0.3.40] - 2026-06-23

### Fixed
- Expand isCJK regex to cover Japanese/Korean ranges in RAG (#1330)
- Remove AbortSignal listener leak on chatAnthropic success path (#1332)
- Add Japanese/Korean stop words to RAG extractKeywords (#1334)
- Add Japanese Hiragana/Katakana + Korean Hangul ranges to is_cjk (#1328)

## [0.3.39] - 2026-06-23

### Added
- MCP HTTP server with POST /mcp endpoint + bearer token auth (#1306)

### Tests
- 54 unit tests for `ai/client.rs` pure functions (#1305)
- 34 unit tests for `http_bridge.rs` pure functions (#1310)

## [0.3.37] - 2026-06-22

### Added
- Agent Mode Phase 1 — AgentProtocol + ToolProxy + vault sandboxing (#1282)
- MCP HTTP server with POST /mcp endpoint + bearer token auth (#1306)

### Changed
- Extract `notes.rs` module from `storage/mod.rs` (#1280)
- Extract `search.rs` module from `storage/mod.rs` (#1281)

### Tests
- 54 unit tests for `ai/client.rs` pure functions (#1305)
- 34 unit tests for `http_bridge.rs` pure functions (#1310)
- 20 unit tests for `mcp_server.rs` pure functions (#1311)
- 24 unit tests for `markdown_utils.rs` pure functions (#1314)

## [0.3.36] - 2026-06-22

### Added
- Agent Mode Phase 2 — write pattern whitelist + process management (#1288)

### Changed
- Split `lib.rs` into orchestration modules (#1287)
- Split `vaultpilot-cli.rs` into 4 modules (#1286)
- Extract `ai/context.rs` module (#1289)

## [0.3.35] - 2026-06-22

### Fixed
- CI: add retry logic for cargo-audit install to handle transient network errors (#1273)

## [0.3.34] - 2026-06-22

### Added
- Proactive knowledge push — `find_related_notes` backend + CLI (#914)
- MCP `notes.related` tool + HTTP endpoint (#914)
- Mobile auto-tagging on note save (#1221)
- Mobile settings export/import (#1222)

### Fixed
- Mobile offline edit queue + sync indicator (#1220)

## [0.3.33] - 2026-06-22

### Changed
- Extract chat session module from storage — Phase 2 (#1197)

### Tests
- Unit tests for `mask_secret`, `ProviderType::from_base_url`, `masked()` (#1252)
- 9 test cases for `updateChecker` compareSemver (#1236)
- GlobalSearch edge case tests (#1251)
- NormalizeApiBase edge case tests (#1250)
- ExecuteSave + parseToolCalls edge case tests (#1249)

## [0.3.32] - 2026-06-21

### Added
- Mobile first-use onboarding screen (#1225)
- Mobile APK auto-update detection (#1226)

### Changed
- Extract shared `fmtTime` utility (#1229)
- Extract API utility functions from `client.ts` to `clientUtils.ts` (#1247)

### Tests
- Extend `db.ts` unit tests — 10 new test cases (#1231)

## [0.3.31] - 2026-06-21

### Added
- MessageV2 unified cross-platform schema (#1239)
- MessageV2 TypeScript types + 7 unit tests (#1239)
- MessageV2 C# types + 7 xUnit tests (#1239)

### Fixed
- RAG search fails to find notes — FTS5 LIKE fallback + relaxed CJK keywords (#1241)
- Extract renderLatex + renderInline to testable utilities, fix regex bugs (#1246)
- Escape `<title>` HTML tag in vault_export_with_context doc comment (#1237)

### Tests
- 19 unit tests for ChatScreen pure logic (#1214)

## [0.3.30] - 2026-06-21

### Fixed
- Pin react to 19.2.3 to match react-native-renderer (#1224)

### Tests
- Extend existing test coverage across mobile modules

## [0.3.29] - 2026-06-21

### Changed
- Major test coverage push — 200+ tests across Rust and mobile

## [0.3.28] - 2026-06-21

### Added
- Initial test infrastructure for mobile (Jest + React Native Testing Library)

### Changed
- Begin systematic test coverage improvement

---

## Earlier Versions

Versions v0.1.x through v0.3.27 are available in the git history.
Key milestones:
- **v0.2.0**: Initial multi-platform architecture (WinUI + CLI + Android)
- **v0.3.0**: Agent Mode Phase 1, MCP server, storage refactoring

[Unreleased]: https://github.com/ryanloee/VaultPilot/compare/v0.3.53...HEAD
[0.3.53]: https://github.com/ryanloee/VaultPilot/compare/v0.3.52...v0.3.53
[0.3.52]: https://github.com/ryanloee/VaultPilot/compare/v0.3.51...v0.3.52
[0.3.51]: https://github.com/ryanloee/VaultPilot/compare/v0.3.50...v0.3.51
[0.3.50]: https://github.com/ryanloee/VaultPilot/compare/v0.3.49...v0.3.50
[0.3.49]: https://github.com/ryanloee/VaultPilot/compare/v0.3.48...v0.3.49
[0.3.48]: https://github.com/ryanloee/VaultPilot/compare/v0.3.47...v0.3.48
[0.3.47]: https://github.com/ryanloee/VaultPilot/compare/v0.3.46...v0.3.47
[0.3.46]: https://github.com/ryanloee/VaultPilot/compare/v0.3.45...v0.3.46
[0.3.45]: https://github.com/ryanloee/VaultPilot/compare/v0.3.44...v0.3.45
[0.3.44]: https://github.com/ryanloee/VaultPilot/compare/v0.3.43...v0.3.44
[0.3.43]: https://github.com/ryanloee/VaultPilot/compare/v0.3.42...v0.3.43
[0.3.42]: https://github.com/ryanloee/VaultPilot/compare/v0.3.41...v0.3.42
[0.3.41]: https://github.com/ryanloee/VaultPilot/compare/v0.3.40...v0.3.41
[0.3.40]: https://github.com/ryanloee/VaultPilot/compare/v0.3.39...v0.3.40
[0.3.39]: https://github.com/ryanloee/VaultPilot/compare/v0.3.37...v0.3.39
[0.3.37]: https://github.com/ryanloee/VaultPilot/compare/v0.3.36...v0.3.37
[0.3.36]: https://github.com/ryanloee/VaultPilot/compare/v0.3.35...v0.3.36
[0.3.35]: https://github.com/ryanloee/VaultPilot/compare/v0.3.34...v0.3.35
[0.3.34]: https://github.com/ryanloee/VaultPilot/compare/v0.3.33...v0.3.34
[0.3.33]: https://github.com/ryanloee/VaultPilot/compare/v0.3.32...v0.3.33
[0.3.32]: https://github.com/ryanloee/VaultPilot/compare/v0.3.31...v0.3.32
[0.3.31]: https://github.com/ryanloee/VaultPilot/compare/v0.3.30...v0.3.31
[0.3.30]: https://github.com/ryanloee/VaultPilot/compare/v0.3.29...v0.3.30
[0.3.29]: https://github.com/ryanloee/VaultPilot/compare/v0.3.28...v0.3.29
[0.3.28]: https://github.com/ryanloee/VaultPilot/releases/tag/v0.3.28
