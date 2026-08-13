# VaultPilot Testing Guide

## Test Organization

### Rust (CLI / Core Library)

All Rust tests use the built-in `#[cfg(test)]` + `#[test]` framework. No external test runner needed.

| Test Type | Location | Naming Convention | Run Command |
|-----------|----------|-------------------|-------------|
| Unit tests | Inline `#[cfg(test)] mod tests {}` in each source file | `fn test_<function_name>_<scenario>()` | `cargo test -p vaultpilot` |
| Regression tests | `src/regression/` directory, one file per issue | `fn regression_<issue_number>_<short_desc>()` | `cargo test regression` |
| Integration tests | `tests/` (⚠ currently empty — only `fixtures/` exists; coming soon) | `fn it_<feature>_<scenario>()` | `cargo test --tests` (⚠ currently empty — exits 0) |

### Regression Test File Naming

```
src/regression/
├── mod.rs                    # Re-exports all regression test modules
├── issue_1175_missing_test_fields.rs  # Regression for issue #1175
├── issue_1326_agent_glob_utf8.rs      # Regression for issue #1326
└── ...
```

Each regression file follows this template:

```rust
// Regression test for issue #XXX: <brief description>
//
// Bug: <what was broken>
// Root cause: <what caused it>
// Fix: <commit hash or PR link>

#[test]
fn regression_xxx_descriptive_name() {
    // Arrange: set up the exact conditions that triggered the bug
    // ...
    
    // Act: call the function that was failing
    // ...
    
    // Assert: verify the fix holds
    // ...
}
```

### Desktop UI (TypeScript)

| Test Type | Location | Naming Convention |
|-----------|----------|-------------------|
| Frontend unit tests | `desktop/src/` tests alongside modules | `*.test.ts(x)` |
| Frontend build gate | `desktop/` | `pnpm build` (tsc --noEmit + vite build) |

---

## Writing a Regression Test (Step-by-Step)

When fixing a bug, **always** add a regression test. This is a hard requirement for PRs tagged `bug`.

### 1. Identify the test location

- **Rust public API bug** → `src/regression/issue_NNN_short_desc.rs`
- **Rust internal/private function bug** → Inline `#[cfg(test)]` in the source file, with a `// REGRESSION: #NNN` comment
- **Desktop UI bug** → frontend test under `desktop/src/`, or Rust regression under `src/regression/` when the bug lives in the shared backend

### 2. Create the regression file

Use the template below. The test must:
- Reproduce the exact failure condition
- Pass with the fix applied
- Fail without the fix (verify by temporarily reverting)

### 3. Register the test (Rust only)

Add the module to `src/regression/mod.rs`:
```rust
mod issue_NNN_short_desc;
```

### 4. Run locally

```bash
# All regression tests
cargo test regression

# Specific regression test
cargo test regression_NNN

# Full suite
cargo test --workspace
```

### 5. CI runs automatically

The CI pipeline runs `cargo test --workspace --exclude vaultpilot-desktop`, which
includes all regression tests in the core workspace. The `vaultpilot-desktop` crate
is excluded from CI — its tests are NOT run there (desktop frontend tests run via
`pnpm test` in a separate job; see CI Integration below). Regression tests are
**never** skipped in CI.

---

## Regression Test Template (Rust)

Copy this into `src/regression/issue_NNN_short_desc.rs`:

```rust
//! Regression test for issue #NNN: <title>
//!
//! **Bug:** <1-line description of what was broken>
//! **Root cause:** <what went wrong>
//! **Fix:** PR #NNN / commit abc1234

#[cfg(test)]
mod tests {
    // Import the functions/types under test.
    // Adjust the path to match the module being tested.
    // e.g., use crate::storage::{...};

    #[test]
    fn regression_NNN_descriptive_name() {
        // Arrange
        // ... set up the exact conditions that triggered the bug

        // Act
        // ... call the function/operation that was failing

        // Assert
        // ... verify correct behavior
        // assert_eq!(result, expected);
    }

    #[test]
    fn regression_NNN_edge_case() {
        // Optional: test related edge cases discovered during investigation
    }
}
```

---

## Regression Test Template (TypeScript / Vitest)

Copy into a test file next to the module under test under `desktop/src/` (or into `src/regression/` for shared-backend bugs)
or `issue_NNN.test.tsx` (component/JSX tests). Vitest picks up both extensions
automatically — run from `desktop/` with `pnpm vitest run issue_NNN`.

```typescript
/**
 * Regression test for issue #NNN: <title>
 *
 * Bug: <description>
 * Root cause: <cause>
 * Fix: PR #NNN / commit abc1234
 */

describe('Regression: Issue #NNN', () => {
  test('should <expected behavior>', () => {
    // Arrange
    // ...

    // Act
    // ...

    // Assert
    // expect(result).toBe(expected);
  });
});
```

---

## CI Integration

### Current Pipeline

```yaml
# .github/workflows/ci.yml
cargo test --workspace --exclude vaultpilot-desktop   # core workspace tests (incl. regression); desktop crate is NOT run in CI
pnpm test                                             # desktop frontend tests (desktop/), incl. regression (= `vitest run`)
```

### Mobile CI (not yet configured)

> **Note:** `mobile/` has no test runner configured yet (no jest/vitest setup in
> `mobile/package.json`). Do not wire Jest into CI — add a runner first, then
> document the command here.

---

## Checklist for Bug-Fix PRs

When submitting a bug fix, your PR **must** include:

- [ ] Regression test that reproduces the original bug
- [ ] Test passes with the fix applied
- [ ] Test fails without the fix (mention this in PR description)
- [ ] Test file follows naming convention: `issue_NNN_short_desc`
- [ ] Test is registered in `mod.rs` (Rust) or discoverable by test runner

PRs tagged `bug` without a regression test will be flagged by the automated
code review cron job.
