//! Regression / integration tests for #2825 — post-merge integration coverage
//! for the batch of features merged in 2026-07.
//!
//! The bug report flagged that many independently-developed modules
//! (#2803 Ollama provider, #2814 vault structured query export, #2801 connector
//! catalog + PDF extraction, #2769 MCP client config, #2768 shared capability
//! registry, #2821 agent trigger rule data model) lacked *cross-feature*
//! integration tests. The tests below lock in the composition contracts so a
//! future refactor can't silently break how these modules talk to each other:
//!
//!   1. Connector catalog  ↔ Capability registry  (register a connector as MCP)
//!   2. Ollama provider    ↔ ProviderConfig model (no-auth config composes)
//!   3. MCP config         ↔ Capability registry  (shared config round-trip)
//!   4. PDF extraction     → vault_query pipeline (extracted text feeds query)

use crate::capability_registry::{AuthConfig, CapabilityRegistry, McpTransport};
use crate::connector::{connector_catalog, find_connector_info};
use crate::file_parsing::{FileParser, PdfParser};
use crate::models::provider::{ProviderConfig, ProviderType};
use crate::vault_query::{parse_query, query_records, QValue, Record};

use std::fs;
use std::path::Path;

/// #2825 (combo 1): the connector catalog is the source of truth for built-in
/// connectors, and a connector entry must be registrable into the shared
/// capability registry without loss of identity (round-trip through disk).
#[test]
fn regression_2825_connector_catalog_integrates_with_capability_registry() {
    // Catalog must be non-empty and self-consistent.
    let catalog = connector_catalog();
    assert!(!catalog.is_empty(), "connector catalog must not be empty");

    // `find_connector_info` must resolve a known type and agree with the catalog.
    let github =
        find_connector_info("github").expect("github connector must be present in catalog");
    assert_eq!(github.connector_type, "github");
    assert!(catalog
        .iter()
        .any(|c| c.connector_type == github.connector_type));

    // A connector blueprint can be surfaced as an MCP server in the shared
    // capability registry and persisted + reloaded intact.
    let dir = std::env::temp_dir().join(format!("vp_regr_2825_conn_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    {
        let mut reg = CapabilityRegistry::default();
        reg.add_server(
            &format!("mcp-{}", github.connector_type),
            &github.label,
            "GitHub connector exposed as an MCP server",
            McpTransport::Http {
                url: "http://localhost:9000/mcp".to_string(),
            },
            None,
        );
        reg.save(&dir)
            .expect("capability registry save must succeed");

        let reloaded = CapabilityRegistry::load(&dir).expect("capability registry load");
        assert!(
            reloaded.list_ids().contains(&"mcp-github"),
            "connector-derived MCP server must survive save/load round-trip"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

/// #2825 (combo 2): the local Ollama provider is no-auth and localhost-allowed.
/// A `ProviderConfig` built for Ollama (empty API key) must validate cleanly and
/// compose with the rest of the provider model — this is the contract that lets
/// the AI layer drive a local LLM without credential friction.
#[test]
fn regression_2825_ollama_provider_no_auth_composes() {
    // URL auto-detection must classify the Ollama endpoint as Ollama.
    let ptype = ProviderType::from_base_url("http://localhost:11434/v1/chat/completions");
    assert_eq!(ptype, ProviderType::Ollama);

    // Ollama must not require an API key and must allow local endpoints.
    assert!(
        !ptype.requires_api_key(),
        "Ollama must not require an API key"
    );
    assert!(
        ptype.allows_local_endpoint(),
        "Ollama must allow local endpoints"
    );

    // A fully-specified Ollama provider config (no key) must pass validation.
    let cfg = ProviderConfig {
        name: "Local Ollama".into(),
        api_key: String::new(),
        base_url: "http://localhost:11434".into(),
        model: "llama3.3:70b".into(),
        ..Default::default()
    };
    assert_eq!(
        cfg.effective_provider_type(),
        ProviderType::Ollama,
        "explicit/auto provider type must resolve to Ollama"
    );
    let errors = cfg.validate();
    assert!(
        errors.is_empty(),
        "Ollama no-auth config must validate without errors: {:?}",
        errors
    );
}

/// #2825 (combo 3): the MCP client config and the shared capability registry
/// share one config file (`.vaultpilot/capabilities.yaml`). Server + skill + tool
/// registrations and agent bindings must survive a save/load round-trip, which is
/// exactly what would break if the two modules disagreed on the schema.
#[test]
fn regression_2825_mcp_config_capability_registry_shared() {
    let dir = std::env::temp_dir().join(format!("vp_regr_2825_mcp_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    {
        let mut reg = CapabilityRegistry::default();
        reg.add_server(
            "mcp-fs",
            "Filesystem MCP",
            "Local filesystem tools",
            McpTransport::Stdio {
                command: "npx".into(),
                args: vec![
                    "-y".into(),
                    "@modelcontextprotocol/server-filesystem".into(),
                ],
                env: Default::default(),
            },
            Some(AuthConfig::BearerToken {
                token: "test-token".into(),
            }),
        );
        reg.add_skill(
            "sk-summary",
            "Summarizer",
            "summarize notes",
            "be concise",
            vec![],
        );
        reg.add_tool(
            "tool-search",
            "Vault Search",
            "search the vault",
            "vault_search",
        );

        assert!(
            reg.enable_for_agent("mcp-fs", "agent-default"),
            "enabling a registered capability must succeed"
        );

        reg.save(&dir).expect("save must succeed");
        let loaded = CapabilityRegistry::load(&dir).expect("load must succeed");

        assert!(
            loaded.list_ids().contains(&"mcp-fs"),
            "MCP server must persist"
        );
        assert!(
            loaded.list_ids().contains(&"sk-summary"),
            "skill must persist"
        );
        assert!(
            loaded.list_ids().contains(&"tool-search"),
            "tool must persist"
        );
        assert_eq!(
            loaded.enabled_for("agent-default").len(),
            1,
            "agent binding must persist across round-trip"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

/// #2825 (combo 4): PDF extraction → vault_query pipeline must be *closed* — the
/// output of `PdfParser` must always be a usable `ParsedFile` that can be turned
/// into a queryable `Record` without panicking, even when the PDF can't actually
/// be decoded (graceful stub fallback). This is the integration boundary the
/// report called out as untested.
#[test]
fn regression_2825_pdf_extraction_feeds_vault_query() {
    let dir = std::env::temp_dir().join(format!("vp_regr_2825_pdf_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("doc.pdf");
    // Deliberately NOT a valid PDF: exercises the graceful stub fallback path.
    fs::write(&path, b"this is not a real pdf, just a .pdf extension").unwrap();

    // Extraction must never panic and must always yield a ParsedFile.
    let parsed = PdfParser
        .parse(Path::new(&path))
        .expect("PdfParser must not panic on un-decodable input");

    // The extracted (possibly empty) text flows downstream into a vault_query
    // Record and is queryable — proving the pipeline is closed end-to-end.
    let record = Record::new(path.to_string_lossy().to_string())
        .with_prop("content", QValue::Text(parsed.text.clone()));

    let query = parse_query("SELECT *").expect("structured query must parse");
    let rows = query_records(&[record], &query);
    assert_eq!(
        rows.len(),
        1,
        "PDF-derived record must be queryable through vault_query"
    );

    let _ = fs::remove_dir_all(&dir);
}
