//! Built-in and user-definable Knowledge Work Skills (#1830, #2946).
//!
//! This module defines a catalog of pre-configured knowledge-work skills
//! that users can invoke directly from the CLI or UI without crafting
//! custom prompts.  Each skill encapsulates a task template (summarize,
//! weekly review, outline, concept-map, etc.) and produces a structured
//! prompt that is fed into the existing `ask` pipeline.
//!
//! In addition to the built-in catalog, users can define their **own**
//! skills as plain Markdown files under `<vault>/.vaultpilot/skills/`.
//! This enables the Notion-AI-Agent-style "workspace-aware" multi-step
//! workflow binding described in #2946 — without requiring code changes.
//!
//! Skills are *not* the system-prompt `AiSkill` structs from `prompting.rs`
//! (those guide the model's internal tool-selection behaviour).  These
//! skills are user-facing task presets — comparable to Obsidian Copilot's
//! "knowledge work skills".

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────────────────
// Data model
// ────────────────────────────────────────────────────────

/// Category for grouping skills in the UI / CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillCategory {
    /// Summarization & digest
    Summarize,
    /// Research & analysis
    Research,
    /// Writing & drafting
    Writing,
    /// Organization & cleanup
    Organize,
    /// Learning & retention
    Learning,
}

impl SkillCategory {
    pub fn label(self) -> &'static str {
        match self {
            SkillCategory::Summarize => "Summarize",
            SkillCategory::Research => "Research",
            SkillCategory::Writing => "Writing",
            SkillCategory::Organize => "Organize",
            SkillCategory::Learning => "Learning",
        }
    }
}

/// A built-in knowledge-work skill — a pre-configured task template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSkill {
    /// Unique kebab-case identifier (e.g. `"weekly-review"`)
    pub id: &'static str,
    /// Human-readable title
    pub title: &'static str,
    /// One-line description
    pub description: &'static str,
    /// Category for grouping
    pub category: SkillCategory,
    /// The prompt template.  `{input}` is replaced with user-provided input
    /// (if any).  When `{input}` is absent the skill runs with vault context
    /// only.
    pub prompt_template: &'static str,
    /// Whether the skill requires user input (e.g. a topic or note path).
    /// Skills like `weekly-review` do not.
    pub requires_input: bool,
}

impl KnowledgeSkill {
    /// Build the final prompt string, substituting `{input}` if present.
    pub fn build_prompt(&self, input: Option<&str>) -> String {
        match input {
            Some(text) if !text.trim().is_empty() => {
                self.prompt_template.replace("{input}", text.trim())
            }
            _ => {
                // If the template contains {input} but none was provided,
                // remove the placeholder gracefully.
                self.prompt_template.replace("{input}", "")
            }
        }
    }
}

// ────────────────────────────────────────────────────────
// Built-in catalog
// ────────────────────────────────────────────────────────

/// Return all built-in knowledge-work skills.
pub fn builtin_skills() -> &'static [KnowledgeSkill] {
    &BUILTIN_SKILLS
}

/// Look up a skill by its id (case-insensitive).
pub fn find_skill(id: &str) -> Option<&'static KnowledgeSkill> {
    BUILTIN_SKILLS
        .iter()
        .find(|s| s.id.eq_ignore_ascii_case(id))
}

