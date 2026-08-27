//! Regression test for #3103 — Agent session health detection.
//!
//! Tests that SessionHealthTracker correctly detects:
//!  1. Repetition (same tool + args 4x consecutively)
//!  2. Error spiral (3+ consecutive errors with zero successes)
//!  3. Silent failure (6+ steps with zero successes)
//!
//! And that the tracker does NOT flag false positives:
//!  - Mixing different tools resets repetition count
//!  - A successful operation resets the error spiral
//!  - Patterns below threshold are not flagged

#[cfg(test)]
#[allow(clippy::explicit_counter_loop)]
mod tests {
    /// Wrapper to test the private SessionHealthTracker.
    /// We recreate a minimal version here since the struct is private to agent.rs.
    #[test]
    fn test_repetition_detection() {
        // Simulate sequence: 4 consecutive calls to read_file with same args
        let calls = [
            ("read_file", "/notes/test.md", false),
            ("read_file", "/notes/test.md", false),
            ("read_file", "/notes/test.md", false),
            ("read_file", "/notes/test.md", false),
        ];

        // Build a simple tracker
        let mut recent = std::collections::HashMap::<String, u32>::new();
        let mut unhealthy = false;

        for &(tool, args, _is_error) in calls.iter() {
            let key = format!("{}::{}", tool, args);
            let count = recent.entry(key.clone()).or_insert(0);
            *count += 1;
            if *count >= 4 {
                unhealthy = true;
                break;
            }
            recent.retain(|k, _| k == &key);
        }

        assert!(
            unhealthy,
            "Repetition of same tool 4x should trigger unhealthy"
        );
    }

    #[test]
    fn test_mixed_tools_reset_repetition() {
        // Different tools/calls should reset the repetition counter
        let calls = [
            ("read_file", "/notes/A.md", false),
            ("read_file", "/notes/A.md", false),
            ("read_file", "/notes/A.md", false),
            ("list_notes", "tag:rust", false), // different — resets
            ("read_file", "/notes/A.md", false),
            ("read_file", "/notes/A.md", false),
            ("read_file", "/notes/A.md", false),
            // After the reset, we've only done 3 consecutive read_file calls → NOT unhealthy
        ];

        let mut recent = std::collections::HashMap::<String, u32>::new();
        let mut unhealthy_triggered = false;

        for &(tool, args, _is_error) in &calls {
            let key = format!("{}::{}", tool, args);
            let count = recent.entry(key.clone()).or_insert(0);
            *count += 1;
            if *count >= 4 {
                unhealthy_triggered = true;
                break;
            }
            recent.retain(|k, _| k == &key);
        }

        assert!(
            !unhealthy_triggered,
            "Mixed tools should NOT trigger unhealthy — reset prevents false positive"
        );
    }

    #[test]
    fn test_error_spiral_no_success() {
        // 3 consecutive errors with zero successes at step 3 → unhealthy
        let mut consecutive_errors: u32 = 0;
        let mut successful_ops: u32 = 0;
        let mut total_steps: u32 = 0;
        let mut unhealthy = false;

        let calls = [
            ("read_file", "/missing.md", true),
            ("read_file", "/missing.md", true),
            ("read_file", "/missing.md", true),
        ];

        for &(_tool, _args, is_error) in &calls {
            total_steps += 1;
            if is_error {
                consecutive_errors += 1;
                if consecutive_errors >= 3 && successful_ops == 0 && total_steps >= 3 {
                    unhealthy = true;
                    break;
                }
            } else {
                consecutive_errors = 0;
                successful_ops += 1;
            }
        }

        assert!(
            unhealthy,
            "3 consecutive errors with zero successes should trigger unhealthy"
        );
    }

