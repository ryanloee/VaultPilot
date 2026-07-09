//! System prompts stored as vault notes — plain `.md` files with YAML frontmatter.
//!
//! Each prompt is a markdown file under `<vault>/.vaultpilot/prompts/<name>.md`
//! with YAML frontmatter containing name, description, and optional model hint.
//!
//! # File format
//! ```markdown
//! ---
//! name: 研究助手
//! description: 深度研究模式，引用来源
//! model: claude-sonnet-4-20250514
//! ---
//! You are a thorough research assistant...
//! ```
//!
//! The `name` field must match the filename stem and must be unique.
//! When a prompt is set as "active" (via `AppSettings.active_prompt_name`),
//! its content is loaded at runtime and prepended to the AI system prompt
//! as custom instructions.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Metadata for a vault prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptEntry {
    /// Display name (must match filename stem).
    pub name: String,
    /// Short description shown in listings.
    #[serde(default)]
    pub description: String,
    /// Optional model hint (e.g. "claude-sonnet-4-20250514").
    /// When non-empty, the model is auto-switched when this prompt is active.
    #[serde(default)]
    pub model: String,
    /// The full prompt body (after YAML frontmatter).
    #[serde(skip)]
    pub content: String,
}

/// Return the path to the prompts directory under a vault.
pub fn prompts_dir(vault_dir: &Path) -> PathBuf {
    vault_dir.join(".vaultpilot").join("prompts")
}

/// Ensure the prompts directory exists.
pub fn ensure_prompts_dir(vault_dir: &Path) -> Result<PathBuf> {
    let dir = prompts_dir(vault_dir);
    fs::create_dir_all(&dir).with_context(|| format!("create prompts dir: {}", dir.display()))?;
    Ok(dir)
}

/// List all prompt files in a vault, returning parsed entries (without content bodies).
pub fn list_prompts(vault_dir: &Path) -> Result<Vec<PromptEntry>> {
    let dir = prompts_dir(vault_dir);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut failed = Vec::new();

    for entry in fs::read_dir(&dir).context("read prompts directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match parse_prompt_file(&path) {
            Ok(Some(parsed)) => entries.push(parsed),
            Ok(None) => {} // empty or no frontmatter
            Err(e) => {
                failed.push(format!("{}: {e}", path.display()));
            }
        }
    }

    if !failed.is_empty() {
        eprintln!(
            "Warning: failed to parse prompts:\n  {}",
            failed.join("\n  ")
        );
    }

    // Sort by name for deterministic output
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Get a single prompt by name.
pub fn get_prompt(vault_dir: &Path, name: &str) -> Result<Option<PromptEntry>> {
    let path = prompts_dir(vault_dir).join(format!("{name}.md"));
    if !path.exists() {
        return Ok(None);
    }
    parse_prompt_file(&path)
}

/// Save (create or overwrite) a prompt file.
pub fn save_prompt(vault_dir: &Path, entry: &PromptEntry) -> Result<()> {
    let dir = ensure_prompts_dir(vault_dir)?;
    let path = dir.join(format!("{}.md", entry.name));

    let frontmatter = serde_yaml_ng::to_string(&PromptFrontmatter {
        name: Some(entry.name.clone()),
        description: Some(entry.description.clone()),
        model: if entry.model.is_empty() {
            None
        } else {
            Some(entry.model.clone())
        },
    })
    .context("serialize prompt frontmatter")?;

    let content = format!("---\n{frontmatter}---\n{}\n", entry.content);
    fs::write(&path, &content).with_context(|| format!("write prompt: {}", path.display()))?;
    Ok(())
}

/// Delete a prompt file by name.
pub fn delete_prompt(vault_dir: &Path, name: &str) -> Result<bool> {
    let path = prompts_dir(vault_dir).join(format!("{name}.md"));
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path).with_context(|| format!("delete prompt: {}", path.display()))?;
    Ok(true)
}

/// Build the active prompt's custom-instructions prefix, if one is set and
/// the file exists.  Returns `None` when no prompt is active or the file is
/// missing.
pub fn load_active_prompt_content(
    vault_dir: &Path,
    active_name: Option<&str>,
) -> Result<Option<String>> {
    let name = match active_name {
        Some(n) if !n.is_empty() => n,
        _ => return Ok(None),
    };
    let entry = get_prompt(vault_dir, name)?;
    match entry {
        Some(e) if !e.content.trim().is_empty() => Ok(Some(format!(
            "[Custom Instructions — {}]\n{}\n\n",
            e.name,
            e.content.trim()
        ))),
        _ => Ok(None),
    }
}

