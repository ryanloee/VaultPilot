use crate::models::AppSettings;
use tracing::debug;

pub fn is_openai_reasoning_model(model: &str) -> bool {
    // #741: Handle namespaced model names from proxy services (OpenRouter, Together, etc.)
    let effective_name = model.rsplit('/').next().unwrap_or(model);
    // Known reasoning model prefixes: o1, o3, o4 (with optional suffixes like -mini, -preview)
    for prefix in &["o1", "o3", "o4"] {
        if let Some(rest) = effective_name.strip_prefix(prefix) {
            // Exact match or followed by a separator (not a letter)
            if rest.is_empty() || !rest.as_bytes()[0].is_ascii_alphabetic() {
                return true;
            }
        }
    }
    false
}

/// A model context window rule for the built-in registry.
///
/// All `substrings` must appear in the model name (lowercased) for the rule to
/// match. Rules are evaluated in order — place more specific patterns before
/// general ones (e.g. `["claude", "1m"]` before `["claude"]`).
struct ContextWindowRule {
    substrings: &'static [&'static str],
    tokens: usize,
}

/// Built-in model context window registry.
///
/// To add support for a new model, append an entry here — no other code changes
/// required. Order matters: first matching rule wins, so put more specific
/// patterns before general ones.
static MODEL_CONTEXT_RULES: &[ContextWindowRule] = &[
    // ── Anthropic ────────────────────────────────────────────────────
    ContextWindowRule {
        substrings: &["claude", "1m"],
        tokens: 1_000_000,
    },
    ContextWindowRule {
        substrings: &["claude"],
        tokens: 200_000,
    },
    // ── Zhipu ────────────────────────────────────────────────────────
    ContextWindowRule {
        substrings: &["glm-5.1"],
        tokens: 200_000,
    },
    // ── OpenAI ───────────────────────────────────────────────────────
    ContextWindowRule {
        substrings: &["gpt-4.1"],
        tokens: 1_047_576,
    },
    ContextWindowRule {
        substrings: &["gpt-5"],
        tokens: 1_047_576,
    },
    ContextWindowRule {
        substrings: &["gpt-4o"],
        tokens: 128_000,
    },
    // ── Google ───────────────────────────────────────────────────────
    ContextWindowRule {
        substrings: &["gemini"],
        tokens: 1_000_000,
    },
];

pub fn resolve_context_window(settings: &AppSettings) -> (usize, String) {
    // Priority 1: explicit user configuration
    if let Some(explicit) = settings
        .provider
        .context_window_tokens
        .filter(|value| *value > 0)
    {
        debug!(
            tokens = explicit,
            source = "manual_override",
            "context window"
        );
        return (explicit, "manual_override".to_string());
    }

    let model = settings
        .effective_provider()
        .model
        .trim()
        .to_ascii_lowercase();

    // Priority 2: built-in registry (data-driven)
    for rule in MODEL_CONTEXT_RULES {
        if rule.substrings.iter().all(|s| model.contains(s)) {
            debug!(
                tokens = rule.tokens,
                model = %model,
                source = "model_registry",
                "context window",
            );
            return (rule.tokens, "model_registry".to_string());
        }
    }

    // Priority 3: OpenAI reasoning models (prefix-based, not substring)
    if is_openai_reasoning_model(&model) {
        debug!(
            tokens = 200_000,
            model = %model,
            source = "model_registry",
            "context window",
        );
        return (200_000, "model_registry".to_string());
    }

    // Priority 4: heuristic default
    debug!(
        tokens = 128_000,
        model = %model,
        source = "heuristic_default",
        "context window",
    );
    (128_000, "heuristic_default".to_string())
}

/// A model output-token rule for the built-in registry.
///
/// Same matching semantics as `ContextWindowRule`: all `substrings` must appear
/// in the lowercased model name for the rule to match.  First match wins.
struct OutputTokenRule {
    substrings: &'static [&'static str],
    tokens: u32,
}