static BUILTIN_SKILLS: [KnowledgeSkill; 7] = [
    KnowledgeSkill {
        id: "summarize",
        title: "Summarize Notes",
        description: "Generate a structured summary of notes matching a topic or path.",
        category: SkillCategory::Summarize,
        requires_input: true,
        prompt_template: "\
Summarize the most relevant notes in my vault about: {input}

Steps:
1. Search the vault for notes matching the topic.
2. Read the top 5–10 most relevant notes.
3. Produce a concise summary with:
   - **Key Themes** — 3–5 bullet points
   - **Important Details** — notable facts, decisions, or data
   - **Open Questions** — anything unresolved or unclear

Format the output as clean Markdown. Cite note titles inline.",
    },
    KnowledgeSkill {
        id: "weekly-review",
        title: "Weekly Review",
        description: "Compile a digest of this week's vault activity into a review document.",
        category: SkillCategory::Summarize,
        requires_input: false,
        prompt_template: "\
Generate a weekly review of my vault activity.

Steps:
1. Identify notes created or modified in the past 7 days.
2. Group them by theme or project.
3. Produce a structured weekly review:

## Week in Review

### Highlights
- (Top 3–5 things accomplished or captured)

### Themes
- (Recurring topics this week)

### Action Items
- (Any tasks, TODOs, or follow-ups mentioned in notes)

### Reflections
- (Suggestions: what to revisit, what's stale, what to prioritize next week)

Format as clean Markdown suitable for saving as a daily note.",
    },
    KnowledgeSkill {
        id: "outline",
        title: "Outline Generator",
        description: "Generate a structured article or document outline from vault knowledge.",
        category: SkillCategory::Writing,
        requires_input: true,
        prompt_template: "\
Create a detailed outline for a document about: {input}

Steps:
1. Search the vault for relevant knowledge and prior notes.
2. Generate a hierarchical outline with:
   - Main sections (H2) and subsections (H3)
   - 1–2 sentence description of each section's content
   - References to specific vault notes that inform each section

Format as a Markdown outline. Suggest an estimated word count for each section.",
    },
    KnowledgeSkill {
        id: "concept-map",
        title: "Concept Map",
        description: "Map related concepts and their relationships from your vault.",
        category: SkillCategory::Research,
        requires_input: true,
        prompt_template: "\
Map the concepts related to: {input}

Steps:
1. Search the vault for notes mentioning this concept.
2. Identify related concepts, sub-concepts, and broader themes.
3. Produce a structured concept map:

## Core Concept
(Brief definition from vault context)

## Related Concepts
| Concept | Relationship | Source |
|---------|-------------|--------|
| ... | broader / narrower / related / contrast | note title |

## Key Relationships
- (Explain how concepts connect, with vault citations)

## Gaps
- (Concepts that are mentioned but not well-developed in the vault)",
    },
    KnowledgeSkill {
        id: "note-cleanup",
        title: "Note Cleanup",
        description: "Identify messy, duplicate, or stale notes and suggest improvements.",
        category: SkillCategory::Organize,
        requires_input: false,
        prompt_template: "\
Analyze my vault for cleanup opportunities.

Steps:
1. Search for notes that may need attention.
2. Identify:
   - **Duplicates** — notes with very similar content
   - **Stale Notes** — old notes that may be outdated
   - **Format Issues** — notes with broken formatting, missing tags, or poor structure
   - **Orphans** — notes with no incoming links

For each category, list up to 5 notes with:
- Note title and ID
- The specific issue
- A suggested fix

Format as a clean Markdown checklist.",
    },
    KnowledgeSkill {
        id: "research-synthesis",
        title: "Research Synthesis",
        description: "Synthesize a research report on a topic from your vault knowledge.",
        category: SkillCategory::Research,
        requires_input: true,
        prompt_template: "\
Synthesize a research report on: {input}

Steps:
1. Search broadly across the vault for all relevant notes.
2. Read and analyze the most important sources.
3. Write a structured research report:

# Research Report: {input}

## Executive Summary
(2–3 paragraph overview)

## Findings
(Organized by theme, with citations to vault notes)

## Analysis
(Cross-reference findings, identify patterns and contradictions)

## Conclusions
(Key takeaways and recommendations)

## Sources
(List of vault notes referenced)

Use inline citations like [Note Title]. Aim for depth over breadth.",
    },
    KnowledgeSkill {
        id: "action-items",
        title: "Extract Action Items",
        description: "Extract tasks, decisions, and follow-ups from notes on a topic.",
        category: SkillCategory::Organize,
        requires_input: true,
        prompt_template: "\
Extract action items from notes about: {input}

Steps:
1. Search the vault for notes matching the topic.
2. Scan for:
   - **Tasks** — explicit TODOs, action items, things to do
   - **Decisions** — decisions made or conclusions reached
   - **Follow-ups** — items requiring future attention
   - **Owners** — people assigned to tasks (if mentioned)

Format as a Markdown checklist:

## Tasks
- [ ] Task description — *Source: note title*

## Decisions
- Decision — *Source: note title*

## Follow-ups
- Item — *Source: note title*

Only include items explicitly stated in the notes. Do not fabricate tasks.",
    },
];

// ────────────────────────────────────────────────────────
// User-defined skills (#2946)
// ────────────────────────────────────────────────────────

/// Directory inside the vault where user-defined skill files live.
pub const CUSTOM_SKILLS_DIR: &str = ".vaultpilot/skills";

/// Return the path to the custom skills directory under a vault.
pub fn custom_skills_dir(vault_dir: &Path) -> PathBuf {
    vault_dir.join(CUSTOM_SKILLS_DIR)
}

/// YAML frontmatter for a user-defined skill file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSkillFrontmatter {
    /// Display title (falls back to the filename stem if omitted).
    #[serde(default)]
    pub title: String,
    /// One-line description shown in listings.
    #[serde(default)]
    pub description: String,
    /// Category for grouping. Defaults to "research" when omitted or
    /// unrecognized.
    #[serde(default)]
    pub category: String,
    /// Whether this skill requires user input (a topic or path).
    /// Defaults to `false`. Set to `true` when the prompt template
    /// contains the `{input}` placeholder.
    #[serde(default)]
    pub requires_input: bool,
}

