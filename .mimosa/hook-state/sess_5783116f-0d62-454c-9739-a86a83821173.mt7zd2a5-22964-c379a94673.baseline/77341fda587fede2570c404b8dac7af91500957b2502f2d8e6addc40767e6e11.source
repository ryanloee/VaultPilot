/// Issue #4066: find_backlinks_with_context misses Obsidian path-form
/// wikilinks `[[folder/Note]]` — inconsistent with cleanup.rs
/// build_backlinked_title_index (#3719 convention).
///
/// A link written as `[[concepts/Important]]` must count as a backlink for a
/// note titled `Important`, and the returned link_target must be the raw
/// target text (`concepts/Important`), matching the real backend contract
/// (`BacklinkEntry.link_target` = raw link text inside `[[…]]`).
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::models::{NoteDocument, NoteMeta};
    use crate::storage::{find_backlinks_with_context, save_note_with_context, StorageContext};

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-backlinks-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
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
                collections: Vec::new(),
            },
            body: body.to_string(),
            search_snippet: None,
            search_score: None,
        }
    }

    #[test]
    fn regression_4066_path_form_wikilink_counts_as_backlink() {
        let (_temp, ctx) = setup_temp_context();

        let target = make_note("target-important", "Important", "# Important\ncontent");
        let referrer = make_note(
            "referrer-a",
            "Referrer A",
            "See [[concepts/Important]] for details.",
        );
        let plain = make_note("referrer-b", "Referrer B", "Plain [[Important]] link here.");
        let alias = make_note(
            "referrer-c",
            "Referrer C",
            "Aliased [[deep/nested/Important|see note]].",
        );
        let unrelated = make_note(
            "unrelated",
            "Unrelated",
            "Links to [[Other]] and [[concepts/NotImportant]] only.",
        );

        save_note_with_context(&ctx, target).expect("save target");
        save_note_with_context(&ctx, referrer).expect("save referrer a");
        save_note_with_context(&ctx, plain).expect("save referrer b");
        save_note_with_context(&ctx, alias).expect("save referrer c");
        save_note_with_context(&ctx, unrelated).expect("save unrelated");

        let backlinks =
            find_backlinks_with_context(&ctx, "target-important").expect("find backlinks");

        // All three referrer forms must match the note titled "Important".
        let mut found: Vec<&str> = backlinks.iter().map(|b| b.meta.id.as_str()).collect();
        found.sort_unstable();
        assert_eq!(
            found,
            vec!["referrer-a", "referrer-b", "referrer-c"],
            "path-form, plain and aliased wikilinks must all count as backlinks"
        );

        // link_target must be the raw link target text (backend contract),
        // not the source note's own title.
        let by_id: std::collections::HashMap<&str, &str> = backlinks
            .iter()
            .map(|b| (b.meta.id.as_str(), b.link_target.as_str()))
            .collect();
        assert_eq!(by_id.get("referrer-a"), Some(&"concepts/Important"));
        assert_eq!(by_id.get("referrer-b"), Some(&"Important"));
        assert_eq!(by_id.get("referrer-c"), Some(&"deep/nested/Important"));
    }
}