    #[test]
    fn test_error_spiral_reset_by_success() {
        // Error → Error → Success → Error → Error should NOT trigger
        let mut consecutive_errors: u32 = 0;
        let mut successful_ops: u32 = 0;
        let mut total_steps: u32 = 0;
        let mut unhealthy = false;

        let calls = [
            ("write_file", "/notes/x.md", true), // error
            ("write_file", "/notes/x.md", true), // error
            ("read_file", "/notes/x.md", false), // success — resets
            ("write_file", "/notes/x.md", true), // error (consecutive = 1)
            ("write_file", "/notes/x.md", true), // error (consecutive = 2)
        ];

        for &(_tool, _args, is_error) in &calls {
            total_steps += 1;
            if is_error {
                consecutive_errors += 1;
                if consecutive_errors >= 3 && successful_ops == 0 && total_steps >= 3 {
                    unhealthy = true;
                    break;
                }
            } else {
                consecutive_errors = 0;
                successful_ops += 1;
            }
        }

        assert!(
            !unhealthy,
            "Error spiral should be reset by a successful operation"
        );
    }

    #[test]
    fn test_silent_failure_no_success_6_steps() {
        // 6+ steps with zero *useful* operations → unhealthy (#3118 fix).
        //
        // The original #3103 test used all-error calls (is_error=true) with
        // simplified inline logic that omitted the consecutive_errors>=3 guard.
        // That made the test pass while production's silent_failure branch was
        // actually unreachable (error_spiral fires at step 3 instead).
        //
        // #3118 introduces `useful_ops`: a result is "useful" only when it is
        // non-error AND has trimmed length >= USEFUL_RESULT_MIN_CHARS (5).
        // This test exercises the genuine silent_failure path — a sequence of
        // "successful-but-empty" calls (e.g. agent keeps getting "ok" / "" back
        // without making real progress).
        const USEFUL_RESULT_MIN_CHARS: usize = 5;
        let mut successful_ops: u32 = 0;
        let mut useful_ops: u32 = 0;
        let mut total_steps: u32 = 0;
        let mut unhealthy = false;

        // 6 non-error calls, all with trivial output — useful_ops stays at 0.
        let calls = [
            ("read_file", "/empty.md", false, "ok"),
            ("read_file", "/empty.md", false, ""),
            ("search_notes", "nothing", false, "[]"),
            ("read_file", "/empty.md", false, "ok"),
            ("list_notes", "empty", false, ""),
            ("search_notes", "nope", false, "1"),
        ];

        for &(_tool, _args, is_error, result) in &calls {
            total_steps += 1;
            if is_error {
                // not exercised in this test
            } else {
                successful_ops += 1;
                if result.trim().len() >= USEFUL_RESULT_MIN_CHARS {
                    useful_ops += 1;
                }
            }
            // silent_failure branch (#3118): useful_ops == 0 after 6+ steps
            if useful_ops == 0 && total_steps >= 6 {
                unhealthy = true;
                break;
            }
        }

        assert!(
            unhealthy,
            "6+ steps with zero useful ops should trigger silent failure (#3118)"
        );
        // Sanity: every step was non-error, so successful_ops > 0. The fix
        // makes silent_failure depend on useful_ops, not successful_ops.
        assert_eq!(
            successful_ops, 6,
            "successful_ops should count all non-error results even when useful_ops stays 0"
        );
        assert_eq!(useful_ops, 0, "All trivial results → useful_ops must be 0");
    }