/// A user-defined skill loaded from a vault markdown file.
///
/// The file format is:
///
/// ```markdown
/// ---
/// title: My Custom Skill
/// description: A description
/// category: research
/// requires_input: true
/// ---
/// The prompt template goes here. Use {input} for user-provided text.
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSkill {
    /// Skill id — derived from the filename stem (kebab-case).
    pub id: String,
    /// Display title.
    pub title: String,
    /// One-line description.
    pub description: String,
    /// Category, parsed from frontmatter (defaults to `Research`).
    pub category: SkillCategory,
    /// Whether user input is required.
    pub requires_input: bool,
    /// The full prompt template body (after frontmatter).
    pub prompt_template: String,
    /// Source file path (for debugging / display).
    pub source_file: String,
}

impl CustomSkill {
    /// Build the final prompt string, substituting `{input}` if present.
    pub fn build_prompt(&self, input: Option<&str>) -> String {
        match input {
            Some(text) if !text.trim().is_empty() => {
                self.prompt_template.replace("{input}", text.trim())
            }
            _ => self.prompt_template.replace("{input}", ""),
        }
    }

    /// Parse a category string into `SkillCategory`, defaulting to `Research`.
    fn parse_category(s: &str) -> SkillCategory {
        match s.trim().to_ascii_lowercase().as_str() {
            "summarize" => SkillCategory::Summarize,
            "research" => SkillCategory::Research,
            "writing" => SkillCategory::Writing,
            "organize" => SkillCategory::Organize,
            "learning" => SkillCategory::Learning,
            _ => SkillCategory::Research,
        }
    }

    /// Parse a single `.md` skill file into a `CustomSkill`.
    ///
    /// The `id` is derived from the filename stem (without extension).
    /// Returns `None` if the file has no body (empty template).
    fn from_file(path: &Path) -> Option<CustomSkill> {
        let content = fs::read_to_string(path).ok()?;
        let id = path.file_stem()?.to_str()?.to_string();

        // Split frontmatter and body.
        let (fm_text, body) = split_frontmatter(&content);

        // Parse frontmatter (YAML). If it fails, use defaults.
        let fm: CustomSkillFrontmatter = if fm_text.trim().is_empty() {
            CustomSkillFrontmatter {
                title: String::new(),
                description: String::new(),
                category: String::new(),
                requires_input: false,
            }
        } else {
            serde_yaml_ng::from_str(fm_text).unwrap_or(CustomSkillFrontmatter {
                title: String::new(),
                description: String::new(),
                category: String::new(),
                requires_input: false,
            })
        };

        let body = body.trim();
        if body.is_empty() {
            return None;
        }

        // Auto-detect `requires_input` from the template body. If the author
        // omitted the frontmatter flag but the template contains the `{input}`
        // placeholder, mark the skill as requiring input so user-supplied text
        // is not silently dropped by `build_prompt` (#2980).
        let mut requires_input = fm.requires_input;
        if !requires_input && body.contains("{input}") {
            requires_input = true;
            tracing::warn!(
                skill = %id,
                "custom skill has no `requires_input: true` in frontmatter but its \
                 template contains the `{{input}}` placeholder — inferred requires_input = true"
            );
        }

        let title = if fm.title.trim().is_empty() {
            id.replace('-', " ")
        } else {
            fm.title
        };
        let description = if fm.description.trim().is_empty() {
            // Auto-generate from the first non-empty line of the body.
            body.lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .chars()
                .take(80)
                .collect()
        } else {
            fm.description
        };

        Some(CustomSkill {
            id,
            title,
            description,
            category: Self::parse_category(&fm.category),
            requires_input,
            prompt_template: body.to_string(),
            source_file: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string(),
        })
    }
}

