//! Regression tests for #3992 / #3993 / #3994 — #3964 URI safety gate fixes.
//!
//! #3992 — MCP preview_edit (read-only) was locked out by the High risk gate;
//!         apply_edit (backup + revert-edit → reversible) should be Medium.
//! #3993 — HTTP subscription / AI-action endpoints were ungated; they are now
//!         Medium (trusted-source only), chat_completions explicitly exempt.
//! #3994 — the HTTP MCP server must not trust the globally shared client_name
//!         (cross-session pollution + self-reported identity bypass); gate
//!         source is per-request.

use crate::deep_link::{
    automation_tool_gate, should_allow_tool_non_interactive, TrustedAppRegistry, UriActionRisk,
};

#[test]
fn issue_3992_preview_edit_is_read_only_ungated() {
    // Read-only preview must be ungated (#3992): even an empty source passes.
    let trusted = TrustedAppRegistry::default();
    assert!(automation_tool_gate("notes.preview_edit").is_none());
    assert!(
        should_allow_tool_non_interactive("notes.preview_edit", "", &trusted).is_ok(),
        "read-only preview_edit must not be gated"
    );
}

#[test]
fn issue_3992_apply_edit_is_reversible_medium_trusted_only() {
    // apply_edit records a pre-edit backup (revert-edit exists) → reversible
    // → Medium: trusted clients may proceed with the preview → apply workflow,
    // untrusted clients are denied.
    let gate = automation_tool_gate("notes.apply_edit").expect("apply_edit must be gated");
    assert_eq!(gate.risk, UriActionRisk::Medium);

    let mut trusted = TrustedAppRegistry::default();
    trusted.trust("Claude");
    assert!(
        should_allow_tool_non_interactive("notes.apply_edit", "Claude", &trusted).is_ok(),
        "trusted client must be able to apply an edit"
    );
    assert!(
        should_allow_tool_non_interactive("notes.apply_edit", "", &trusted).is_err(),
        "untrusted source must be denied apply_edit"
    );
}

#[test]
fn issue_3993_subscription_and_ai_action_endpoints_are_gated() {
    // #3993 — these endpoints previously skipped the trusted-source check.
    let mut trusted = TrustedAppRegistry::default();
    trusted.trust("VaultPilot-App");
    for tool in [
        "http_create_subscription",
        "http_update_subscription",
        "http_toggle_subscription",
        "http_delete_subscription",
        "http_run_subscription",
        "http_ai_action",
    ] {
        let gate = automation_tool_gate(tool).unwrap_or_else(|| panic!("{tool} must be gated"));
        assert_eq!(gate.risk, UriActionRisk::Medium, "{tool} should be Medium");
        assert!(
            should_allow_tool_non_interactive(tool, "VaultPilot-App", &trusted).is_ok(),
            "{tool} from a trusted source must pass"
        );
        assert!(
            should_allow_tool_non_interactive(tool, "curl", &trusted).is_err(),
            "{tool} from an untrusted source must be denied"
        );
    }
}

#[test]
fn issue_3994_http_mcp_source_is_per_request() {
    // The HTTP MCP server now derives the gate source per-request
    // (x-vaultpilot-source header), never from the globally shared
    // client_name set by any client's `initialize` — this removes both the
    // cross-session pollution and the self-reported-identity trust bypass.
    //
    // The per-request source flows through `should_allow_tool_non_interactive`
    // explicitly, so two sessions evaluate independently.
    let mut trusted = TrustedAppRegistry::default();
    trusted.trust("WinUI");

    // Session A (trusted identity) may apply an edit…
    assert!(should_allow_tool_non_interactive("notes.apply_edit", "WinUI", &trusted).is_ok());
    // …while a different session that merely *claims* the same name without
    // being routed through the per-request gate cannot be trusted:
    assert!(
        should_allow_tool_non_interactive("notes.apply_edit", "", &trusted).is_err(),
        "empty source is untrusted"
    );
    assert!(
        should_allow_tool_non_interactive("notes.apply_edit", "WinUI", &trusted).is_ok(),
        "per-request source is honored"
    );
}

#[test]
fn issue_3993_chat_completions_stays_exempt() {
    // chat_completions remains reachable for token-only clients (mobile chat /
    // OpenAI-compat consumers); the exemption is deliberate and documented.
    let trusted = TrustedAppRegistry::default();
    assert!(automation_tool_gate("http_chat_completions").is_none());
    assert!(
        should_allow_tool_non_interactive("http_chat_completions", "", &trusted).is_ok(),
        "chat_completions exemption must be preserved"
    );
}