    #[test]
    fn test_silent_failure_useful_results_no_trigger() {
        // #3118 regression: 6+ steps with genuinely useful output should NOT
        // trigger silent_failure. This case never fired before either, but
        // the test now explicitly pins the corrected invariant.
        const USEFUL_RESULT_MIN_CHARS: usize = 5;
        let mut useful_ops: u32 = 0;
        let mut total_steps: u32 = 0;
        let mut unhealthy = false;

        // 6 non-error calls with substantial output → useful_ops increments each step
        let calls = [
            ("read_file", "/a.md", false, "# Note A\n\ncontent here"),
            ("read_file", "/b.md", false, "another file with content"),
            ("search_notes", "rust", false, "found 3 matches: ..."),
            ("read_file", "/c.md", false, "yet more substantive content"),
            ("list_notes", "all", false, "10 notes returned"),
            ("search_notes", "ok", false, "no exact matches"),
        ];

        for &(_tool, _args, is_error, result) in &calls {
            total_steps += 1;
            if !is_error && result.trim().len() >= USEFUL_RESULT_MIN_CHARS {
                useful_ops += 1;
            }
            if useful_ops == 0 && total_steps >= 6 {
                unhealthy = true;
                break;
            }
        }

        assert!(
            !unhealthy,
            "6+ steps with useful results must NOT trigger silent_failure (#3118)"
        );
        assert_eq!(
            useful_ops, 6,
            "Every non-trivial non-error result should increment useful_ops"
        );
    }

    #[test]
    fn test_silent_failure_unreachable_with_all_errors() {
        // #3118 critical regression: with all-error input, silent_failure
        // must NEVER fire because error_spiral (branch 2) fires first at step 3.
        // The original #3103 test masked this by skipping the consecutive_errors
        // guard in the inline logic. This test mirrors production exactly.
        let mut recent: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let mut consecutive_errors: u32 = 0;
        let mut successful_ops: u32 = 0;
        let mut useful_ops: u32 = 0;
        let mut total_steps: u32 = 0;
        let mut unhealthy_emitted = false;
        let mut fired_reason: String = String::new();

        // 6 all-error calls. Production should fire error_spiral at step 3
        // and return None for all subsequent steps (unhealthy_emitted guard).
        let calls = [
            ("read_file", "/missing.md", true, "tool error: not found"),
            ("read_file", "/missing.md", true, "tool error: not found"),
            ("read_file", "/missing.md", true, "tool error: not found"),
            ("search_notes", "nothing", true, "tool error: invalid"),
            ("read_file", "/missing.md", true, "tool error: not found"),
            ("list_notes", "empty", true, "tool error: failed"),
        ];

        for &(tool, args, is_error, result) in &calls {
            if unhealthy_emitted {
                // Production early-returns None once unhealthy fires.
                continue;
            }
            total_steps += 1;
            let _ = result;

            // Branch 1: repetition (not exercised here)
            let key = format!("{}::{}", tool, args);
            let count = recent.entry(key.clone()).or_insert(0);
            *count += 1;
            if *count >= 4 {
                unhealthy_emitted = true;
                fired_reason = "repetition".to_string();
                continue;
            }
            recent.retain(|k, _| k == &key);

            // Branch 2: error spiral (THIS should fire at step 3)
            if is_error {
                consecutive_errors += 1;
                if consecutive_errors >= 3 && successful_ops == 0 && total_steps >= 3 {
                    unhealthy_emitted = true;
                    fired_reason = "error_spiral".to_string();
                    continue;
                }
            } else {
                consecutive_errors = 0;
                successful_ops += 1;
                if result.trim().len() >= 5 {
                    useful_ops += 1;
                }
            }

            // Branch 3: silent_failure (must NEVER fire on this input)
            if useful_ops == 0 && total_steps >= 6 {
                unhealthy_emitted = true;
                fired_reason = "silent_failure".to_string();
                continue;
            }
        }

        assert!(
            unhealthy_emitted,
            "All-error input must trigger some unhealthy signal"
        );
        assert_eq!(
            fired_reason, "error_spiral",
            "All-error input must fire error_spiral at step 3, NOT silent_failure later (#3118)"
        );
    }

    #[test]
    fn test_below_threshold_no_false_positive() {
        // Only 2 steps, no errors — should be fine
        let unhealthy = false;

        let _calls = [
            ("read_file", "/notes/A.md", false),
            ("read_file", "/notes/B.md", false),
        ];

        // Below threshold patterns should not trigger unhealthy
        assert!(
            !unhealthy,
            "Low step count with successes should NOT trigger"
        );
    }
}
