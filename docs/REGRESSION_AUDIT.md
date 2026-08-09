# Code Review Cron: Regression Test Compliance
#
# This config defines a periodic code review check that verifies
# recent bug-fix PRs include regression tests.
#
# Schedule: Run weekly (or on-demand via cron job)
#
# Integration options:
#   - GitHub Actions scheduled workflow
#   - Hermes cron job
#   - Manual script execution

# ─────────────────────────────────────────────────────────────────────
# Hermes Cron Job Config
# ─────────────────────────────────────────────────────────────────────
#
# Add this to your Hermes cron configuration:
#
#   name: "VaultPilot regression test audit"
#   schedule: "0 9 * * 1"   # Every Monday at 09:00
#   prompt: |
#     Review recent VaultPilot PRs (last 7 days) for regression test compliance.
#
#     For each PR labeled 'bug' or containing 'fix' in the title:
#     1. Check if src/regression/ has a corresponding new file
#     2. Check if the source file has a // REGRESSION: #NNN comment
#     3. Report any PRs missing regression tests
#
#     Run: scripts/check-regression-tests.sh
#
#     Report format:
#     - ✅ PR #NNN: Has regression test (issue_NNN_desc.rs)
#     - ⚠️ PR #NNN: Missing regression test
#     - ❌ PR #NNN: Bug label but no test, no REGRESSION comment
#
# ─────────────────────────────────────────────────────────────────────
# GitHub Actions PR-Gate Workflow
# ─────────────────────────────────────────────────────────────────────
#
# The actual workflow is .github/workflows/regression-check.yml:
#
#   on:
#     pull_request:
#       branches: [main]
#
# It runs automatically on every PR, but only takes action
# (posts a warning comment) when the PR has the 'bug' label
# and no regression test changes are detected. This is a soft
# PR gate — it does NOT block merge, it informs.
#
# ─────────────────────────────────────────────────────────────────────
# What to check
# ─────────────────────────────────────────────────────────────────────
#
# For each recent bug-fix PR:
#
# 1. PR has 'bug' label?
#    → Yes: MUST have regression test
#    → No: skip
#
# 2. Regression test exists?
#    → Check src/regression/ for issue_NNN_*.rs
#    → Check inline tests for "// REGRESSION: #NNN" comments
#    → Check desktop/src/ for frontend test files covering the UI bug
#
# 3. Regression test is meaningful?
#    → Not just `assert!(true)`
#    → Actually tests the bug scenario
#
# 4. Report
#    → Slack/email/webhook notification
#    → Or GitHub issue creation
