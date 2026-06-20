//! Regression tests for VaultPilot bugs.
//!
//! Each module is named after the issue it covers:
//!   - `issue_042_empty_vault` — regression for issue #42
//!   - `issue_099_search_crash` — regression for issue #99
//!
//! See `docs/TESTING.md` for the template and conventions.
//!
//! ### Adding a new regression test:
//! 1. Copy `src/regression/_TEMPLATE.rs` → `src/regression/issue_NNN_desc.rs`
//! 2. Add `mod issue_NNN_desc;` below (keep alphabetical by issue number)
//! 3. Fill in the test details
//! 4. Run: `cargo test regression_NNN`

// ┌────────────────────────────────────────────────────────────────────┐
// │ Add new regression test modules here. Keep alphabetical by issue. │
// └────────────────────────────────────────────────────────────────────┘
//
// mod issue_042_empty_vault;
// mod issue_099_search_crash;
mod issue_1175_missing_test_fields;

// Example — reference only, delete once you have real regression tests
mod _example_042;
