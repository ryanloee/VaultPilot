//! Intelligent model routing (#1842).
//!
//! Inspects each outgoing request and selects the most appropriate model for
//! its task type:
//!
//! - **Code** tasks (contain code fences or programming keywords) →
//!   `code_task_model`
//! - **Complex** tasks (long-form analysis / reasoning / multi-step) →
//!   `complex_task_model`
//! - **Simple** tasks (short Q&A / translation / summary) →
//!   `simple_task_model`
//!
//! Routing is **off by default** and only activates once the user enables it
//! and configures per-task models. When routing is inactive, or no model is
//! configured for the detected task type, the active provider's default model
//! is used unchanged.
//!
//! The classifier is deliberately fast: it scans the input with simple
//! substring/keyword checks and a length threshold — no regex, no
//! allocations beyond a lowercase copy of the input — so routing adds no
//! measurable latency.

use crate::models::{ModelRoutingConfig, ProviderConfig};

/// The broad category of an AI request, used to pick a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    /// Short Q&A, translation, summaries — cheap model territory.
    Simple,
    /// Long-form analysis, reasoning, multi-step instructions.
    Complex,
    /// Contains code blocks or programming keywords — benefits from a
    /// code-oriented model.
    Code,
}

impl TaskType {
    /// Stable, human-readable label used for logging and CLI display.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Complex => "complex",
            Self::Code => "code",
        }
    }
}

/// The outcome of a routing decision.
///
/// When routing selects a model, [`Self::model`] holds the chosen model name
/// and [`Self::reason`] explains why. When routing is inactive or no
/// per-task override applies, [`route`] returns `None` and the caller uses the
/// provider's default model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingDecision {
    pub task_type: TaskType,
    pub model: String,
    pub reason: &'static str,
}

// ── Tunable heuristics ───────────────────────────────────────────────────

/// Prompts (system + user) whose combined length exceeds this many characters
/// are treated as "complex". Chosen to stay well below typical chat lengths
/// while still flagging documents/analysis-style requests.
const COMPLEX_LENGTH_THRESHOLD: usize = 1500;

/// Programming languages and tooling keywords that, when present, strongly
/// suggest a code task. Checked case-insensitively as whole-word-ish tokens.
/// Kept short on purpose — this runs on every request.
const CODE_KEYWORDS: &[&str] = &[
    // Fenced code blocks / inline code.
    "```",
    "`",
    // Common language & ecosystem tokens.
    "python",
    "rust",
    "javascript",
    "typescript",
    "java ",
    "golang",
    "c++",
    "c#",
    "sql",
    "shell",
    "bash",
    "regex",
    "docker",
    "kubernetes",
    // Programming constructs / verbs.
    "function ",
    "def ",
    "class ",
    "import ",
    "return ",
    "compile",
    "compileerror",
    "stacktrace",
    "stack trace",
    "bug ",
    "debug",
    "refactor",
    "algorithm",
    "api endpoint",
    "unit test",
];

/// Keywords indicating a complex / high-effort task. Their presence (in a
/// prompt that isn't short and trivial) bumps the classification to Complex.
const COMPLEX_KEYWORDS: &[&str] = &[
    "analyze",
    "analyse",
    "compare",
    "contrast",
    "evaluate",
    "design",
    "architect",
    "strategy",
    "step by step",
    "reasoning",
    "derive",
    "prove",
    "investigate",
    "brainstorm",
    "plan in detail",
    "in-depth",
    "comprehensive",
    "trade-off",
    "tradeoff",
];

/// Classify a request into a [`TaskType`] using fast heuristics.
///
/// Inputs are the system prompt and the user prompt exactly as they will be
/// sent to the provider. The combined text is scanned once.
///
/// Priority: **Code** (most specific) > **Complex** > **Simple**.
pub fn classify_task(system: &str, prompt: &str) -> TaskType {
    // Fast path: a tiny request with no system prompt is almost certainly simple.
    let combined_len = system.len() + prompt.len();
    if combined_len == 0 {
        return TaskType::Simple;
    }

    // Build a single lowercase scratch buffer to scan once for all keywords.
    // We scan the prompt first since code is most likely to appear there, but
    // keyword detection considers the full combined text.
    let prompt_lower = prompt.to_ascii_lowercase();
    let system_lower = system.to_ascii_lowercase();

    // ── Code detection ──
    // A fenced code block is a strong signal on its own.
    if prompt_lower.contains("```") {
        return TaskType::Code;
    }
    for kw in CODE_KEYWORDS {
        if prompt_lower.contains(kw) || system_lower.contains(kw) {
            return TaskType::Code;
        }
    }

    // ── Complex detection ──
    if combined_len >= COMPLEX_LENGTH_THRESHOLD {
        return TaskType::Complex;
    }
    for kw in COMPLEX_KEYWORDS {
        if prompt_lower.contains(kw) || system_lower.contains(kw) {
            return TaskType::Complex;
        }
    }

    // ── Default: simple ──
    TaskType::Simple
}

