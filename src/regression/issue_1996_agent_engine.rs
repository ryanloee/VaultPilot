/// Issue #1996: 多 Agent 引擎接入 — agent_engine adapter layer should expose a
/// uniform registry/context/response model for builtin + external (Claude
/// Code/Codex) agents, vault-scoped, without spawning real CLIs in tests.
///
/// Feature: Multi-Agent engine adapter layer (#1996).
#[cfg(test)]
mod tests {
    use crate::agent_engine::*;
    use std::path::PathBuf;

    /// Create a unique real temp directory. No agent CLI is ever spawned by
    /// these tests — we only exercise the registry, context, and availability
    /// logic.
    fn temp_vault() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot_regression_1996_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp vault");
        dir
    }

    #[test]
    fn regression_1996_registry_lists_builtin_claude_code_codex() {
        let registry = AgentEngineRegistry::new();
        let infos = registry.engine_infos();
        assert!(
            infos.len() >= 3,
            "registry should expose at least 3 engines, got {}",
            infos.len()
        );

        let names: Vec<&str> = infos.iter().map(|i| i.name.as_str()).collect();
        assert!(
            names.contains(&"builtin"),
            "missing 'builtin' engine: {names:?}"
        );
        assert!(
            names.contains(&"claude-code"),
            "missing 'claude-code' engine: {names:?}"
        );
        assert!(
            names.contains(&"codex"),
            "missing 'codex' engine: {names:?}"
        );

        // The builtin engine has no external dependency and must always be
        // available.
        let builtin = infos
            .iter()
            .find(|i| i.name == "builtin")
            .expect("builtin engine present");
        assert!(
            builtin.available,
            "builtin engine must report available == true"
        );
    }

    #[test]
    fn regression_1996_select_by_name_case_insensitive() {
        let registry = AgentEngineRegistry::new();

        // Known engines resolve.
        assert!(
            registry.select("builtin").is_some(),
            "select('builtin') should return Some"
        );
        assert!(
            registry.select("claude-code").is_some(),
            "select('claude-code') should return Some"
        );

        // Unknown engines return None.
        assert!(
            registry.select("does-not-exist").is_none(),
            "select('does-not-exist') should return None"
        );

        // Selection is case-insensitive.
        let upper = registry
            .select("BUILTIN")
            .expect("select('BUILTIN') should resolve case-insensitively");
        assert_eq!(upper.name(), "builtin");
    }

    #[test]
    fn regression_1996_builtin_engine_send_prompt_bails_to_agent_command() {
        let mut engine = BuiltinEngine;
        // The builtin engine is always available (it is the in-process loop).
        assert!(engine.available(), "BuiltinEngine must always be available");

        // It deliberately does NOT run through this adapter — it redirects the
        // caller to the dedicated `agent` command. Assert Err, never unwrap Ok.
        let vault = temp_vault();
        let ctx = EngineContext::new(&vault);
        let result = engine.send_prompt("hello", &ctx);
        assert!(
            result.is_err(),
            "BuiltinEngine::send_prompt must return Err (it defers to the `agent` command)"
        );
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn regression_1996_engine_context_compose_and_validate() {
        // compose_prompt surfaces capabilities + task text.
        let vault = temp_vault();
        let ctx = EngineContext::new(&vault)
            .with_capabilities(vec!["search_notes".to_string(), "read_note".to_string()]);
        let composed = ctx.compose_prompt("summarize today");
        assert!(
            composed.contains("search_notes"),
            "composed prompt should list the 'search_notes' capability"
        );
        assert!(
            composed.contains("read_note"),
            "composed prompt should list the 'read_note' capability"
        );
        assert!(
            composed.contains("summarize today"),
            "composed prompt should include the user task text"
        );

        // A context rooted at a real temp dir validates cleanly.
        assert!(
            ctx.validate().is_ok(),
            "validate() should succeed for an existing directory"
        );
        let _ = std::fs::remove_dir_all(&vault);

        // A context rooted at a path that does not exist must fail validation.
        let bad = EngineContext::new("/vaultpilot/definitely/does/not/exist/1996");
        assert!(
            bad.validate().is_err(),
            "validate() should fail for a non-existent vault_dir"
        );
    }

    #[test]
    fn regression_1996_subprocess_engine_fail_closed_when_binary_missing() {
        // An engine whose backing CLI is guaranteed not to be on PATH must
        // report unavailable — proving the fail-closed behaviour. We do NOT
        // call send_prompt here (that would attempt to resolve/run a binary).
        let engine = SubprocessEngine::new(
            "vaultpilot-no-such-agent-binary-xyz",
            vec!["vaultpilot-no-such-agent-binary-xyz".to_string()],
            Vec::new(),
            true,
            "deterministic missing-binary engine (never installed)",
        );
        assert!(
            !engine.available(),
            "an engine with no backing binary on PATH must be unavailable"
        );

        // The public factories build the same SubprocessEngine machinery. They
        // are also unavailable whenever their CLI is absent; we only assert the
        // deterministic case above to keep this test hermetic, but exercising
        // the constructors confirms the public API is wired through the
        // subprocess adapter.
        let _claude = claude_code_engine();
        let _codex = codex_engine();
    }
}
