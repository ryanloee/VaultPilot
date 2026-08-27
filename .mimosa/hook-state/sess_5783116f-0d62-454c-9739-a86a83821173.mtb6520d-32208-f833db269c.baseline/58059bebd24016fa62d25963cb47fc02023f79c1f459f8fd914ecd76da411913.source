//! Regression test for issue #1594: `vaultpilot config show` / `config edit`
//! CLI subcommand — surfaces vault-facing configuration (vault root, settings
//! file, `.vaultpilot/{sessions,prompts,projects}/` directories, prompt &
//! project listings) so users can find and edit the files that hold their
//! data.
//!
//! Bug:        No CLI command exposed the vault-facing configuration surface.
//!             Users had no way to discover where chat sessions, prompts, or
//!             projects were materialised on disk, undermining the "user data
//!             sovereignty" goal (#1594).
//! Root cause: Missing `Config` subcommand — the underlying modules
//!             (`prompt_store`, `storage::projects`, `storage::session_export`)
//!             already implemented the file-based storage; only the CLI
//!             surface was absent.
//! Fix:        Added `vaultpilot config show` / `config edit` to
//!             `src/bin/vaultpilot-cli/main.rs`.
//!
//! These tests pin the behaviour of the public library APIs that
//! `handle_config` composes, so a future refactor cannot silently break the
//! command's output without a failing test.

#[cfg(test)]
mod tests {
    use crate::prompt_store;
    use crate::storage::pool::StorageContext;
    use crate::storage::{list_projects_with_context, load_settings_with_context};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(label: &str) -> (StorageContext, PathBuf) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-reg1594-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        // for_test() seeds default_vault_dir = temp/vault — materialise it so
        // prompt_store / projects helpers can build sub-directories inside.
        let vault_dir = temp.join("vault");
        fs::create_dir_all(&vault_dir).expect("vault dir");
        (ctx, vault_dir)
    }

    /// `prompts_dir` must point at `<vault>/.vaultpilot/prompts` so that
    /// `config show` reports a path the user can actually open.
    #[test]
    fn regression_1594_prompts_dir_path_is_vault_scoped() {
        let (_ctx, vault_dir) = temp_context("pdir");
        let dir = prompt_store::prompts_dir(&vault_dir);
        assert_eq!(
            dir,
            vault_dir.join(".vaultpilot").join("prompts"),
            "prompts_dir must live under <vault>/.vaultpilot/prompts"
        );
    }

    /// `list_prompts` returns an empty list (not an error) on a fresh vault
    /// where the prompts directory does not yet exist. This is the contract
    /// `config show` relies on to avoid spurious failures on first run.
    #[test]
    fn regression_1594_list_prompts_empty_when_dir_absent() {
        let (_ctx, vault_dir) = temp_context("pempty");
        // Intentionally do NOT create .vaultpilot/prompts/.
        let entries = prompt_store::list_prompts(&vault_dir).expect("list_prompts must succeed");
        assert!(entries.is_empty(), "fresh vault must report zero prompts");
    }

    /// `list_prompts` parses user-authored prompt files and surfaces their
    /// metadata. `config show` uses this to populate its `prompts` array, so
    /// the frontmatter fields (name, description, model) must round-trip.
    #[test]
    fn regression_1594_list_prompts_parses_user_files() {
        let (_ctx, vault_dir) = temp_context("pfiles");
        let dir = prompt_store::ensure_prompts_dir(&vault_dir).expect("ensure prompts dir");
        let body = "---\n\
                    name: research\n\
                    description: 深度研究模式\n\
                    model: claude-sonnet-4-20250514\n\
                    ---\n\
                    You are a thorough research assistant.\n";
        fs::write(dir.join("research.md"), body).expect("write prompt file");

        let entries = prompt_store::list_prompts(&vault_dir).expect("list_prompts");
        assert_eq!(entries.len(), 1);
        let p = &entries[0];
        assert_eq!(p.name, "research");
        assert_eq!(p.description, "深度研究模式");
        assert_eq!(p.model, "claude-sonnet-4-20250514");
    }

    /// `list_projects_with_context` returns an empty vector on a fresh vault.
    /// `config show` uses this to populate `projects` / `projects_count`.
    #[test]
    fn regression_1594_list_projects_empty_on_fresh_vault() {
        let (ctx, _vault_dir) = temp_context("projempty");
        let projects =
            list_projects_with_context(&ctx).expect("list_projects_with_context must succeed");
        assert!(projects.is_empty(), "fresh vault must report zero projects");
    }

    /// The settings file path is the one `config edit` opens in `$EDITOR`.
    /// The on-disk file does not need to pre-exist for `settings_path()` to
    /// return a stable, vault-scoped path — verify the contract holds.
    #[test]
    fn regression_1594_settings_path_is_stable() {
        let (ctx, _vault_dir) = temp_context("setpath");
        let path = ctx.settings_path();
        // The path itself is constructed by for_test() as temp/settings.json;
        // we only need to assert it is non-empty and ends with settings.json.
        assert!(path.to_string_lossy().ends_with("settings.json"));
    }

    /// `load_settings_with_context` must succeed on a fresh context (it seeds
    /// defaults when settings.json is absent) so that `config show` never
    /// crashes on first run. Additionally, the `session_export_enabled` flag
    /// must default to `false` (per #1594 opt-in contract).
    #[test]
    fn regression_1594_settings_load_with_safe_defaults() {
        let (ctx, _vault_dir) = temp_context("defaults");
        let settings =
            load_settings_with_context(&ctx).expect("settings must load with seeded defaults");
        assert!(
            !settings.session_export_enabled,
            "session_export_enabled must default to false (opt-in, #1594)"
        );
    }
}
