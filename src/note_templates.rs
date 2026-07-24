//! Note template system — create notes from user-defined templates (#3383).
//!
//! Templates are Markdown files stored in `.vaultpilot/templates/*.md`.
//! Each template may include optional YAML frontmatter to declare a
//! description and required variables. The body uses the existing
//! [`template`] engine syntax (`{{ var }}`, `{% if %}`, etc.).
//!
//! Built-in variables:
//! - `{{title}}` — note title
//! - `{{date}}` — current date (YYYY-MM-DD)
//! - `{{time}}` — current time (HH:MM)
//! - `{{datetime}}` — ISO-8601 timestamp
//! - `{{tags}}` — comma-separated tags
//! - Custom variables supplied by the user at creation time

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::template::{self, Value};

/// A note template loaded from `.vaultpilot/templates/`.
#[derive(Debug, Clone)]
pub struct NoteTemplate {
    /// Template name (file stem, e.g. `meeting` from `meeting.md`).
    pub name: String,
    /// Human-readable description from frontmatter.
    pub description: String,
    /// Variable names declared in frontmatter that the user should fill.
    pub variables: Vec<String>,
    /// Raw template body (Markdown with `{{ }}` placeholders).
    pub body: String,
}

/// Directory where user templates live.
pub fn templates_dir(vault_dir: &Path) -> PathBuf {
    vault_dir.join(".vaultpilot/templates")
}

/// List all available template names in the vault.
///
/// Returns a sorted list of template names (file stems of `.md` files
/// in `.vaultpilot/templates/`).
pub fn list_template_names(vault_dir: &Path) -> Vec<String> {
    let dir = templates_dir(vault_dir);
    let mut names = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }

    names.sort();
    names.dedup();
    names
}

/// Load a single template by name from the vault.
pub fn load_template(vault_dir: &Path, name: &str) -> Result<NoteTemplate, String> {
    let path = templates_dir(vault_dir).join(format!("{name}.md"));
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("template '{name}' not found at {}: {e}", path.display()))?;

    Ok(parse_template(name, &content))
}

