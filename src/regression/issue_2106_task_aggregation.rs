/// Issue #2106: 跨笔记任务聚合视图 — scan vault `.md` files for GFM task list
/// markers (`- [ ]` / `- [x]`) and aggregate them into a queryable index.
///
/// Feature: cross-note task aggregation with `extract_tasks`, the `tasks` SQLite
/// table, and `list_tasks_with_context`. Tasks are extracted during note
/// indexing (save / index rebuild) and can be filtered by completion state.
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::notes::extract_tasks;
    use crate::storage::{
        list_tasks_with_context, rebuild_index_with_context, save_note_with_context,
        StorageContext, TaskFilter,
    };
    use chrono::Utc;

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-tasks-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    fn make_note(id: &str, title: &str, body: &str) -> NoteDocument {
        NoteDocument {
            meta: NoteMeta {
                id: id.to_string(),
                title: title.to_string(),
                tags: vec![],
                keywords: vec![],
                platform: String::new(),
                board: String::new(),
                kernel: String::new(),
                status: String::new(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                source: "test".to_string(),
                path: String::new(),
                summary: String::new(),
            },
            body: body.to_string(),
            search_snippet: None,
        }
    }

    // ── extract_tasks unit tests ────────────────────────────────────

    #[test]
    fn regression_2106_extract_tasks_basic_markers() {
        let body = "\
# Sprint plan

- [ ] Write integration tests
- [x] Set up CI
* [ ] Review PRs
+ [X] Deploy to staging

Not a task:
- Regular bullet
1. [ ] Ordered task
1. [x] Ordered done
";
        let tasks = extract_tasks(body);

        // 6 real tasks: 3 unchecked + 3 checked across -, *, +, and ordered markers.
        let (open, done): (Vec<_>, Vec<_>) = tasks.iter().partition(|t| !t.2);
        assert_eq!(open.len(), 3, "should find 3 unchecked tasks");
        assert_eq!(done.len(), 3, "should find 3 checked tasks");
        assert_eq!(tasks.len(), 6, "should find 6 tasks total");

        // Line numbers are 1-based.
        assert!(tasks.iter().all(|(ln, _, _)| *ln >= 1));
        // First task text is correct.
        assert_eq!(tasks[0].1, "Write integration tests");
        assert!(!tasks[0].2);
    }

    #[test]
    fn regression_2106_extract_tasks_ignores_non_tasks() {
        let body = "\
Some prose line with [ ] in it but no marker
- [ ]real task (no space after ])
- [  ] double space inside brackets (not a task)
- [] empty brackets (not a task)
- [ ]   	
- [X]done-no-space (not a task)
";
        let tasks = extract_tasks(body);
        // None of the above match the strict GFM pattern `- [ ] ` / `- [x] `:
        //  - prose line has no list marker
        //  - `- [ ]real` has no space after `]`
        //  - `- [  ]` has a space, not a single char, before `]`
        //  - `- []` is only 3 chars (too short)
        //  - `- [ ]   \t` matches the box but text trims to empty → skipped
        //  - `- [X]done` has no space after `]`
        assert!(
            tasks.is_empty(),
            "non-task lines should produce no tasks, got {tasks:?}"
        );
    }

    #[test]
    fn regression_2106_extract_tasks_nested_indented() {
        let body = "\
- [ ] Top-level task
    - [ ] Nested task
        - [x] Deeply nested done
";
        let tasks = extract_tasks(body);
        assert_eq!(tasks.len(), 3, "nested tasks should be included");
        assert!(tasks[2].2, "deeply nested task should be completed");
    }

    #[test]
    fn regression_2106_extract_tasks_empty_body() {
        assert!(extract_tasks("").is_empty());
        assert!(extract_tasks("no tasks here\njust prose").is_empty());
    }

    // ── Storage integration: indexing populates the tasks table ─────

    #[test]
    fn regression_2106_index_note_populates_tasks_table() {
        let (temp, ctx) = setup_temp_context();

        let note = make_note(
            "task-note-a",
            "Sprint backlog",
            "\
- [ ] Implement login
- [x] Design database
- [ ] Write docs
some prose
- [x] Code review
",
        );
        save_note_with_context(&ctx, note).expect("save note");

        // Default filter is Open → should return the 2 unchecked tasks only.
        let result = list_tasks_with_context(&ctx, TaskFilter::Open, 100).expect("list open tasks");
        assert_eq!(result.tasks.len(), 2, "2 open tasks expected");
        assert_eq!(result.total, 2);
        assert!(result.tasks.iter().all(|t| !t.completed));
        // Note title is denormalized via JOIN.
        assert!(result
            .tasks
            .iter()
            .all(|t| t.note_title == "Sprint backlog"));

        let done = list_tasks_with_context(&ctx, TaskFilter::Done, 100).expect("list done tasks");
        assert_eq!(done.tasks.len(), 2, "2 done tasks expected");
        assert!(done.tasks.iter().all(|t| t.completed));

        let all = list_tasks_with_context(&ctx, TaskFilter::All, 100).expect("list all tasks");
        assert_eq!(all.tasks.len(), 4, "4 total tasks expected");
        assert_eq!(all.total, 4);

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_2106_reindex_stale_tasks_removed() {
        let (temp, ctx) = setup_temp_context();

        let note = make_note("task-note-b", "Chores", "- [ ] Buy milk\n- [x] Pay bills\n");
        save_note_with_context(&ctx, note).expect("save note");

        // Re-save with a body that has fewer tasks → old task rows must go away.
        let updated = make_note("task-note-b", "Chores", "- [ ] Buy milk only\n");
        save_note_with_context(&ctx, updated).expect("update note");

        let all = list_tasks_with_context(&ctx, TaskFilter::All, 100).expect("list tasks");
        assert_eq!(
            all.tasks.len(),
            1,
            "stale task rows should be removed on re-index"
        );
        assert_eq!(all.tasks[0].text, "Buy milk only");

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_2106_rebuild_index_aggregates_across_notes() {
        let (temp, ctx) = setup_temp_context();

        save_note_with_context(
            &ctx,
            make_note("n1", "Alpha", "- [ ] task one\n- [x] done one\n"),
        )
        .expect("save n1");
        save_note_with_context(
            &ctx,
            make_note("n2", "Beta", "- [ ] task two\n- [ ] task three\n"),
        )
        .expect("save n2");

        let stats = rebuild_index_with_context(&ctx).expect("rebuild");
        assert!(stats.indexed >= 2, "rebuild should index the notes");

        let open = list_tasks_with_context(&ctx, TaskFilter::Open, 100).expect("list open");
        // n1 has 1 open, n2 has 2 open → 3 total.
        assert_eq!(
            open.tasks.len(),
            3,
            "aggregated open tasks across both notes"
        );
        assert_eq!(open.total, 3);

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn regression_2106_limit_caps_results_but_total_is_true_count() {
        let (temp, ctx) = setup_temp_context();

        let body: String = (0..10).map(|i| format!("- [ ] task {i}\n")).collect();
        save_note_with_context(&ctx, make_note("many", "Many", &body)).expect("save");

        let result = list_tasks_with_context(&ctx, TaskFilter::Open, 3).expect("list");
        assert_eq!(result.tasks.len(), 3, "LIMIT should cap returned rows");
        assert_eq!(result.total, 10, "total should reflect the true count");

        let _ = fs::remove_dir_all(&temp);
    }
}
