//! Saved Skills CRUD — user-defined reusable AI command library (#3068).
//!
//! Saved Skills let users store named, versionable multi-step prompts that can
//! be invoked from the chat input (e.g. `/skill-name`) or @mentioned. This is
//! distinct from `trigger_rules` (which fire automatically in the background):
//! a Saved Skill is summoned *actively* by the user in the foreground.
//!
//! Skill templates support `{{selection}}` and `{{note}}` placeholders that are
//! interpolated against the current editor selection / active note at call time.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use super::pool::open_connection;
use super::StorageContext;

// ────────────────────────────────────────────────────────
// Types
// ────────────────────────────────────────────────────────

/// A user-defined, reusable AI command.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SavedSkill {
    /// Unique identifier (UUID).
    pub id: String,
    /// Human-readable name shown in the Skills panel and used as the `/command`.
    pub name: String,
    /// Optional longer description.
    #[serde(default)]
    pub description: String,
    /// Prompt template body. May contain `{{selection}}` / `{{note}}` placeholders.
    pub prompt: String,
    /// Scope: empty (global) or a vault-relative directory path.
    #[serde(default)]
    pub scope: String,
    /// Whether the skill is enabled (selectable in the UI).
    #[serde(default)]
    pub enabled: bool,
    /// Creation timestamp (ISO-8601).
    pub created_at: String,
    /// Last modification timestamp (ISO-8601).
    pub updated_at: String,
}

/// Context values supplied when invoking a skill, used to fill placeholders.
#[derive(Debug, Clone, Default)]
pub struct SkillInvocation {
    /// Current editor selection (for `{{selection}}`).
    pub selection: String,
    /// Active note body / title (for `{{note}}`).
    pub note: String,
}

impl SavedSkill {
    /// Interpolate the prompt template against the given invocation context.
    ///
    /// Supports `{{selection}}` and `{{note}}` placeholders. Unknown placeholders
    /// are left untouched so user-authored templates remain robust.
    pub fn render(&self, invocation: &SkillInvocation) -> String {
        self.prompt
            .replace("{{selection}}", &invocation.selection)
            .replace("{{note}}", &invocation.note)
    }
}

// ────────────────────────────────────────────────────────
// CRUD
// ────────────────────────────────────────────────────────

/// Create a new saved skill. Returns the created skill.
#[instrument(skip(context))]
pub fn create_skill_with_context(
    context: &StorageContext,
    name: &str,
    description: &str,
    prompt: &str,
    scope: &str,
) -> Result<SavedSkill> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    let enabled: i32 = 1;

    connection
        .execute(
            "INSERT INTO skills (id, name, description, prompt, scope, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, name, description, prompt, scope, enabled, now, now],
        )
        .with_context(|| format!("failed to create skill '{name}'"))?;

    Ok(SavedSkill {
        id,
        name: name.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        scope: scope.to_string(),
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Get a single skill by ID.
#[instrument(skip(context))]
pub fn get_skill_with_context(context: &StorageContext, id: &str) -> Result<Option<SavedSkill>> {
    let (connection, _) = open_connection(context)?;
    connection
        .query_row(
            r#"SELECT id, name, description, prompt, scope, enabled, created_at, updated_at
               FROM skills WHERE id = ?1"#,
            params![id],
            |row| Ok(build_skill(row)),
        )
        .optional()
        .map_err(|e| anyhow::anyhow!("failed to query skill: {e}"))
}

/// List all skills, optionally filtered by enabled state.
#[instrument(skip(context))]
pub fn list_skills_with_context(
    context: &StorageContext,
    only_enabled: bool,
) -> Result<Vec<SavedSkill>> {
    let (connection, _) = open_connection(context)?;

    let sql = if only_enabled {
        r#"SELECT id, name, description, prompt, scope, enabled, created_at, updated_at
           FROM skills WHERE enabled = 1 ORDER BY name COLLATE NOCASE"#
    } else {
        r#"SELECT id, name, description, prompt, scope, enabled, created_at, updated_at
           FROM skills ORDER BY name COLLATE NOCASE"#
    };

    let mut stmt = connection.prepare(sql)?;
    let rows = stmt.query_map([], |row| Ok(build_skill(row)))?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| anyhow::anyhow!("failed to list skills: {e}"))
}

/// Update a skill's mutable fields by ID.
///
/// `None` arguments leave the corresponding field unchanged. Returns the
/// updated skill, or `None` if no row matched.
#[instrument(skip(context))]
pub fn update_skill_with_context(
    context: &StorageContext,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    prompt: Option<&str>,
    scope: Option<&str>,
) -> Result<Option<SavedSkill>> {
    let existing = get_skill_with_context(context, id)?;
    let Some(mut skill) = existing else {
        return Ok(None);
    };

    if let Some(v) = name {
        skill.name = v.to_string();
    }
    if let Some(v) = description {
        skill.description = v.to_string();
    }
    if let Some(v) = prompt {
        skill.prompt = v.to_string();
    }
    if let Some(v) = scope {
        skill.scope = v.to_string();
    }
    skill.updated_at = Utc::now().to_rfc3339();

    let (connection, _) = open_connection(context)?;
    connection
        .execute(
            "UPDATE skills SET name = ?1, description = ?2, prompt = ?3, scope = ?4, updated_at = ?5 WHERE id = ?6",
            params![
                skill.name,
                skill.description,
                skill.prompt,
                skill.scope,
                skill.updated_at,
                id
            ],
        )
        .with_context(|| format!("failed to update skill '{id}'"))?;

    Ok(Some(skill))
}

/// Delete a skill by ID. Returns `true` if a row was deleted.
#[instrument(skip(context))]
pub fn delete_skill_with_context(context: &StorageContext, id: &str) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let rows = connection
        .execute("DELETE FROM skills WHERE id = ?1", params![id])
        .with_context(|| format!("failed to delete skill '{id}'"))?;
    Ok(rows > 0)
}

