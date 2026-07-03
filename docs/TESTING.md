# VaultPilot Testing Guide

## Test Organization

### Rust (CLI / Core Library)

All Rust tests use the built-in `#[cfg(test)]` + `#[test]` framework. No external test runner needed.

| Test Type | Location | Naming Convention | Run Command |
|-----------|----------|-------------------|-------------|
| Unit tests | Inline `#[cfg(test)] mod tests {}` in each source file | `fn test_<function_name>_<scenario>()` | `cargo test -p vaultpilot` |
| Regression tests | `src/regression/` directory, one file per issue | `fn regression_<issue_number>_<short_desc>()` | `cargo test regression` |
| Integration tests | `tests/` directory (top-level) | `fn it_<feature>_<scenario>()` | `cargo test --test '*'` |

### Regression Test File Naming

```
src/regression/
├── mod.rs                    # Re-exports all regression test modules
├── issue_042_empty_vault.rs  # Regression for issue #42
├── issue_099_search_crash.rs # Regression for issue #99
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

### Mobile (TypeScript / Jest)

| Test Type | Location | Naming Convention |
|-----------|----------|-------------------|
| Unit tests | `mobile/src/__tests__/` | `<module>.test.ts` |
| Regression tests | `mobile/src/__tests__/regression/` | `issue_xxx.test.ts` |

### WinUI (C# / xUnit)

| Test Type | Location | Naming Convention |
|-----------|----------|-------------------|
| Unit tests | `native/VaultPilot.WinUI.Tests/` | `<Class>Tests.cs` |
| Regression tests | `native/VaultPilot.WinUI.Tests/Regression/` | `IssueXxxTests.cs` |

---

## Writing a Regression Test (Step-by-Step)

When fixing a bug, **always** add a regression test. This is a hard requirement for PRs tagged `bug`.

### 1. Identify the test location

- **Rust public API bug** → `src/regression/issue_NNN_short_desc.rs`
- **Rust internal/private function bug** → Inline `#[cfg(test)]` in the source file, with a `// REGRESSION: #NNN` comment
- **Mobile bug** → `mobile/src/__tests__/regression/issue_NNN.test.ts`
- **WinUI bug** → `native/VaultPilot.WinUI.Tests/Regression/IssueNNNTests.cs`

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

The CI pipeline runs `cargo test --workspace` which includes all regression tests.
Regression tests are **never** skipped in CI.

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

## Regression Test Template (TypeScript / Jest)

Copy into `mobile/src/__tests__/regression/issue_NNN.test.ts`:

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
cargo test --workspace   # runs ALL tests including regression
```

### Regression-Specific CI Step

The CI includes a dedicated step to report regression test results separately:

```yaml
- name: Run regression tests (report)
  run: cargo test regression -- --format=terse 2>&1 | tee regression-results.txt
```

### Mobile CI (when enabled)

```yaml
- name: Run mobile regression tests
  working-directory: mobile
  run: npx jest --testPathPattern=regression
```

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
