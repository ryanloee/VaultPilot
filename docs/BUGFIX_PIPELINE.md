# Bug-Fix Pipeline Prompt Template
#
# Use this prompt when fixing a bug in VaultPilot.
# Include this as part of the AI coding agent's instruction set.
#
# This template should be referenced by CI automation and coding agents
# when processing bug-fix issues.

---

## Bug-Fix Workflow Prompt

When fixing a bug in VaultPilot, follow this exact workflow:

### Step 1: Understand the Bug
- Read the issue description and reproduction steps
- Identify the affected module (CLI/Rust, Mobile/TS, WinUI/C#)
- Find the root cause in the source code

### Step 2: Write a Failing Regression Test FIRST (TDD)
Before fixing the bug, write a test that reproduces the failure:

**For Rust (CLI/Core):**
```bash
# Copy the template (for public API bugs)
cp src/regression/_TEMPLATE.rs src/regression/issue_NNN_short_desc.rs

# Fill in the test that reproduces the bug
# Register in src/regression/mod.rs:
#   mod issue_NNN_short_desc;

# Run and confirm it FAILS
cargo test regression_NNN

# NOTE: For private function bugs, add inline tests in the source file:
#   #[cfg(test)]
#   mod tests {
#       // REGRESSION: #NNN — <description>
#       #[test]
#       fn regression_NNN_desc() { ... }
#   }
```

**For Desktop UI (Tauri / React):**
```bash
# Create the test file under desktop/src/ (alongside the module under test)
# desktop/src/__tests__/issue_NNN.test.ts(x)

# Run and confirm it FAILS, then passes with the fix
cd desktop && pnpm vitest issue_NNN   # when vitest is wired up
pnpm build                            # type check + build gate
```

### Step 3: Implement the Fix
- Make the minimal change necessary to fix the bug
- Do not refactor unrelated code in the same commit

### Step 4: Verify the Regression Test Passes
```bash
cargo test regression_NNN  # or equivalent for the platform
```

### Step 5: Run Full Test Suite
```bash
cargo test --workspace
```

### Step 6: Commit with Convention
```
fix(<scope>): <short description>

Fixes #NNN

Regression test: src/regression/issue_NNN_short_desc.rs
```

### PR Checklist
- [ ] Regression test file created and registered
- [ ] Regression test FAILS without the fix (mentioned in PR description)
- [ ] Regression test PASSES with the fix
- [ ] Full test suite passes
- [ ] No unrelated changes in the same commit