/// Toggle a skill's enabled state. Returns the new enabled state, or `None` if
/// the skill does not exist.
#[instrument(skip(context))]
pub fn toggle_skill_with_context(context: &StorageContext, id: &str) -> Result<Option<bool>> {
    let (connection, _) = open_connection(context)?;

    let current: Option<i32> = connection
        .query_row(
            "SELECT enabled FROM skills WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;

    match current {
        None => Ok(None),
        Some(val) => {
            let new_enabled = if val == 0 { 1 } else { 0 };
            let now = Utc::now().to_rfc3339();
            connection.execute(
                "UPDATE skills SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
                params![new_enabled, now, id],
            )?;
            Ok(Some(new_enabled != 0))
        }
    }
}

// ────────────────────────────────────────────────────────
// Internal helpers
// ────────────────────────────────────────────────────────

/// Map a result row to a [`SavedSkill`].
fn build_skill(row: &rusqlite::Row<'_>) -> SavedSkill {
    SavedSkill {
        id: row.get(0).expect("skills.id"),
        name: row.get(1).expect("skills.name"),
        description: row.get(2).expect("skills.description"),
        prompt: row.get(3).expect("skills.prompt"),
        scope: row.get(4).expect("skills.scope"),
        enabled: row.get::<_, i32>(5).expect("skills.enabled") != 0,
        created_at: row.get(6).expect("skills.created_at"),
        updated_at: row.get(7).expect("skills.updated_at"),
    }
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn setup_context() -> (PathBuf, StorageContext) {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-skill-test-{}-{}",
            std::process::id(),
            counter
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        super::super::initialize_storage_with_context(&ctx).unwrap();
        (temp, ctx)
    }

    #[test]
    fn test_create_and_get_skill() {
        let (_tmp, ctx) = setup_context();
        let skill = create_skill_with_context(
            &ctx,
            "Summarize Meeting",
            "Condense a meeting transcript",
            "Summarize the following: {{selection}}",
            "",
        )
        .expect("create skill");

        assert!(!skill.id.is_empty());
        assert_eq!(skill.name, "Summarize Meeting");
        assert!(skill.enabled);

        let fetched = get_skill_with_context(&ctx, &skill.id).unwrap();
        assert!(fetched.is_some());
        assert_eq!(
            fetched.unwrap().prompt,
            "Summarize the following: {{selection}}"
        );
    }

    #[test]
    fn test_list_skills_orders_by_name() {
        let (_tmp, ctx) = setup_context();
        create_skill_with_context(&ctx, "Zebra", "", "z", "").unwrap();
        create_skill_with_context(&ctx, "alpha", "", "a", "").unwrap();
        create_skill_with_context(&ctx, "Mike", "", "m", "").unwrap();

        let all = list_skills_with_context(&ctx, false).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].name, "alpha");
        assert_eq!(all[1].name, "Mike");
        assert_eq!(all[2].name, "Zebra");
    }

    #[test]
    fn test_list_only_enabled() {
        let (_tmp, ctx) = setup_context();
        let a = create_skill_with_context(&ctx, "A", "", "a", "").unwrap();
        create_skill_with_context(&ctx, "B", "", "b", "").unwrap();

        toggle_skill_with_context(&ctx, &a.id).unwrap();
        let enabled = list_skills_with_context(&ctx, true).unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "B");
    }

    #[test]
    fn test_update_skill() {
        let (_tmp, ctx) = setup_context();
        let skill = create_skill_with_context(&ctx, "Old", "desc", "old prompt", "scope1").unwrap();

        let updated =
            update_skill_with_context(&ctx, &skill.id, Some("New"), None, Some("new prompt"), None)
                .unwrap()
                .expect("updated");

        assert_eq!(updated.name, "New");
        assert_eq!(updated.prompt, "new prompt");
        // untouched fields preserved
        assert_eq!(updated.description, "desc");
        assert_eq!(updated.scope, "scope1");
        assert_ne!(updated.updated_at, skill.updated_at);
    }

    #[test]
    fn test_update_nonexistent_skill() {
        let (_tmp, ctx) = setup_context();
        assert!(
            update_skill_with_context(&ctx, "nope", Some("x"), None, None, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_delete_skill() {
        let (_tmp, ctx) = setup_context();
        let skill = create_skill_with_context(&ctx, "Temp", "", "t", "").unwrap();
        assert!(delete_skill_with_context(&ctx, &skill.id).unwrap());
        assert!(get_skill_with_context(&ctx, &skill.id).unwrap().is_none());
    }

    #[test]
    fn test_toggle_skill() {
        let (_tmp, ctx) = setup_context();
        let skill = create_skill_with_context(&ctx, "Toggle", "", "t", "").unwrap();
        assert!(skill.enabled);

        let off = toggle_skill_with_context(&ctx, &skill.id).unwrap().unwrap();
        assert!(!off);
        let on = toggle_skill_with_context(&ctx, &skill.id).unwrap().unwrap();
        assert!(on);
    }

    #[test]
    fn test_toggle_nonexistent() {
        let (_tmp, ctx) = setup_context();
        assert!(toggle_skill_with_context(&ctx, "nope").unwrap().is_none());
    }

    #[test]
    fn test_render_placeholders() {
        let (_tmp, ctx) = setup_context();
        let skill = create_skill_with_context(
            &ctx,
            "Inject",
            "",
            "Note: {{note}}\nSelection: {{selection}}",
            "",
        )
        .unwrap();

        let rendered = skill.render(&SkillInvocation {
            selection: "hello world".to_string(),
            note: "# My Note".to_string(),
        });
        assert_eq!(rendered, "Note: # My Note\nSelection: hello world");
    }

    #[test]
    fn test_render_empty_invocation_leaves_blank() {
        let (_tmp, ctx) = setup_context();
        let skill =
            create_skill_with_context(&ctx, "Inject", "", "Sel: [{{selection}}]", "").unwrap();
        let rendered = skill.render(&SkillInvocation::default());
        assert_eq!(rendered, "Sel: []");
    }

    #[test]
    fn test_create_then_delete_roundtrip() {
        let (_tmp, ctx) = setup_context();
        let skill = create_skill_with_context(
            &ctx,
            "Summarize selection",
            "Wraps the current selection in a summarize prompt",
            "Summarize the following: {{selection}}",
            "",
        )
        .unwrap();
        assert!(!skill.id.is_empty());
        assert!(skill.enabled);

        // Deleting an existing skill reports true.
        let removed = delete_skill_with_context(&ctx, &skill.id).unwrap();
        assert!(removed);

        // After deletion the skill is gone.
        assert!(get_skill_with_context(&ctx, &skill.id).unwrap().is_none());

        // Deleting a non-existent skill reports false (CLI surfaces this as an error).
        let missing = delete_skill_with_context(&ctx, &skill.id).unwrap();
        assert!(!missing);
    }
}
