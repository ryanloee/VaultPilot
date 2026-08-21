//! Project CRUD — isolated knowledge spaces with independent RAG contexts (#1927).
//!
//! Projects are stored under `.vaultpilot/projects/` as individual JSON files,
//! one file per project. Each project contains a name, description, and a list
//! of note identifiers (paths) that define its knowledge scope.
//!
//! Unlike collections (SQLite-based), projects are file-based so they can be
//! version-controlled, synced, and edited manually.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use super::settings::load_settings_with_context;
use super::StorageContext;

// ────────────────────────────────────────────────────────
// Data model
// ────────────────────────────────────────────────────────

/// A project definition — an isolated knowledge space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Unique identifier (UUID v4)
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Optional description
    #[serde(default)]
    pub description: String,
    /// Note identifiers (paths or IDs) included in this project
    #[serde(default)]
    pub note_ids: Vec<String>,
    /// Glob / wildcard patterns for including notes (e.g. "projects/**/*.md")
    #[serde(default)]
    pub glob_patterns: Vec<String>,
    /// When the project was created
    pub created_at: String,
    /// When the project was last modified
    pub updated_at: String,
}

// ────────────────────────────────────────────────────────
// Path helpers
// ────────────────────────────────────────────────────────

/// Resolve the `.vaultpilot/projects/` directory from settings.
fn projects_dir(context: &StorageContext) -> Result<PathBuf> {
    let settings = load_settings_with_context(context)?;
    let vault_dir = Path::new(&settings.vault_dir);
    let dir = vault_dir.join(".vaultpilot").join("projects");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create projects directory: {}", dir.display()))?;
    Ok(dir)
}

/// Project file path for a given project ID.
fn project_path(dir: &Path, id: &str) -> PathBuf {
    // Sanitize: only alphanumeric + hyphens allowed in filename
    let safe: String = id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect();
    dir.join(format!("{}.json", safe))
}

// ────────────────────────────────────────────────────────
// Project CRUD
// ────────────────────────────────────────────────────────

/// Create a new project. Returns the created project.
#[instrument(skip(context))]
pub fn create_project_with_context(
    context: &StorageContext,
    name: &str,
    description: &str,
) -> Result<Project> {
    let dir = projects_dir(context)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();

    let project = Project {
        id: id.clone(),
        name: name.to_string(),
        description: description.to_string(),
        note_ids: Vec::new(),
        glob_patterns: Vec::new(),
        created_at: now.clone(),
        updated_at: now,
    };

    let path = project_path(&dir, &id);
    let json = serde_json::to_string_pretty(&project).context("failed to serialize project")?;
    // Write atomically via temp file + rename
    let tmp = dir.join(format!("{}.tmp", id));
    fs::write(&tmp, &json)
        .with_context(|| format!("failed to write project file: {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| {
        format!(
            "failed to rename project file: {} -> {}",
            tmp.display(),
            path.display()
        )
    })?;

    Ok(project)
}