/// Split a markdown string into `(frontmatter_yaml, body)`.
/// Frontmatter is delimited by `---` lines at the top.
fn split_frontmatter(content: &str) -> (&str, &str) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return ("", content);
    }
    // Find the closing `---` delimiter.
    let after_open = &trimmed[3..]; // skip opening ---
    if let Some(end) = after_open.find("\n---") {
        let fm = after_open[..end].trim();
        // Skip past the closing ---\n
        let body_start = end + 4; // "\n---" = 4 bytes
        let body = after_open.get(body_start..).unwrap_or("").trim_start();
        (fm, body)
    } else {
        ("", content)
    }
}

/// Load all user-defined skills from `<vault>/.vaultpilot/skills/*.md`.
///
/// Returns a vector sorted by skill id for deterministic ordering.
/// Files that fail to parse are silently skipped (with a `tracing::warn`).
/// Returns an empty vector if the directory does not exist.
pub fn load_custom_skills(vault_dir: &Path) -> Vec<CustomSkill> {
    let dir = custom_skills_dir(vault_dir);
    if !dir.is_dir() {
        return Vec::new();
    }

    let mut skills: Vec<CustomSkill> = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, dir = ?dir, "failed to read custom skills directory");
            return Vec::new();
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match CustomSkill::from_file(&path) {
            Some(skill) => skills.push(skill),
            None => {
                tracing::warn!(
                    file = ?path,
                    "skipping custom skill file: empty or invalid"
                );
            }
        }
    }

    skills.sort_by(|a, b| a.id.cmp(&b.id));
    skills
}

/// A unified skill descriptor that can be either built-in or user-defined.
/// Used by the CLI / UI to present all skills in a single list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub requires_input: bool,
    /// Whether this is a built-in or user-defined skill.
    pub source: SkillSource,
    /// The prompt template (may be large; UIs may choose to truncate).
    pub prompt_template: String,
}

/// Where a skill comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillSource {
    /// Hardcoded in the VaultPilot binary.
    Builtin,
    /// Defined by the user as a `.md` file in the vault.
    Custom,
}