/// Data-driven output token limits by model family.
///
/// Order matters: more specific patterns before general ones.
static MODEL_OUTPUT_TOKEN_RULES: &[OutputTokenRule] = &[
    // ── Anthropic ────────────────────────────────────────────────────
    // Opus variants have a lower default output ceiling.
    OutputTokenRule {
        substrings: &["claude", "opus"],
        tokens: 4096,
    },
    // ── OpenAI ───────────────────────────────────────────────────────
    OutputTokenRule {
        substrings: &["gpt-4.1"],
        tokens: 16384,
    },
    OutputTokenRule {
        substrings: &["gpt-5"],
        tokens: 16384,
    },
    OutputTokenRule {
        substrings: &["gpt-4o"],
        tokens: 16384,
    },
];

/// Resolve the maximum output tokens to use for an API request.
///
/// Priority: explicit user configuration > model-name registry > reasoning
/// model heuristic > default (8192).
pub fn resolve_max_output_tokens(model: &str, configured: Option<u32>) -> u32 {
    if let Some(value) = configured.filter(|v| *v > 0) {
        return value;
    }

    let model_lower = model.trim().to_ascii_lowercase();

    // Data-driven registry (same pattern as resolve_context_window).
    for rule in MODEL_OUTPUT_TOKEN_RULES {
        if rule.substrings.iter().all(|s| model_lower.contains(s)) {
            return rule.tokens;
        }
    }

    // Reasoning models (o1/o3/o4) support 32768+ output tokens (#742).
    if is_openai_reasoning_model(&model_lower) {
        return 32768;
    }

    8192
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_tokens_claude_opus_returns_4096() {
        assert_eq!(resolve_max_output_tokens("claude-3-opus", None), 4096);
        assert_eq!(resolve_max_output_tokens("claude-opus-4", None), 4096);
        assert_eq!(
            resolve_max_output_tokens("anthropic/claude-opus-4", None),
            4096
        );
    }

    #[test]
    fn output_tokens_gpt_models_return_16384() {
        assert_eq!(resolve_max_output_tokens("gpt-4o", None), 16384);
        assert_eq!(resolve_max_output_tokens("gpt-4.1", None), 16384);
        assert_eq!(resolve_max_output_tokens("gpt-5", None), 16384);
        assert_eq!(resolve_max_output_tokens("openai/gpt-4o", None), 16384);
    }

    #[test]
    fn output_tokens_reasoning_models_return_32768() {
        assert_eq!(resolve_max_output_tokens("o1-mini", None), 32768);
        assert_eq!(resolve_max_output_tokens("o3-mini", None), 32768);
        assert_eq!(resolve_max_output_tokens("o4-mini", None), 32768);
    }

    #[test]
    fn output_tokens_unknown_model_returns_8192() {
        assert_eq!(resolve_max_output_tokens("unknown-model", None), 8192);
        assert_eq!(resolve_max_output_tokens("deepseek-chat", None), 8192);
        assert_eq!(resolve_max_output_tokens("gemini-pro", None), 8192);
    }

    #[test]
    fn output_tokens_explicit_override_wins() {
        assert_eq!(resolve_max_output_tokens("gpt-4o", Some(4096)), 4096);
        assert_eq!(resolve_max_output_tokens("o1-mini", Some(1000)), 1000);
        // Zero and None should fall through to heuristic.
        assert_eq!(resolve_max_output_tokens("gpt-4o", Some(0)), 16384);
        assert_eq!(resolve_max_output_tokens("gpt-4o", None), 16384);
    }

    #[test]
    fn output_tokens_claude_non_opus_returns_default() {
        // Sonnet / Haiku should get the default, not Opus's 4096.
        assert_eq!(resolve_max_output_tokens("claude-3-5-sonnet", None), 8192);
        assert_eq!(resolve_max_output_tokens("claude-3-haiku", None), 8192);
    }

    #[test]
    fn output_tokens_registry_rules_are_addable() {
        // Verify the static array is non-empty and contains expected entries.
        assert!(!MODEL_OUTPUT_TOKEN_RULES.is_empty());
        // First rule should be claude+opus.
        assert_eq!(MODEL_OUTPUT_TOKEN_RULES[0].substrings, &["claude", "opus"]);
        assert_eq!(MODEL_OUTPUT_TOKEN_RULES[0].tokens, 4096);
    }
}