/// Delete a project by its ID. Returns true if deleted.
#[instrument(skip(context))]
pub fn delete_project_with_context(context: &StorageContext, project_id: &str) -> Result<bool> {
    let dir = projects_dir(context)?;
    let path = project_path(&dir, project_id);
    if path.exists() {
        fs::remove_file(&path)
            .with_context(|| format!("failed to delete project file: {}", path.display()))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Add a note (by path or ID) to a project's scope.
/// Returns the updated project, or `None` if the project does not exist.
#[instrument(skip(context))]
pub fn add_note_to_project_with_context(
    context: &StorageContext,
    project_id: &str,
    note_id: &str,
) -> Result<Option<Project>> {
    let dir = projects_dir(context)?;
    let path = project_path(&dir, project_id);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read project file: {}", path.display()))?;
    let mut project: Project = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse project file: {}", path.display()))?;

    if !project.note_ids.iter().any(|n| n == note_id) {
        project.note_ids.push(note_id.to_string());
        project.updated_at = Utc::now().to_rfc3339();
        let json = serde_json::to_string_pretty(&project)
            .context("failed to serialize updated project")?;
        super::atomic_write(&path, json.as_bytes())?;
    }

    Ok(Some(project))
}

/// Remove a note (by path or ID) from a project's scope.
/// Returns the updated project, or `None` if the project does not exist.
#[instrument(skip(context))]
pub fn remove_note_from_project_with_context(
    context: &StorageContext,
    project_id: &str,
    note_id: &str,
) -> Result<Option<Project>> {
    let dir = projects_dir(context)?;
    let path = project_path(&dir, project_id);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read project file: {}", path.display()))?;
    let mut project: Project = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse project file: {}", path.display()))?;

    let before = project.note_ids.len();
    project.note_ids.retain(|n| n != note_id);
    if project.note_ids.len() != before {
        project.updated_at = Utc::now().to_rfc3339();
        let json = serde_json::to_string_pretty(&project)
            .context("failed to serialize updated project")?;
        super::atomic_write(&path, json.as_bytes())?;
    }

    Ok(Some(project))
}

/// List all projects with note counts.
#[instrument(skip(context))]
pub fn list_projects_with_context(context: &StorageContext) -> Result<Vec<Project>> {
    let dir = projects_dir(context)?;
    let mut projects = Vec::new();

    if !dir.exists() {
        return Ok(projects);
    }

    let entries = fs::read_dir(&dir)
        .with_context(|| format!("failed to read projects directory: {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension() != Some(std::ffi::OsStr::new("json")) {
            continue;
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read project file: {}", path.display()))?;
        match serde_json::from_str::<Project>(&content) {
            Ok(project) => projects.push(project),
            Err(e) => {
                tracing::warn!("skipping invalid project file {}: {}", path.display(), e);
            }
        }
    }

    // Sort by name for consistent output
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(projects)
}

/// Get a single project by ID.
#[instrument(skip(context))]
pub fn get_project_with_context(
    context: &StorageContext,
    project_id: &str,
) -> Result<Option<Project>> {
    let dir = projects_dir(context)?;
    let path = project_path(&dir, project_id);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read project file: {}", path.display()))?;
    let project = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse project file: {}", path.display()))?;
    Ok(Some(project))
}

/// Update a project's metadata (name, description). Returns the updated project.
#[instrument(skip(context))]
pub fn update_project_with_context(
    context: &StorageContext,
    project_id: &str,
    name: &str,
    description: &str,
) -> Result<Option<Project>> {
    let dir = projects_dir(context)?;
    let path = project_path(&dir, project_id);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read project file: {}", path.display()))?;
    let mut project: Project = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse project file: {}", path.display()))?;

    project.name = name.to_string();
    project.description = description.to_string();
    project.updated_at = Utc::now().to_rfc3339();

    let json =
        serde_json::to_string_pretty(&project).context("failed to serialize updated project")?;
    // Use atomic_write which handles temp file naming with UUID suffixes and
    // proper permissions, avoiding unsanitized project_id in temp path (#2615)
    super::atomic_write(&path, json.as_bytes())?;

    Ok(Some(project))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageContext;
    use chrono::Utc;

    fn setup_temp_context() -> StorageContext {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-projects-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        crate::storage::initialize_storage_with_context(&ctx).unwrap();
        ctx
    }

    #[test]
    fn test_create_and_list_project() {
        let ctx = setup_temp_context();
        let project = create_project_with_context(&ctx, "Test Project", "A test project")
            .expect("create should succeed");
        assert_eq!(project.name, "Test Project");
        assert_eq!(project.description, "A test project");
        assert!(project.note_ids.is_empty());

        let projects = list_projects_with_context(&ctx).expect("list should succeed");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Test Project");

        // Verify file exists on disk
        let dir = projects_dir(&ctx).unwrap();
        let path = project_path(&dir, &project.id);
        assert!(path.exists(), "project file should exist on disk");
    }

    #[test]
    fn test_delete_project() {
        let ctx = setup_temp_context();
        let project = create_project_with_context(&ctx, "Delete Me", "").unwrap();
        let deleted = delete_project_with_context(&ctx, &project.id).unwrap();
        assert!(deleted, "delete should return true");

        let projects = list_projects_with_context(&ctx).unwrap();
        assert_eq!(projects.len(), 0);
    }

    #[test]
    fn test_get_project() {
        let ctx = setup_temp_context();
        let created = create_project_with_context(&ctx, "Get Test", "desc").unwrap();
        let fetched = get_project_with_context(&ctx, &created.id)
            .expect("get should succeed")
            .expect("project should exist");
        assert_eq!(fetched.name, "Get Test");
        assert_eq!(fetched.description, "desc");
    }

    #[test]
    fn test_update_project() {
        let ctx = setup_temp_context();
        let created = create_project_with_context(&ctx, "Old Name", "old desc").unwrap();

        let updated = update_project_with_context(&ctx, &created.id, "New Name", "new desc")
            .expect("update should succeed")
            .expect("updated project should exist");
        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.description, "new desc");

        // Verify persisted
        let fetched = get_project_with_context(&ctx, &created.id)
            .unwrap()
            .unwrap();
        assert_eq!(fetched.name, "New Name");
    }

    #[test]
    fn test_get_nonexistent_project() {
        let ctx = setup_temp_context();
        let result = get_project_with_context(&ctx, "nonexistent-id").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_nonexistent_project() {
        let ctx = setup_temp_context();
        let result = delete_project_with_context(&ctx, "nonexistent-id").unwrap();
        assert!(!result, "deleting nonexistent project should return false");
    }

    #[test]
    fn test_empty_list_when_no_projects() {
        let ctx = setup_temp_context();
        let projects = list_projects_with_context(&ctx).unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn test_multiple_projects() {
        let ctx = setup_temp_context();
        create_project_with_context(&ctx, "Beta", "").unwrap();
        create_project_with_context(&ctx, "Alpha", "").unwrap();
        create_project_with_context(&ctx, "Charlie", "").unwrap();

        let projects = list_projects_with_context(&ctx).unwrap();
        assert_eq!(projects.len(), 3);
        // Should be sorted alphabetically
        assert_eq!(projects[0].name, "Alpha");
        assert_eq!(projects[1].name, "Beta");
        assert_eq!(projects[2].name, "Charlie");
    }

    #[test]
    fn test_add_note_to_project() {
        let ctx = setup_temp_context();
        let project = create_project_with_context(&ctx, "Notes Project", "").unwrap();
        assert!(project.note_ids.is_empty());

        // Add first note
        let updated =
            add_note_to_project_with_context(&ctx, &project.id, "notes/project-design.md")
                .unwrap()
                .expect("project should exist");
        assert_eq!(updated.note_ids.len(), 1);
        assert_eq!(updated.note_ids[0], "notes/project-design.md");

        // Add second note
        let updated = add_note_to_project_with_context(&ctx, &project.id, "notes/architecture.md")
            .unwrap()
            .expect("project should exist");
        assert_eq!(updated.note_ids.len(), 2);

        // Adding the same note again should be idempotent
        let updated =
            add_note_to_project_with_context(&ctx, &project.id, "notes/project-design.md")
                .unwrap()
                .expect("project should exist");
        assert_eq!(
            updated.note_ids.len(),
            2,
            "duplicate add should be idempotent"
        );
    }

    #[test]
    fn test_remove_note_from_project() {
        let ctx = setup_temp_context();
        let project = create_project_with_context(&ctx, "Remove Test", "").unwrap();
        add_note_to_project_with_context(&ctx, &project.id, "a.md").unwrap();
        add_note_to_project_with_context(&ctx, &project.id, "b.md").unwrap();
        add_note_to_project_with_context(&ctx, &project.id, "c.md").unwrap();

        let updated = remove_note_from_project_with_context(&ctx, &project.id, "b.md")
            .unwrap()
            .expect("project should exist");
        assert_eq!(updated.note_ids.len(), 2);
        assert!(!updated.note_ids.contains(&"b.md".to_string()));
        assert!(updated.note_ids.contains(&"a.md".to_string()));
        assert!(updated.note_ids.contains(&"c.md".to_string()));

        // Removing a non-existent note should be a no-op
        let updated = remove_note_from_project_with_context(&ctx, &project.id, "nonexistent.md")
            .unwrap()
            .expect("project should exist");
        assert_eq!(updated.note_ids.len(), 2);
    }

    #[test]
    fn test_add_note_nonexistent_project() {
        let ctx = setup_temp_context();
        let result = add_note_to_project_with_context(&ctx, "fake-id", "note.md").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_remove_note_nonexistent_project() {
        let ctx = setup_temp_context();
        let result = remove_note_from_project_with_context(&ctx, "fake-id", "note.md").unwrap();
        assert!(result.is_none());
    }
}
