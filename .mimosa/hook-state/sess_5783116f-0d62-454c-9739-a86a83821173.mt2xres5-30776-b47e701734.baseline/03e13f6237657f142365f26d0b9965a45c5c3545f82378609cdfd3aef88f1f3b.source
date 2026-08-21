//! #3375 — Agent Plan Mode auto-detection
//!
//! Tests for `should_auto_plan()` heuristic that detects major/destructive
//! changes in a prompt and triggers Plan Mode automatically.

use crate::agent::should_auto_plan;

#[cfg(test)]
mod tests {
    use super::*;

    // ── English: bulk deletions ────────────────────────────────────────

    #[test]
    fn detects_english_bulk_delete() {
        assert!(should_auto_plan("delete all notes in the inbox folder"));
        assert!(should_auto_plan("Please delete every note from last year"));
        assert!(should_auto_plan("remove all templates"));
        assert!(should_auto_plan("remove every duplicate note"));
        assert!(should_auto_plan("drop all notes tagged 'draft'"));
        assert!(should_auto_plan("purge all archived notes"));
        assert!(should_auto_plan("erase all content from the vault"));
    }

    // ── English: batch rewrites ────────────────────────────────────────

    #[test]
    fn detects_english_batch_rewrite() {
        assert!(should_auto_plan(
            "batch rewrite all daily notes to new format"
        ));
        assert!(should_auto_plan("batch update all meeting notes with tags"));
        assert!(should_auto_plan("bulk rewrite project documentation"));
        assert!(should_auto_plan("bulk update the wiki structure"));
        assert!(should_auto_plan("rewrite all notes from 2025"));
        assert!(should_auto_plan("update all notes with new frontmatter"));
        assert!(should_auto_plan("reorganize all folders"));
    }

    // ── English: merge / archive / move ────────────────────────────────

    #[test]
    fn detects_english_merge_archive() {
        assert!(should_auto_plan("merge all duplicate notes into one"));
        assert!(should_auto_plan("consolidate all project notes"));
        assert!(should_auto_plan("combine all fragments into full notes"));
        assert!(should_auto_plan("archive all completed tasks"));
        assert!(should_auto_plan("move all notes to a new folder"));
    }

    // ── English: schema / structural changes ───────────────────────────

    #[test]
    fn detects_english_schema_change() {
        assert!(should_auto_plan(
            "schema change: add tags field to all notes"
        ));
        assert!(should_auto_plan("restructure the vault hierarchy"));
        assert!(should_auto_plan("add column 'priority' to the Bases table"));
        assert!(should_auto_plan("remove column 'legacy_id' from notes"));
        assert!(should_auto_plan("change schema to support nested folders"));
    }

    // ── Chinese: bulk deletions ────────────────────────────────────────

    #[test]
    fn detects_chinese_bulk_delete() {
        assert!(should_auto_plan("删除所有草稿笔记"));
        assert!(should_auto_plan("删除全部去年的日记"));
        assert!(should_auto_plan("批量删除重复笔记"));
        assert!(should_auto_plan("清空所有已归档的笔记"));
        assert!(should_auto_plan("清空全部收件箱"));
    }

    // ── Chinese: batch rewrites ────────────────────────────────────────

    #[test]
    fn detects_chinese_batch_rewrite() {
        assert!(should_auto_plan("批量改写所有会议纪要"));
        assert!(should_auto_plan("批量更新笔记格式"));
        assert!(should_auto_plan("批量重写项目文档"));
        assert!(should_auto_plan("重写所有日记为 Markdown 格式"));
        assert!(should_auto_plan("重写全部技术笔记"));
    }

    // ── Chinese: merge / archive / move ────────────────────────────────

    #[test]
    fn detects_chinese_merge_archive() {
        assert!(should_auto_plan("合并所有重复笔记"));
        assert!(should_auto_plan("合并全部项目笔记"));
        assert!(should_auto_plan("归档所有已完成的任务"));
        assert!(should_auto_plan("归档全部旧笔记"));
        assert!(should_auto_plan("移动所有笔记到新文件夹"));
        assert!(should_auto_plan("移动全部草稿到归档"));
        assert!(should_auto_plan("批量移动笔记到新目录"));
    }

    // ── Chinese: schema / structural changes ───────────────────────────

    #[test]
    fn detects_chinese_schema_change() {
        assert!(should_auto_plan("重构所有笔记结构"));
        assert!(should_auto_plan("重组所有文件夹"));
        assert!(should_auto_plan("批量重构 vault 目录"));
    }

    // ── Negative tests: should NOT trigger ─────────────────────────────

    #[test]
    fn does_not_trigger_for_simple_operations() {
        assert!(!should_auto_plan("create a new note about Rust"));
        assert!(!should_auto_plan("search for notes about AI"));
        assert!(!should_auto_plan("edit the meeting notes from yesterday"));
        assert!(!should_auto_plan("summarize my latest notes"));
        assert!(!should_auto_plan("add a tag to this note"));
        assert!(!should_auto_plan("delete a single word from the note"));
        assert!(!should_auto_plan("merge two paragraphs"));
        assert!(!should_auto_plan("reorganize my desk"));
    }

    #[test]
    fn does_not_trigger_for_read_only_queries() {
        assert!(!should_auto_plan("list all notes in the inbox"));
        assert!(!should_auto_plan("show me notes from last week"));
        assert!(!should_auto_plan("find notes tagged 'important'"));
        assert!(!should_auto_plan("what did I write about Rust?"));
    }

    #[test]
    fn does_not_trigger_for_empty_prompt() {
        assert!(!should_auto_plan(""));
        assert!(!should_auto_plan("   "));
    }

    // ── Case insensitivity ─────────────────────────────────────────────

    #[test]
    fn case_insensitive_matching() {
        assert!(should_auto_plan("DELETE ALL notes"));
        assert!(should_auto_plan("Delete All Notes"));
        assert!(should_auto_plan("REWRITE ALL daily notes"));
        assert!(should_auto_plan("Merge ALL duplicates"));
    }

    // ── Mixed language ─────────────────────────────────────────────────

    #[test]
    fn detects_mixed_language_prompts() {
        assert!(should_auto_plan("请帮我 delete all notes from 2024"));
        assert!(should_auto_plan("batch rewrite 所有会议纪要"));
    }
}
