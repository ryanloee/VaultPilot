//! Regression tests for user-definable skills (#2946).
//!
//! Verifies that:
//! - Custom skills load from `<vault>/.vaultpilot/skills/*.md`
//! - Frontmatter parsing works (title, description, category, requires_input)
//! - Custom skills override built-ins with the same id
//! - `list_all_skills` merges built-in + custom (dedup by id)
//! - `resolve_skill` finds both built-in and custom skills
//! - Empty/invalid files are skipped gracefully
//! - Missing directory returns empty vec (no panic)

use std::fs;
use std::path::{Path, PathBuf};

use crate::skills::{
    custom_skills_dir, list_all_skills, load_custom_skills, resolve_skill, SkillSource,
};

fn temp_vault(label: &str) -> PathBuf {
    let temp = std::env::temp_dir().join(format!(
        "vaultpilot-reg2946-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp).expect("temp dir");
    temp
}

fn write_skill(vault: &Path, filename: &str, content: &str) {
    let dir = custom_skills_dir(vault);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(filename), content).unwrap();
}

#[test]
fn test_load_custom_skills_empty_dir() {
    let tmp = temp_vault("empty");
    // No skills directory — should return empty vec, not panic.
    let skills = load_custom_skills(&tmp);
    assert!(skills.is_empty());
}

#[test]
fn test_load_custom_skills_basic() {
    let tmp = temp_vault("basic");
    write_skill(
        &tmp,
        "meeting-notes.md",
        "---\ntitle: Meeting Notes\ndescription: Generate meeting notes\ncategory: writing\nrequires_input: true\n---\nCreate structured meeting notes about: {input}\n\n1. Identify attendees\n2. Capture decisions\n3. List action items\n",
    );

    let skills = load_custom_skills(&tmp);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id, "meeting-notes");
    assert_eq!(skills[0].title, "Meeting Notes");
    assert_eq!(skills[0].description, "Generate meeting notes");
    assert!(skills[0].requires_input);
    assert!(skills[0].prompt_template.contains("{input}"));
}

#[test]
fn test_load_custom_skills_no_frontmatter() {
    let tmp = temp_vault("no_fm");
    write_skill(
        &tmp,
        "simple-skill.md",
        "Just search the vault and summarize what you find.",
    );

    let skills = load_custom_skills(&tmp);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id, "simple-skill");
    assert!(!skills[0].requires_input);
    // Title falls back to filename with spaces.
    assert_eq!(skills[0].title, "simple skill");
}

#[test]
fn test_load_custom_skills_empty_file_skipped() {
    let tmp = temp_vault("empty_file");
    write_skill(&tmp, "empty.md", "");
    write_skill(&tmp, "frontmatter-only.md", "---\ntitle: No Body\n---\n");

    let skills = load_custom_skills(&tmp);
    assert!(skills.is_empty(), "empty/body-less files should be skipped");
}

#[test]
fn test_load_custom_skills_multiple_files() {
    let tmp = temp_vault("multiple");
    write_skill(&tmp, "zebra.md", "---\n---\nZ prompt");
    write_skill(&tmp, "alpha.md", "---\n---\nA prompt");
    write_skill(&tmp, "midnight.md", "---\n---\nM prompt");

    let skills = load_custom_skills(&tmp);
    assert_eq!(skills.len(), 3);
    // Sorted by id.
    assert_eq!(skills[0].id, "alpha");
    assert_eq!(skills[1].id, "midnight");
    assert_eq!(skills[2].id, "zebra");
}

#[test]
fn test_non_md_files_ignored() {
    let tmp = temp_vault("non_md");
    write_skill(&tmp, "valid.md", "---\n---\nValid prompt");
    // Also write a .txt file — should be ignored.
    let dir = custom_skills_dir(&tmp);
    fs::write(dir.join("readme.txt"), "not a skill").unwrap();

    let skills = load_custom_skills(&tmp);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id, "valid");
}