/// Pick the model for a request, applying routing config when active.
///
/// Returns `Some(RoutingDecision)` when a per-task model override applies,
/// or `None` when routing is disabled / no model is configured for the
/// detected task (in which case the provider's default model is used).
pub fn route(
    config: &ModelRoutingConfig,
    provider: &ProviderConfig,
    system: &str,
    prompt: &str,
) -> Option<RoutingDecision> {
    if !config.is_active() {
        return None;
    }

    let task = classify_task(system, prompt);
    let candidate = match task {
        TaskType::Code => config.code_task_model.as_deref(),
        TaskType::Complex => config.complex_task_model.as_deref(),
        TaskType::Simple => config.simple_task_model.as_deref(),
    };

    let model = candidate?;
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return None;
    }

    // If the routed model equals the provider's configured model, there is
    // nothing to switch — report it as a no-op so callers don't log a
    // misleading "switched model" message.
    if trimmed.eq_ignore_ascii_case(provider.model.trim()) {
        return None;
    }

    let reason = match task {
        TaskType::Code => "code task (code block or programming keyword detected)",
        TaskType::Complex => "complex task (long input or reasoning keyword detected)",
        TaskType::Simple => "simple task (short Q&A / translation / summary)",
    };

    Some(RoutingDecision {
        task_type: task,
        model: trimmed.to_string(),
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ModelRoutingConfig::is_active ──

    #[test]
    fn is_active_false_when_disabled() {
        let cfg = ModelRoutingConfig {
            enabled: false,
            simple_task_model: Some("m".into()),
            ..ModelRoutingConfig::default()
        };
        assert!(!cfg.is_active());
    }

    #[test]
    fn is_active_false_when_enabled_but_no_models() {
        let cfg = ModelRoutingConfig {
            enabled: true,
            ..ModelRoutingConfig::default()
        };
        assert!(!cfg.is_active());
    }

    #[test]
    fn is_active_true_when_enabled_with_any_model() {
        let cfg = ModelRoutingConfig {
            enabled: true,
            simple_task_model: Some("cheap-model".into()),
            ..ModelRoutingConfig::default()
        };
        assert!(cfg.is_active());
    }

    // ── classify_task ──

    #[test]
    fn classify_empty_is_simple() {
        assert_eq!(classify_task("", ""), TaskType::Simple);
    }

    #[test]
    fn classify_short_qa_is_simple() {
        assert_eq!(
            classify_task("", "What is the capital of France?"),
            TaskType::Simple
        );
        assert_eq!(
            classify_task("", "Translate hello to French"),
            TaskType::Simple
        );
        assert_eq!(
            classify_task("", "Summarize: the cat sat on the mat."),
            TaskType::Simple
        );
    }

    #[test]
    fn classify_code_fence_is_code() {
        assert_eq!(
            classify_task("", "Fix this:\n```rust\nfn main() {}\n```"),
            TaskType::Code
        );
    }

    #[test]
    fn classify_programming_keyword_is_code() {
        assert_eq!(
            classify_task("", "Explain how python decorators work"),
            TaskType::Code
        );
        assert_eq!(
            classify_task("", "debug this stacktrace please"),
            TaskType::Code
        );
        assert_eq!(classify_task("", "refactor the function "), TaskType::Code);
    }

    #[test]
    fn classify_long_input_is_complex() {
        let long = "x".repeat(COMPLEX_LENGTH_THRESHOLD + 1);
        assert_eq!(classify_task("", &long), TaskType::Complex);
    }

    #[test]
    fn classify_reasoning_keyword_is_complex() {
        assert_eq!(
            classify_task("", "analyze the trade-offs of microservices"),
            TaskType::Complex
        );
        assert_eq!(
            classify_task("", "compare and contrast these options"),
            TaskType::Complex
        );
        assert_eq!(
            classify_task("", "design a scalable architecture step by step"),
            TaskType::Complex
        );
    }

    #[test]
    fn classify_code_beats_complex() {
        // A long prompt containing a code fence is classified as code, not complex.
        let long_code = format!(
            "```python\n{}\n```",
            "x".repeat(COMPLEX_LENGTH_THRESHOLD + 1)
        );
        assert_eq!(classify_task("", &long_code), TaskType::Code);
    }

    #[test]
    fn classify_system_prompt_considered() {
        // Programming keyword in the system prompt should trigger code routing.
        assert_eq!(
            classify_task("You are a rust expert.", "explain ownership"),
            TaskType::Code
        );
    }

    // ── route ──

    fn provider_with_model(model: &str) -> ProviderConfig {
        ProviderConfig {
            model: model.to_string(),
            ..ProviderConfig::default()
        }
    }

    #[test]
    fn route_returns_none_when_disabled() {
        let cfg = ModelRoutingConfig {
            enabled: false,
            simple_task_model: Some("cheap".into()),
            ..ModelRoutingConfig::default()
        };
        let provider = provider_with_model("default");
        assert!(route(&cfg, &provider, "", "hello").is_none());
    }

    #[test]
    fn route_returns_none_when_no_model_for_task() {
        // Only code model configured, but the task is simple.
        let cfg = ModelRoutingConfig {
            enabled: true,
            code_task_model: Some("code-model".into()),
            ..ModelRoutingConfig::default()
        };
        let provider = provider_with_model("default");
        assert!(route(&cfg, &provider, "", "What is 2+2?").is_none());
    }

    #[test]
    fn route_simple_uses_simple_model() {
        let cfg = ModelRoutingConfig {
            enabled: true,
            simple_task_model: Some("haiku-cheap".into()),
            complex_task_model: Some("sonnet-strong".into()),
            code_task_model: Some("coder".into()),
        };
        let provider = provider_with_model("default");
        let decision = route(&cfg, &provider, "", "Translate this to French").expect("routed");
        assert_eq!(decision.task_type, TaskType::Simple);
        assert_eq!(decision.model, "haiku-cheap");
        assert!(decision.reason.contains("simple task"));
    }

    #[test]
    fn route_complex_uses_complex_model() {
        let cfg = ModelRoutingConfig {
            enabled: true,
            complex_task_model: Some("sonnet-strong".into()),
            ..ModelRoutingConfig::default()
        };
        let provider = provider_with_model("default");
        let long = "x".repeat(COMPLEX_LENGTH_THRESHOLD + 1);
        let decision = route(&cfg, &provider, "", &long).expect("routed");
        assert_eq!(decision.task_type, TaskType::Complex);
        assert_eq!(decision.model, "sonnet-strong");
    }

    #[test]
    fn route_code_uses_code_model() {
        let cfg = ModelRoutingConfig {
            enabled: true,
            code_task_model: Some("deepseek-coder".into()),
            ..ModelRoutingConfig::default()
        };
        let provider = provider_with_model("default");
        let decision = route(&cfg, &provider, "", "```python\nprint(1)\n```").expect("routed");
        assert_eq!(decision.task_type, TaskType::Code);
        assert_eq!(decision.model, "deepseek-coder");
    }

    #[test]
    fn route_no_op_when_routed_equals_provider_model() {
        // If the routed model is the same as the provider's model, no switch is
        // needed — route() returns None so callers don't log a spurious switch.
        let cfg = ModelRoutingConfig {
            enabled: true,
            simple_task_model: Some("same-model".into()),
            ..ModelRoutingConfig::default()
        };
        let provider = provider_with_model("same-model");
        assert!(route(&cfg, &provider, "", "hi").is_none());
    }

    #[test]
    fn route_ignores_empty_or_whitespace_model() {
        let cfg = ModelRoutingConfig {
            enabled: true,
            simple_task_model: Some("   ".into()),
            ..ModelRoutingConfig::default()
        };
        let provider = provider_with_model("default");
        assert!(route(&cfg, &provider, "", "hi").is_none());
    }

    #[test]
    fn route_is_case_insensitive_for_equality_check() {
        // Different case but same model name => no-op.
        let cfg = ModelRoutingConfig {
            enabled: true,
            simple_task_model: Some("GPT-4o".into()),
            ..ModelRoutingConfig::default()
        };
        let provider = provider_with_model("gpt-4o");
        assert!(route(&cfg, &provider, "", "hi").is_none());
    }
}
