// Regression test for #3379: Formula && and || have wrong precedence.
//
// Root cause: parse_expression handled both && and || at the same level (left-to-right).
// In all mainstream languages, && binds tighter than ||, so `a || b && c`
// should parse as `a || (b && c)`, not `(a || b) && c`.
//
// Also tests short-circuit evaluation for && and ||.

#[cfg(test)]
mod tests {
    use crate::bases_formula::{evaluate, FmlEnv};
    use crate::models::NoteMeta;
    use std::collections::HashMap;

    fn env() -> FmlEnv<'static> {
        // NoteMeta::default is fine — our test expressions use literals only.
        // We leak the NoteMeta to get a 'static reference for simplicity in tests.
        let note = Box::leak(Box::new(NoteMeta::default()));
        let fv = Box::leak(Box::new(HashMap::new()));
        FmlEnv {
            note,
            formula_values: fv,
        }
    }

    #[test]
    fn regression_3379_or_binds_looser_than_and() {
        let env = env();
        // true || false && false
        // Correct: true || (false && false) = true || false = true
        // Wrong:   (true || false) && false = true && false = false
        assert_eq!(
            evaluate("1 || 0 && 0", &env).to_string(),
            "true",
            "1 || 0 && 0 should be true (|| lower precedence than &&)"
        );
    }

    #[test]
    fn regression_3379_and_binds_tighter_than_or() {
        let env = env();
        // false || true && false
        // Correct: false || (true && false) = false || false = false
        // Wrong:   (false || true) && false = true && false = false (coincidentally same)
        assert_eq!(evaluate("0 || 1 && 0", &env).to_string(), "false",);

        // false || true && true
        // Correct: false || (true && true) = false || true = true
        // Wrong:   (false || true) && true = true && true = true (coincidentally same)
        assert_eq!(evaluate("0 || 1 && 1", &env).to_string(), "true",);
    }

    #[test]
    fn regression_3379_mixed_precedence_chain() {
        let env = env();
        // true && true || false && false
        // Correct: (true && true) || (false && false) = true || false = true
        assert_eq!(evaluate("1 && 1 || 0 && 0", &env).to_string(), "true",);

        // false && true || true && false
        // Correct: (false && true) || (true && false) = false || false = false
        assert_eq!(evaluate("0 && 1 || 1 && 0", &env).to_string(), "false",);
    }

    #[test]
    fn regression_3379_parentheses_override_precedence() {
        let env = env();
        // (true || false) && false — explicit parentheses force wrong-grouping on purpose
        assert_eq!(evaluate("(1 || 0) && 0", &env).to_string(), "false",);

        // true || (false && false) — explicit parentheses
        assert_eq!(evaluate("1 || (0 && 0)", &env).to_string(), "true",);
    }

    #[test]
    fn regression_3379_short_circuit_and() {
        let env = env();
        // 0 && (something) — left is false, right should be skipped
        // Using division: 100/0 in formula returns 0.0 (guard in eval_binop)
        // But with short-circuit, 0 && 100/0 should not even reach division
        // Result should be false regardless
        assert_eq!(
            evaluate("0 && 100 / 0", &env).to_string(),
            "false",
            "&& should short-circuit when left is false"
        );
    }

    #[test]
    fn regression_3379_short_circuit_or() {
        let env = env();
        // 1 || (something) — left is true, right should be skipped
        assert_eq!(
            evaluate("1 || 100 / 0", &env).to_string(),
            "true",
            "|| should short-circuit when left is true"
        );
    }
}
