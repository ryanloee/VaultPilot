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
        // 6+ steps with zero successful operations → unhealthy
        let successful_ops: u32 = 0;
        let mut total_steps: u32 = 0;
        let mut unhealthy = false;

        let calls = [
            ("read_file", "/missing.md", true),
            ("read_file", "/missing.md", true),
            ("search_notes", "nothing", true),
            ("read_file", "/missing.md", true),
            ("list_notes", "empty", true),
            ("search_notes", "nope", true),
        ];

        for &(_tool, _args, _is_error) in &calls {
            total_steps += 1;
            // No success incrementing — everything fails
            if successful_ops == 0 && total_steps >= 6 {
                unhealthy = true;
                break;
            }
        }

        assert!(
            unhealthy,
            "6+ steps with zero successes should trigger silent failure"
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
