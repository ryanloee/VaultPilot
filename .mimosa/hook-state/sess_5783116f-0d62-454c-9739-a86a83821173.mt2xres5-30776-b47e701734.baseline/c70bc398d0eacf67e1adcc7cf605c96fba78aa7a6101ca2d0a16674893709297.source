//! Note templates stored as `.md` files with optional YAML frontmatter (#3383).
//!
//! Each template lives at `<vault>/.vaultpilot/templates/<name>.md` and may
//! declare variables in its frontmatter.  When a note is created from a
//! template the [`crate::template`] engine renders the body with a context
//! populated from built-in variables (title, date, time, tags, …) plus any
//! user-supplied values.
//!
//! # File format
//! ```markdown
//! ---
//! description: Meeting notes template
//! variables:
//!   - attendees
//!   - agenda
//! ---
//! # {{title}}
//!
//! **Date:** {{date}}
//! **Attendees:** {{attendees}}
//!
//! ## Agenda
//! {{agenda}}
//! ```
//!
//! Templates without frontmatter are also valid — the entire file is the
//! template body.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::template::{render, Context as TplContext, Value as TplValue};

/// Metadata for a note template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateEntry {
    /// Display name (derived from the filename stem).
    pub name: String,
    /// Short description shown in listings.
    #[serde(default)]
    pub description: String,
    /// Variable names that the template expects the user to supply.
    #[serde(default)]
    pub variables: Vec<String>,
    /// The full template body (after YAML frontmatter, if any).
    #[serde(skip)]
    pub content: String,
}

/// Return the path to the templates directory under a vault.
pub fn templates_dir(vault_dir: &Path) -> PathBuf {
    vault_dir.join(".vaultpilot").join("templates")
}

/// Ensure the templates directory exists.
pub fn ensure_templates_dir(vault_dir: &Path) -> Result<PathBuf> {
    let dir = templates_dir(vault_dir);
    fs::create_dir_all(&dir).with_context(|| format!("create templates dir: {}", dir.display()))?;
    Ok(dir)
}

/// List all template files in a vault, returning parsed entries.
pub fn list_templates(vault_dir: &Path) -> Result<Vec<TemplateEntry>> {
    let dir = templates_dir(vault_dir);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut failed = Vec::new();

    for entry in fs::read_dir(&dir).context("read templates directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match parse_template_file(&path) {
            Ok(parsed) => entries.push(parsed),
            Err(e) => {
                failed.push(format!("{}: {e}", path.display()));
            }
        }
    }

    if !failed.is_empty() {
        eprintln!(
            "Warning: failed to parse templates:\n  {}",
            failed.join("\n  ")
        );
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Get a single template by name.
pub fn get_template(vault_dir: &Path, name: &str) -> Result<Option<TemplateEntry>> {
    let path = templates_dir(vault_dir).join(format!("{name}.md"));
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(parse_template_file(&path)?))
}

/// Save (create or overwrite) a template file.
pub fn save_template(vault_dir: &Path, entry: &TemplateEntry) -> Result<()> {
    let dir = ensure_templates_dir(vault_dir)?;
    let path = dir.join(format!("{}.md", entry.name));

    let frontmatter = serde_yaml_ng::to_string(&TemplateFrontmatter {
        description: if entry.description.is_empty() {
            None
        } else {
            Some(entry.description.clone())
        },
        variables: if entry.variables.is_empty() {
            None
        } else {
            Some(entry.variables.clone())
        },
    })
    .context("serialize template frontmatter")?;

    let content = if frontmatter.trim() == "{}" || frontmatter.trim().is_empty() {
        format!("{}\n", entry.content)
    } else {
        format!("---\n{frontmatter}---\n{}\n", entry.content)
    };
    fs::write(&path, &content).with_context(|| format!("write template: {}", path.display()))?;
    Ok(())
}

/// Delete a template file by name.
pub fn delete_template(vault_dir: &Path, name: &str) -> Result<bool> {
    let path = templates_dir(vault_dir).join(format!("{name}.md"));
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path).with_context(|| format!("delete template: {}", path.display()))?;
    Ok(true)
}

