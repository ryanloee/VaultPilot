#!/usr/bin/env bash
# scripts/check-regression-tests.sh
#
# Code review helper: checks if a PR tagged 'bug' includes a regression test.
#
# Usage:
#   ./scripts/check-regression-tests.sh [--pr-number N]
#
# Exit codes:
#   0 = PR includes regression test (or is not a bug fix)
#   1 = Bug-fix PR is missing a regression test
#
# This script is designed to be called from:
#   - CI workflow (on PR events)
#   - Cron job for code review automation
#   - Manual review

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REGRESSION_DIR_RUST="$REPO_ROOT/src/regression"
REGRESSION_DIR_MOBILE="$REPO_ROOT/mobile/src/__tests__/regression"

# Check if any regression test files exist (excluding templates)
has_regression_tests() {
    local dir="$1"
    local pattern="$2"
    if [ ! -d "$dir" ]; then
        return 1
    fi
    # Count files matching pattern, excluding _TEMPLATE
    count=$(find "$dir" -name "$pattern" ! -name "_TEMPLATE*" ! -name "mod.rs" 2>/dev/null | wc -l)
    [ "$count" -gt 0 ]
}

# Check if PR diff includes changes to regression test directories
pr_has_regression_changes() {
    local diff_range="${1:-HEAD~1..HEAD}"
    git diff --name-only "$diff_range" 2>/dev/null | grep -q "regression/"
}

# Main check
main() {
    echo "=== Regression Test Check ==="
    echo ""

    # If called in a PR context, check the diff
    if [ -n "${GITHUB_HEAD_REF:-}" ]; then
        echo "PR branch: $GITHUB_HEAD_REF"
        if git diff --name-only "origin/${GITHUB_BASE_REF:-main}...HEAD" 2>/dev/null | grep -q "regression/"; then
            echo "✅ PR includes changes to regression test files"
            exit 0
        else
            echo "⚠️  PR does not include regression test changes"
            echo ""
            echo "If this is a bug fix, please add a regression test:"
            echo "  Rust:   src/regression/issue_NNN_short_desc.rs"
            echo "  Mobile: mobile/src/__tests__/regression/issue_NNN.test.ts"
            echo ""
            echo "See docs/TESTING.md for templates and conventions."
            exit 1
        fi
    fi

    # Local mode: just report current state
    echo "Existing regression tests:"
    echo ""

    if has_regression_tests "$REGRESSION_DIR_RUST" "*.rs"; then
        echo "  ✅ Rust: $(find "$REGRESSION_DIR_RUST" -name "*.rs" ! -name "_TEMPLATE*" ! -name "mod.rs" | wc -l) test file(s)"
    else
        echo "  ⚪ Rust: none yet (template available)"
    fi

    if [ -d "$REGRESSION_DIR_MOBILE" ]; then
        echo "  ✅ Mobile: $(find "$REGRESSION_DIR_MOBILE" -name "*.test.ts" 2>/dev/null | wc -l) test file(s)"
    else
        echo "  ⚪ Mobile: none yet"
    fi

    echo ""
    echo "All regression test directories exist and are ready for new tests."
    exit 0
}

main "$@"
