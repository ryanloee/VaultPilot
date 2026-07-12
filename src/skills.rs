//! Built-in Knowledge Work Skills (#1830).
//!
//! This module defines a catalog of pre-configured knowledge-work skills
//! that users can invoke directly from the CLI or UI without crafting
//! custom prompts.  Each skill encapsulates a task template (summarize,
//! weekly review, outline, concept-map, etc.) and produces a structured
//! prompt that is fed into the existing `ask` pipeline.
//!
//! Skills are *not* the system-prompt `AiSkill` structs from `prompting.rs`
//! (those guide the model's internal tool-selection behaviour).  These
//! skills are user-facing task presets — comparable to Obsidian Copilot's
//! "knowledge work skills".

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
}