/// Parse raw template file content into a [`NoteTemplate`].
///
/// Supports optional YAML frontmatter delimited by `---`. If present,
/// extracts `description` and `variables` fields. The rest is the
/// template body.
fn parse_template(name: &str, content: &str) -> NoteTemplate {
    let (frontmatter, body) = split_frontmatter(content);

    let mut description = String::new();
    let mut variables = Vec::new();

    if let Some(fm) = frontmatter {
        // Minimal YAML parsing — we only need simple key: value pairs.
        for line in fm.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("description:") {
                description = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(rest) = line.strip_prefix("variables:") {
                // variables: [a, b, c]  or  variables: a, b, c
                let rest = rest.trim();
                if let Some(arr) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                    variables = arr
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                } else if !rest.is_empty() {
                    variables = rest
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }
    }

    NoteTemplate {
        name: name.to_string(),
        description,
        variables,
        body,
    }
}

/// Split content into optional frontmatter and body.
fn split_frontmatter(content: &str) -> (Option<String>, String) {
    let trimmed = content.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
    {
        if let Some(end) = rest.find("\n---\n").or_else(|| rest.find("\n---\r\n")) {
            let frontmatter = rest[..end].to_string();
            let body_start = end + "\n---\n".len();
            let body = if body_start < rest.len() {
                rest[body_start..].trim_start().to_string()
            } else {
                String::new()
            };
            return (Some(frontmatter), body);
        }
        // Handle case where frontmatter is at the very end of file (no trailing newline)
        if let Some(end) = rest.find("\n---") {
            let frontmatter = rest[..end].to_string();
            let body = rest[end + "\n---".len()..].trim_start().to_string();
            return (Some(frontmatter), body);
        }
    }
    (None, content.to_string())
}

/// Build a template context with built-in note variables plus user-supplied
/// custom variables.
pub fn build_context(
    title: &str,
    tags: &[String],
    custom_vars: &HashMap<String, String>,
) -> template::Context {
    let mut ctx = template::Context::new();

    ctx.insert("title".to_string(), Value::Str(title.to_string()));
    ctx.insert(
        "date".to_string(),
        Value::Str(chrono::Local::now().format("%Y-%m-%d").to_string()),
    );
    ctx.insert(
        "time".to_string(),
        Value::Str(chrono::Local::now().format("%H:%M").to_string()),
    );
    ctx.insert(
        "datetime".to_string(),
        Value::Str(chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()),
    );
    ctx.insert("tags".to_string(), Value::Str(tags.join(", ")));

    for (k, v) in custom_vars {
        ctx.insert(k.clone(), Value::Str(v.clone()));
    }

    ctx
}

/// Render a template with the given context.
pub fn render_template(template: &NoteTemplate, ctx: &template::Context) -> String {
    template::render(&template.body, ctx)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_template_no_frontmatter() {
        let t = parse_template("simple", "Hello {{title}}!");
        assert_eq!(t.name, "simple");
        assert!(t.description.is_empty());
        assert!(t.variables.is_empty());
        assert_eq!(t.body, "Hello {{title}}!");
    }

    #[test]
    fn test_parse_template_with_frontmatter() {
        let content = "---\ndescription: A meeting note\ntitle: Meeting\nvariables: [project, attendees]\n---\n# {{title}}\n\nProject: {{project}}";
        let t = parse_template("meeting", content);
        assert_eq!(t.description, "A meeting note");
        assert_eq!(t.variables, vec!["project", "attendees"]);
        assert!(t.body.starts_with("# {{title}}"));
    }

    #[test]
    fn test_parse_template_variables_comma_format() {
        let content = "---\nvariables: a, b, c\n---\nBody";
        let t = parse_template("test", content);
        assert_eq!(t.variables, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_split_frontmatter_none() {
        let (fm, body) = split_frontmatter("Just text");
        assert!(fm.is_none());
        assert_eq!(body, "Just text");
    }

    #[test]
    fn test_split_frontmatter_present() {
        let (fm, body) = split_frontmatter("---\nkey: value\n---\nBody here");
        assert!(fm.is_some());
        assert!(fm.unwrap().contains("key: value"));
        assert_eq!(body, "Body here");
    }

    #[test]
    fn test_build_context_builtins() {
        let ctx = build_context("My Note", &["tag1".to_string()], &HashMap::new());
        assert_eq!(ctx.get("title"), Some(&Value::Str("My Note".to_string())));
        assert_eq!(ctx.get("tags"), Some(&Value::Str("tag1".to_string())));
        assert!(ctx.contains_key("date"));
        assert!(ctx.contains_key("time"));
        assert!(ctx.contains_key("datetime"));
    }

    #[test]
    fn test_build_context_custom_vars() {
        let mut custom = HashMap::new();
        custom.insert("project".to_string(), "VaultPilot".to_string());
        let ctx = build_context("Test", &[], &custom);
        assert_eq!(
            ctx.get("project"),
            Some(&Value::Str("VaultPilot".to_string()))
        );
    }

    #[test]
    fn test_render_template_basic() {
        let t = NoteTemplate {
            name: "test".to_string(),
            description: String::new(),
            variables: Vec::new(),
            body: "# {{title}}\n\nTags: {{tags}}".to_string(),
        };
        let ctx = build_context(
            "Hello World",
            &["a".to_string(), "b".to_string()],
            &HashMap::new(),
        );
        let result = render_template(&t, &ctx);
        assert!(result.contains("# Hello World"));
        assert!(result.contains("Tags: a, b"));
    }

    #[test]
    fn test_render_template_with_custom_var() {
        let t = NoteTemplate {
            name: "test".to_string(),
            description: String::new(),
            variables: vec!["project".to_string()],
            body: "Project: {{project}}".to_string(),
        };
        let mut custom = HashMap::new();
        custom.insert("project".to_string(), "Apollo".to_string());
        let ctx = build_context("Note", &[], &custom);
        let result = render_template(&t, &ctx);
        assert_eq!(result, "Project: Apollo");
    }

    #[test]
    fn test_render_template_with_fallback() {
        let t = NoteTemplate {
            name: "test".to_string(),
            description: String::new(),
            variables: vec![],
            body: "Value: {{missing ?? \"default\"}}".to_string(),
        };
        let ctx = build_context("Note", &[], &HashMap::new());
        let result = render_template(&t, &ctx);
        assert_eq!(result, "Value: default");
    }

    #[test]
    fn test_render_template_conditional() {
        let t = NoteTemplate {
            name: "test".to_string(),
            description: String::new(),
            variables: vec!["status".to_string()],
            body: "{% if status %}Status: {{status}}{% else %}No status{% endif %}".to_string(),
        };
        let mut custom = HashMap::new();
        custom.insert("status".to_string(), "done".to_string());
        let ctx = build_context("Note", &[], &custom);
        let result = render_template(&t, &ctx);
        assert_eq!(result, "Status: done");
    }

    #[test]
    fn test_list_template_names_empty_dir() {
        let tmp = std::env::temp_dir().join("vp-template-test-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(list_template_names(&tmp).is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_list_template_names_with_files() {
        let tmp =
            std::env::temp_dir().join(format!("vp-template-test-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let templates = tmp.join(".vaultpilot/templates");
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(templates.join("meeting.md"), "# Meeting").unwrap();
        std::fs::write(templates.join("standup.md"), "# Standup").unwrap();
        std::fs::write(templates.join("readme.txt"), "not a template").unwrap();

        let names = list_template_names(&tmp);
        assert_eq!(names, vec!["meeting", "standup"]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_template_from_disk() {
        let tmp =
            std::env::temp_dir().join(format!("vp-template-test-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let templates = tmp.join(".vaultpilot/templates");
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("project.md"),
            "---\ndescription: Project kickoff\nvariables: [team]\n---\n# {{title}}\nTeam: {{team}}",
        )
        .unwrap();

        let t = load_template(&tmp, "project").unwrap();
        assert_eq!(t.name, "project");
        assert_eq!(t.description, "Project kickoff");
        assert_eq!(t.variables, vec!["team"]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_template_not_found() {
        let tmp = std::env::temp_dir().join("vp-template-test-missing");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let result = load_template(&tmp, "nonexistent");
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Regression test for #3383: full end-to-end flow — load template,
    /// build context, render, and verify output.
    #[test]
    fn test_end_to_end_template_creation() {
        let tmp = std::env::temp_dir().join(format!("vp-template-test-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let templates = tmp.join(".vaultpilot/templates");
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("bug_report.md"),
            "---\ndescription: Bug report template\nvariables: [severity, component]\n---\n# {{title}}\n\
             **Severity:** {{severity}}\n**Component:** {{component}}\n**Date:** {{date}}\n\n## Description\n\n",
        )
        .unwrap();

        let template = load_template(&tmp, "bug_report").unwrap();
        let mut custom = HashMap::new();
        custom.insert("severity".to_string(), "High".to_string());
        custom.insert("component".to_string(), "search".to_string());
        let ctx = build_context(
            "Search returns wrong results",
            &["bug".to_string()],
            &custom,
        );
        let rendered = render_template(&template, &ctx);

        assert!(rendered.contains("# Search returns wrong results"));
        assert!(rendered.contains("**Severity:** High"));
        assert!(rendered.contains("**Component:** search"));
        assert!(rendered.contains("**Date:**"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