/// List all available skills — both built-in and user-defined.
///
/// If a custom skill has the same id as a built-in, the custom one
/// takes precedence (user override). The result is sorted by id.
pub fn list_all_skills(vault_dir: &Path) -> Vec<SkillEntry> {
    let mut map: HashMap<String, SkillEntry> = HashMap::new();

    for skill in builtin_skills() {
        map.insert(
            skill.id.to_string(),
            SkillEntry {
                id: skill.id.to_string(),
                title: skill.title.to_string(),
                description: skill.description.to_string(),
                category: skill.category.label().to_string(),
                requires_input: skill.requires_input,
                source: SkillSource::Builtin,
                prompt_template: skill.prompt_template.to_string(),
            },
        );
    }

    for skill in load_custom_skills(vault_dir) {
        map.insert(
            skill.id.clone(),
            SkillEntry {
                id: skill.id,
                title: skill.title,
                description: skill.description,
                category: skill.category.label().to_string(),
                requires_input: skill.requires_input,
                source: SkillSource::Custom,
                prompt_template: skill.prompt_template,
            },
        );
    }

    let mut entries: Vec<SkillEntry> = map.into_values().collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

/// Look up a skill by id across both built-in and custom skills.
/// Returns the prompt template and requires_input flag.
/// Custom skills take precedence over built-ins with the same id.
pub fn resolve_skill(vault_dir: &Path, id: &str) -> Option<(String, bool, SkillSource)> {
    // Check custom skills first (user override).
    for skill in load_custom_skills(vault_dir) {
        if skill.id.eq_ignore_ascii_case(id) {
            return Some((
                skill.prompt_template,
                skill.requires_input,
                SkillSource::Custom,
            ));
        }
    }
    // Fall back to built-in.
    find_skill(id).map(|s| {
        (
            s.prompt_template.to_string(),
            s.requires_input,
            SkillSource::Builtin,
        )
    })
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_skills_nonempty() {
        assert!(!BUILTIN_SKILLS.is_empty());
        assert!(BUILTIN_SKILLS.len() >= 7);
    }

    #[test]
    fn test_skill_ids_unique() {
        let mut ids: Vec<&str> = BUILTIN_SKILLS.iter().map(|s| s.id).collect();
        ids.sort();
        let len_before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len_before, "duplicate skill IDs found");
    }

    #[test]
    fn test_skill_ids_are_kebab_case() {
        for skill in &BUILTIN_SKILLS {
            assert!(
                skill.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "skill id '{}' must be lowercase kebab-case",
                skill.id
            );
        }
    }

    #[test]
    fn test_find_skill_case_insensitive() {
        assert!(find_skill("Summarize").is_some());
        assert!(find_skill("SUMMARIZE").is_some());
        assert!(find_skill("summarize").is_some());
        assert!(find_skill("nonexistent").is_none());
    }

    #[test]
    fn test_build_prompt_with_input() {
        let skill = find_skill("summarize").unwrap();
        let prompt = skill.build_prompt(Some("Rust async runtimes"));
        assert!(prompt.contains("Rust async runtimes"));
        assert!(!prompt.contains("{input}"));
    }

    #[test]
    fn test_build_prompt_without_input() {
        let skill = find_skill("weekly-review").unwrap();
        let prompt = skill.build_prompt(None);
        assert!(!prompt.contains("{input}"));
        assert!(prompt.contains("Weekly Review") || prompt.contains("weekly review"));
    }

    #[test]
    fn test_build_prompt_empty_input() {
        let skill = find_skill("summarize").unwrap();
        let prompt = skill.build_prompt(Some("   "));
        assert!(!prompt.contains("{input}"));
    }

    #[test]
    fn test_all_skills_have_nonempty_fields() {
        for skill in &BUILTIN_SKILLS {
            assert!(!skill.id.is_empty(), "skill id is empty");
            assert!(
                !skill.title.is_empty(),
                "skill '{}' title is empty",
                skill.id
            );
            assert!(
                !skill.description.is_empty(),
                "skill '{}' description is empty",
                skill.id
            );
            assert!(
                !skill.prompt_template.is_empty(),
                "skill '{}' prompt_template is empty",
                skill.id
            );
        }
    }

    #[test]
    fn test_skill_category_labels() {
        assert_eq!(SkillCategory::Summarize.label(), "Summarize");
        assert_eq!(SkillCategory::Research.label(), "Research");
        assert_eq!(SkillCategory::Writing.label(), "Writing");
        assert_eq!(SkillCategory::Organize.label(), "Organize");
        assert_eq!(SkillCategory::Learning.label(), "Learning");
    }

    #[test]
    fn test_requires_input_flags() {
        let weekly = find_skill("weekly-review").unwrap();
        assert!(
            !weekly.requires_input,
            "weekly-review should not require input"
        );

        let summarize = find_skill("summarize").unwrap();
        assert!(summarize.requires_input, "summarize should require input");
    }

    #[test]
    fn test_issue_2980_requires_input_inferred_from_input_placeholder() {
        use std::io::Write;
        let dir =
            std::env::temp_dir().join(format!("vaultpilot-skill-test-2980-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("my-skill.md");
        {
            let mut f = std::fs::File::create(&file).unwrap();
            // Frontmatter omits `requires_input`, but the body uses {input}.
            writeln!(f, "---").unwrap();
            writeln!(f, "title: My Skill").unwrap();
            writeln!(f, "---").unwrap();
            writeln!(f, "Summarize: {{input}}").unwrap();
        }

        let skill = CustomSkill::from_file(&file).expect("skill should parse");
        assert!(
            skill.requires_input,
            "requires_input should be inferred true from {{input}} placeholder (#2980)"
        );
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_issue_2980_no_input_placeholder_leaves_requires_input_false() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-skill-test-2980b-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("plain-skill.md");
        {
            let mut f = std::fs::File::create(&file).unwrap();
            writeln!(f, "---").unwrap();
            writeln!(f, "title: Plain Skill").unwrap();
            writeln!(f, "---").unwrap();
            writeln!(f, "Do something without input.").unwrap();
        }

        let skill = CustomSkill::from_file(&file).expect("skill should parse");
        assert!(
            !skill.requires_input,
            "requires_input should remain false when no {{input}} placeholder (#2980)"
        );
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&dir);
    }
}