/// Build the template context for a new note, populating built-in variables
/// (title, date, time, weekday, date_display, now) and user-supplied values.
pub fn build_note_context(
    title: &str,
    tags: &[String],
    user_vars: &HashMap<String, String>,
) -> TplContext {
    use chrono::Local;

    let now = Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let time_str = now.format("%H:%M").to_string();
    let weekday = now.format("%A").to_string();
    let date_display = now.format("%B %-d, %Y").to_string();
    let now_iso = now.to_rfc3339();

    let mut ctx = TplContext::new();
    ctx.insert("title".into(), TplValue::Str(title.to_string()));
    ctx.insert("note_title".into(), TplValue::Str(title.to_string()));
    ctx.insert("date".into(), TplValue::Str(date_str));
    ctx.insert("time".into(), TplValue::Str(time_str));
    ctx.insert("weekday".into(), TplValue::Str(weekday));
    ctx.insert("date_display".into(), TplValue::Str(date_display));
    ctx.insert("now".into(), TplValue::Str(now_iso));
    ctx.insert("tags".into(), TplValue::Str(tags.join(", ")));

    for (k, v) in user_vars {
        ctx.insert(k.clone(), TplValue::Str(v.clone()));
    }

    ctx
}

/// Render a template body with the given context, returning the expanded
/// markdown content for a new note.
pub fn render_template(body: &str, ctx: &TplContext) -> String {
    render(body, ctx)
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// YAML frontmatter struct for serialization.
#[derive(Debug, Serialize, Deserialize)]
struct TemplateFrontmatter {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variables: Option<Vec<String>>,
}

/// Parse a single template `.md` file.
///
/// Unlike [`crate::prompt_store`], a template without frontmatter is still
/// valid — the entire file becomes the template body.
fn parse_template_file(path: &Path) -> Result<TemplateEntry> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read template file: {}", path.display()))?;

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let stripped = raw.trim_start();
    if !stripped.starts_with("---") {
        // No frontmatter — entire file is the body.
        return Ok(TemplateEntry {
            name,
            description: String::new(),
            variables: Vec::new(),
            content: raw,
        });
    }

    let after_first = &stripped[3..].trim_start();

    let end_marker = after_first
        .find("\n---\n")
        .map(|pos| pos + 1)
        .or_else(|| after_first.find("\n---").map(|pos| pos + 1))
        .unwrap_or(0);

    if end_marker == 0 {
        // Malformed frontmatter — treat entire file as body.
        return Ok(TemplateEntry {
            name,
            description: String::new(),
            variables: Vec::new(),
            content: raw,
        });
    }

    let yaml_str = &after_first[..end_marker];
    let content = after_first[end_marker + 4..].trim().to_string();

    let frontmatter: TemplateFrontmatter = serde_yaml_ng::from_str(yaml_str)
        .with_context(|| format!("parse YAML frontmatter in: {}", path.display()))?;

    Ok(TemplateEntry {
        name,
        description: frontmatter.description.unwrap_or_default(),
        variables: frontmatter.variables.unwrap_or_default(),
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_dir(label: &str) -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let d =
            std::env::temp_dir().join(format!("vp-template-{label}-{}-{}", std::process::id(), n));
        cleanup(&d);
        d
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_save_get_list_delete_template() {
        let dir = test_dir("crud");
        fs::create_dir_all(dir.join(".vaultpilot")).unwrap();

        let entry = TemplateEntry {
            name: "meeting".into(),
            description: "Meeting notes".into(),
            variables: vec!["attendees".into()],
            content: "# {{title}}\n\nDate: {{date}}\nAttendees: {{attendees}}".into(),
        };
        save_template(&dir, &entry).unwrap();

        // get
        let loaded = get_template(&dir, "meeting").unwrap().unwrap();
        assert_eq!(loaded.name, "meeting");
        assert_eq!(loaded.description, "Meeting notes");
        assert_eq!(loaded.variables, vec!["attendees".to_string()]);
        assert!(loaded.content.contains("{{title}}"));

        // list
        let list = list_templates(&dir).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "meeting");

        // delete
        assert!(delete_template(&dir, "meeting").unwrap());
        assert!(!delete_template(&dir, "meeting").unwrap()); // already gone
        assert!(get_template(&dir, "meeting").unwrap().is_none());

        cleanup(&dir);
    }

    #[test]
    fn test_template_without_frontmatter() {
        let dir = test_dir("no_fm");
        fs::create_dir_all(dir.join(".vaultpilot/templates")).unwrap();

        let path = dir.join(".vaultpilot/templates/plain.md");
        fs::write(&path, "# {{title}}\n\nJust a body.").unwrap();

        let entry = parse_template_file(&path).unwrap();
        assert_eq!(entry.name, "plain");
        assert!(entry.description.is_empty());
        assert!(entry.variables.is_empty());
        assert!(entry.content.contains("{{title}}"));

        cleanup(&dir);
    }

    #[test]
    fn test_build_context_and_render() {
        let mut user_vars = HashMap::new();
        user_vars.insert("attendees".into(), "Alice, Bob".into());

        let ctx = build_note_context("Sprint Review", &["meeting".into()], &user_vars);

        assert_eq!(ctx.get("title").unwrap().to_display(), "Sprint Review");
        assert_eq!(ctx.get("attendees").unwrap().to_display(), "Alice, Bob");
        assert!(!ctx.get("date").unwrap().to_display().is_empty());

        let body = "# {{title}}\nDate: {{date}}\nAttendees: {{attendees}}";
        let rendered = render_template(body, &ctx);
        assert!(rendered.contains("# Sprint Review"));
        assert!(rendered.contains("Attendees: Alice, Bob"));
    }

    #[test]
    fn test_render_with_filters() {
        let ctx = build_note_context("My Note", &[], &HashMap::new());

        let body = "{{title | upper}}";
        let rendered = render_template(body, &ctx);
        assert_eq!(rendered, "MY NOTE");
    }

    #[test]
    fn test_list_empty_when_no_dir() {
        let dir = test_dir("empty");
        let list = list_templates(&dir).unwrap();
        assert!(list.is_empty());
        cleanup(&dir);
    }

    // ── Regression: #3383 note template system ───────────────────────────────

    #[test]
    fn regression_3383_template_load_and_render() {
        let dir = test_dir("reg3383");
        fs::create_dir_all(dir.join(".vaultpilot/templates")).unwrap();

        // Write a meeting template with frontmatter
        let template_md = "---\n\
            description: Sprint planning template\n\
            variables:\n  - sprint_goal\n  - attendees\n\
            ---\n\
            # {{title}}\n\
            **Date:** {{date}}\n\
            **Sprint Goal:** {{sprint_goal}}\n\
            **Attendees:** {{attendees}}\n\n\
            ## Agenda\n\n- [ ] Review last sprint\n- [ ] Plan new sprint\n";
        let path = dir.join(".vaultpilot/templates/sprint-planning.md");
        fs::write(&path, template_md).unwrap();

        // Load
        let entry = get_template(&dir, "sprint-planning")
            .unwrap()
            .expect("template should exist");
        assert_eq!(entry.name, "sprint-planning");
        assert_eq!(entry.variables, vec!["sprint_goal", "attendees"]);

        // Build context with user-supplied variables
        let mut user_vars = HashMap::new();
        user_vars.insert("sprint_goal".into(), "Ship v1.0".into());
        user_vars.insert("attendees".into(), "Alice, Bob, Carol".into());

        let ctx = build_note_context("Sprint 42 Planning", &["meeting".into()], &user_vars);

        // Render
        let rendered = render_template(&entry.content, &ctx);
        assert!(rendered.contains("# Sprint 42 Planning"));
        assert!(rendered.contains("**Sprint Goal:** Ship v1.0"));
        assert!(rendered.contains("**Attendees:** Alice, Bob, Carol"));
        assert!(rendered.contains("## Agenda"));

        cleanup(&dir);
    }
}
