//! Regression tests for VaultPilot bugs.
//!
//! Each module is named after the issue it covers:
//!   - `issue_042_empty_vault` — regression for issue #42
//!   - `issue_099_search_crash` — regression for issue #99
//!
//! See `docs/TESTING.md` for the template and conventions.
//!
//! ### Adding a new regression test:
//! 1. Copy `src/regression/_TEMPLATE.rs` → `src/regression/issue_NNN_desc.rs`
//! 2. Add `mod issue_NNN_desc;` below (keep alphabetical by issue number)
//! 3. Fill in the test details
//! 4. Run: `cargo test regression_NNN`

// ┌────────────────────────────────────────────────────────────────────┐
// │ Add new regression test modules here. Keep alphabetical by issue. │
// └────────────────────────────────────────────────────────────────────┘
//
// mod issue_042_empty_vault;
// mod issue_099_search_crash;
mod issue_1175_missing_test_fields;
mod issue_1326_agent_glob_utf8;
mod issue_1328_is_cjk_ranges;
mod issue_1342_agent_loop;
mod issue_1354_agent_expect;
mod issue_1358_write_approval;
mod issue_1359_agent_integration;
mod issue_1594_config_command;
mod issue_1887_vault_encryption;
mod issue_1912_fsrs_spaced_repetition;
mod issue_1987_file_parsing;
mod issue_1994_calendar;
mod issue_1995_context_surface;
mod issue_1996_agent_engine;
mod issue_2746_drain_truncation;
mod issue_2804_diff_cli;
mod issue_2813_query_export;
mod issue_2825_integration_combos;
mod issue_2826_api_key_secure_storage;
mod issue_2832_graph_unlinked_mentions;
mod issue_2859_mirror;
mod issue_2884_mirror_state;
mod issue_2887_mirror_relative_path;
mod issue_2889_mirror_orphan;
mod issue_2913_deterministic_columns;
mod issue_2921_formulas;
mod issue_2924_mirror_backflow;
mod issue_2936_attachment_cleanup;
mod issue_2946_user_skills;
mod issue_2953_single_backflow_entry;
mod issue_3077_feed_dedup;
mod issue_3078_changelog;
mod issue_3083_changelog_limit;
mod issue_3084_feed_dedup_preserve_created_at;
mod issue_3103_agent_health_detection;
mod issue_3104_notes_batch_operations;
mod issue_3159_os_keychain;
mod issue_3170_keychain_clear;
mod issue_3189_web_clipper;
mod issue_3251_kanban_trim_keys;
mod issue_3258_kanban_multi_value_group;
mod issue_3270_synthesize_notes;
mod issue_3271_suggest_links;
mod issue_3276_export_documents;
mod issue_3320_tag_merge;
mod issue_3342_callouts;
mod issue_3343_bases_table;
mod issue_3350_savenote_created_at;
mod issue_3359_undo_redo;
mod issue_914_related_notes;
