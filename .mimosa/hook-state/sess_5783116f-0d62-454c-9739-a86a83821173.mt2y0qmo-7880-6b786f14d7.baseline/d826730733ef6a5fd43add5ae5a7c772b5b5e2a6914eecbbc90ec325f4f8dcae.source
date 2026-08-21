# src/regression — Regression Tests (one file per fixed issue)

## OVERVIEW
86 files, one per bugfix — `issue_NNN_desc.rs` + registration in `mod.rs`. Required by `docs/BUGFIX_PIPELINE.md`.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Registry | `mod.rs` | every `issue_*` must be `mod` + `#[cfg(test)]` entry |
| Naming | `issue_NNN_desc.rs` | NNN = GitHub issue number |
| Fixtures | `tests/` (repo root) | fixtures only — integration tests live inline |

## CONVENTIONS
- Public API bug → new file `src/regression/issue_NNN_desc.rs` + register in `mod.rs`.
- Private function bug → inline `#[cfg(test)] // REGRESSION: #NNN` in same file.
- Frontend bug → `desktop/src/*.test.ts(x)` (vitest).
- See `docs/BUGFIX_PIPELINE.md` for template.

## ANTI-PATTERNS
- No bugfix without a regression test — CI-adjacent policy.
- No generic `test.rs` — one file per issue number.