/// Create default built-in prompts if none exist.
pub fn create_default_prompts(vault_dir: &Path) -> Result<Vec<String>> {
    let dir = ensure_prompts_dir(vault_dir)?;
    let mut created = Vec::new();

    let defaults: Vec<PromptEntry> = vec![
        PromptEntry {
            name: "general".into(),
            description: "通用对话助手 — 友好、全面的日常 AI 助手".into(),
            model: String::new(),
            content: "你是一个友好、全面的 AI 助手。你擅长回答各种问题，提供清晰的解释，\
            并在需要时主动提供额外信息。你的回答结构清晰、语言自然，\
            既不过于简略也不过于冗长。"
                .into(),
        },
        PromptEntry {
            name: "writing-assistant".into(),
            description: "写作助手 — 帮助撰写、修改和润色中英文内容".into(),
            model: String::new(),
            content: "你是一个专业的写作助手。你擅长协助撰写、修改和润色各种文体，\
            包括学术论文、技术文档、商业报告和创意写作。\
            你注重语言的准确性、逻辑的连贯性和风格的恰当性。\
            在修改时，你会解释修改原因，帮助用户提升写作能力。\
            你始终保持原文的核心观点和风格基调。"
                .into(),
        },
        PromptEntry {
            name: "code-reviewer".into(),
            description: "代码审查 — 严谨的代码审查者，注重安全性、性能和可维护性".into(),
            model: String::new(),
            content: "你是一个严谨的代码审查助手。审查代码时，你关注：\n\
            1. 安全性：检查潜在的安全漏洞和注入风险\n\
            2. 性能：识别性能瓶颈和不必要的资源消耗\n\
            3. 可维护性：评估代码结构、命名和文档质量\n\
            4. 正确性：检查边界条件和错误处理\n\
            5. Rust 最佳实践：关注所有权、生命周期和错误处理模式\n\n\
            你的反馈始终具体、可操作，并附带代码示例。"
                .into(),
        },
        PromptEntry {
            name: "researcher".into(),
            description: "研究助手 — 深度研究模式，注重来源引用和分析".into(),
            model: String::new(),
            content: "你是一个严谨的研究助手。回答问题时：\n\
            1. 优先引用可靠来源和具体数据\n\
            2. 明确区分已知事实和推测\n\
            3. 呈现多角度观点，避免片面结论\n\
            4. 提供研究建议和进一步阅读方向\n\
            5. 对于不确定的信息，坦率说明局限性\n\n\
            你的回答结构完整，包含背景介绍、核心分析、结论和参考文献。"
                .into(),
        },
    ];

    for prompt in &defaults {
        let path = dir.join(format!("{}.md", prompt.name));
        if !path.exists() {
            save_prompt(vault_dir, prompt)?;
            created.push(prompt.name.clone());
        }
    }

    Ok(created)
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// YAML frontmatter struct for serialization.
#[derive(Debug, Serialize, Deserialize)]
struct PromptFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

/// Parse a single prompt .md file, extracting YAML frontmatter and content body.
fn parse_prompt_file(path: &Path) -> Result<Option<PromptEntry>> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read prompt file: {}", path.display()))?;

    // Extract YAML frontmatter (delimited by `---` on its own line).
    let stripped = raw.trim_start();
    if !stripped.starts_with("---") {
        return Ok(None);
    }
    let after_first = &stripped[3..].trim_start();

    let end_marker = after_first
        .find("\n---")
        .map(|pos| pos + 1) // +1 for the \n
        .or_else(|| after_first.find("\n---\n").map(|pos| pos + 1))
        .unwrap_or(0);

    if end_marker == 0 {
        return Ok(None);
    }

    let yaml_str = &after_first[..end_marker];
    let content = after_first[end_marker + 4..].trim().to_string(); // skip "\n---\n"

    let frontmatter: PromptFrontmatter = serde_yaml_ng::from_str(yaml_str)
        .with_context(|| format!("parse YAML frontmatter in: {}", path.display()))?;

    let name = frontmatter
        .name
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            // Fall back to filename stem
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    Ok(Some(PromptEntry {
        name,
        description: frontmatter.description.unwrap_or_default(),
        model: frontmatter.model.unwrap_or_default(),
        content,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_dir(name: &str) -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("vp-prompt-{name}-{n}"))
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_save_and_list_prompts() {
        let dir = test_dir("save_list");
        cleanup(&dir);
        fs::create_dir_all(dir.join(".vaultpilot")).unwrap();

        let entry = PromptEntry {
            name: "test-prompt".into(),
            description: "A test".into(),
            model: "test-model".into(),
            content: "You are a test assistant.".into(),
        };
        save_prompt(&dir, &entry).unwrap();

        let listed = list_prompts(&dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "test-prompt");
        assert_eq!(listed[0].description, "A test");
        assert_eq!(listed[0].model, "test-model");
        assert_eq!(listed[0].content, "You are a test assistant.");

        cleanup(&dir);
    }

    #[test]
    fn test_get_prompt() {
        let dir = test_dir("get_prompt");
        cleanup(&dir);
        fs::create_dir_all(dir.join(".vaultpilot")).unwrap();

        let entry = PromptEntry {
            name: "my-prompt".into(),
            description: "desc".into(),
            model: String::new(),
            content: "Content here.".into(),
        };
        save_prompt(&dir, &entry).unwrap();

        let found = get_prompt(&dir, "my-prompt")
            .unwrap()
            .expect("should exist");
        assert_eq!(found.name, "my-prompt");
        assert_eq!(found.content, "Content here.");

        let missing = get_prompt(&dir, "nonexistent").unwrap();
        assert!(missing.is_none());

        cleanup(&dir);
    }

    #[test]
    fn test_delete_prompt() {
        let dir = test_dir("delete_prompt");
        cleanup(&dir);
        fs::create_dir_all(dir.join(".vaultpilot")).unwrap();

        let entry = PromptEntry {
            name: "delete-me".into(),
            description: String::new(),
            model: String::new(),
            content: "delete me".into(),
        };
        save_prompt(&dir, &entry).unwrap();
        assert!(get_prompt(&dir, "delete-me").unwrap().is_some());

        let deleted = delete_prompt(&dir, "delete-me").unwrap();
        assert!(deleted);
        assert!(get_prompt(&dir, "delete-me").unwrap().is_none());

        let not_found = delete_prompt(&dir, "doesnt-exist").unwrap();
        assert!(!not_found);

        cleanup(&dir);
    }

    #[test]
    fn test_load_active_prompt_content() {
        let dir = test_dir("active_prompt");
        cleanup(&dir);
        fs::create_dir_all(dir.join(".vaultpilot")).unwrap();

        let entry = PromptEntry {
            name: "active-one".into(),
            description: String::new(),
            model: String::new(),
            content: "Custom instructions here.".into(),
        };
        save_prompt(&dir, &entry).unwrap();

        let result = load_active_prompt_content(&dir, Some("active-one")).unwrap();
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("Custom instructions here."));
        assert!(content.contains("[Custom Instructions — active-one]"));

        // No active name → None
        let result = load_active_prompt_content(&dir, None).unwrap();
        assert!(result.is_none());

        // Empty active name → None
        let result = load_active_prompt_content(&dir, Some("")).unwrap();
        assert!(result.is_none());

        cleanup(&dir);
    }

    #[test]
    fn test_create_default_prompts() {
        let dir = test_dir("create_defaults");
        cleanup(&dir);
        fs::create_dir_all(dir.join(".vaultpilot")).unwrap();

        let created = create_default_prompts(&dir).unwrap();
        assert!(!created.is_empty());
        assert!(created.contains(&"general".to_string()));
        assert!(created.contains(&"writing-assistant".to_string()));
        assert!(created.contains(&"code-reviewer".to_string()));
        assert!(created.contains(&"researcher".to_string()));

        // Second call should create nothing (files already exist)
        let created2 = create_default_prompts(&dir).unwrap();
        assert!(created2.is_empty());

        // Verify they actually wrote files
        let listed = list_prompts(&dir).unwrap();
        assert_eq!(listed.len(), 4);

        cleanup(&dir);
    }

    #[test]
    fn test_parse_existing_md_without_frontmatter() {
        let dir = test_dir("parse_no_fm");
        cleanup(&dir);
        fs::create_dir_all(dir.join(".vaultpilot").join("prompts")).unwrap();
        let path = dir.join(".vaultpilot").join("prompts").join("plain.md");
        fs::write(&path, "Just some markdown content\nwithout frontmatter.").unwrap();

        let result = parse_prompt_file(&path).unwrap();
        assert!(result.is_none()); // No YAML frontmatter → skip

        cleanup(&dir);
    }
}