#[test]
fn test_list_all_skills_merges_builtin_and_custom() {
    let tmp = temp_vault("merge");
    write_skill(
        &tmp,
        "my-custom-skill.md",
        "---\ntitle: My Custom\ndescription: Custom desc\n---\nDo something custom.",
    );

    let entries = list_all_skills(&tmp);
    // Should include all 7 built-in skills + 1 custom.
    assert!(
        entries.len() >= 8,
        "expected at least 8 entries (7 builtin + 1 custom), got {}",
        entries.len()
    );

    // Custom skill should be in the list.
    let custom = entries.iter().find(|e| e.id == "my-custom-skill");
    assert!(custom.is_some(), "custom skill not found in list");
    assert_eq!(custom.unwrap().source, SkillSource::Custom);

    // Built-in skills should still be present.
    let builtin = entries.iter().find(|e| e.id == "summarize");
    assert!(builtin.is_some(), "built-in summarize skill missing");
    assert_eq!(builtin.unwrap().source, SkillSource::Builtin);
}

#[test]
fn test_custom_skill_overrides_builtin_same_id() {
    let tmp = temp_vault("override");
    // Create a custom skill with the same id as a built-in ("summarize").
    write_skill(
        &tmp,
        "summarize.md",
        "---\ntitle: Custom Summarize\ndescription: My custom summarizer\n---\nCustom summary prompt.",
    );

    // resolve_skill should return the custom version, not the built-in.
    let (prompt, _requires_input, source) = resolve_skill(&tmp, "summarize").unwrap();
    assert_eq!(source, SkillSource::Custom);
    assert_eq!(prompt, "Custom summary prompt.");
}

#[test]
fn test_resolve_skill_builtin() {
    let tmp = temp_vault("resolve_builtin");
    // No custom skills dir — should still find built-in.
    let (prompt, requires_input, source) = resolve_skill(&tmp, "weekly-review").unwrap();
    assert_eq!(source, SkillSource::Builtin);
    assert!(!requires_input);
    assert!(prompt.contains("weekly review") || prompt.contains("Weekly Review"));
}

#[test]
fn test_resolve_skill_not_found() {
    let tmp = temp_vault("not_found");
    assert!(resolve_skill(&tmp, "nonexistent-skill-xyz").is_none());
}

#[test]
fn test_resolve_skill_case_insensitive() {
    let tmp = temp_vault("case_insensitive");
    write_skill(&tmp, "my-skill.md", "---\n---\nMy skill prompt.");

    // Case-insensitive lookup.
    assert!(resolve_skill(&tmp, "My-Skill").is_some());
    assert!(resolve_skill(&tmp, "MY-SKILL").is_some());
    assert!(resolve_skill(&tmp, "my-skill").is_some());
}

#[test]
fn test_custom_skill_build_prompt() {
    use crate::skills::CustomSkill;

    let tmp = temp_vault("build_prompt");
    write_skill(
        &tmp,
        "topic-skill.md",
        "---\nrequires_input: true\n---\nResearch {input} in detail.",
    );

    let skills = load_custom_skills(&tmp);
    assert_eq!(skills.len(), 1);
    let skill: &CustomSkill = &skills[0];

    // With input.
    let prompt = skill.build_prompt(Some("quantum computing"));
    assert!(prompt.contains("quantum computing"));
    assert!(!prompt.contains("{input}"));

    // Without input — placeholder removed.
    let prompt = skill.build_prompt(None);
    assert!(!prompt.contains("{input}"));
}

#[test]
fn test_custom_skill_category_parsing() {
    let tmp = temp_vault("categories");
    write_skill(
        &tmp,
        "writing-skill.md",
        "---\ncategory: writing\n---\nWrite something.",
    );
    write_skill(
        &tmp,
        "organize-skill.md",
        "---\ncategory: organize\n---\nOrganize something.",
    );
    write_skill(
        &tmp,
        "unknown-skill.md",
        "---\ncategory: nonexistent\n---\nUnknown category.",
    );

    let skills = load_custom_skills(&tmp);
    let by_id: std::collections::HashMap<String, _> =
        skills.into_iter().map(|s| (s.id.clone(), s)).collect();

    assert_eq!(
        by_id.get("writing-skill").unwrap().category.label(),
        "Writing"
    );
    assert_eq!(
        by_id.get("organize-skill").unwrap().category.label(),
        "Organize"
    );
    // Unknown category defaults to Research.
    assert_eq!(
        by_id.get("unknown-skill").unwrap().category.label(),
        "Research"
    );
}
