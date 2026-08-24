// Regression tests for #3568: Calendar and Gallery view grouping.
// Tests the CalendarGroup/GalleryGroup structs and grouping functions directly.

#[cfg(test)]
mod tests {
    use crate::bases::{
        build_gallery_groups, BaseRow, CalendarGroup, GalleryGroup, DEFAULT_KANBAN_UNGROUPED,
    };

    fn row(id: &str, title: &str) -> BaseRow {
        BaseRow {
            note_id: id.to_string(),
            title: title.to_string(),
            values: vec![title.to_string()],
        }
    }

    /// #3568: CalendarGroup struct exists and can be constructed.
    #[test]
    fn calendar_group_struct_works_3568() {
        let group = CalendarGroup {
            key: "2026-08".to_string(),
            notes: vec![row("n1", "August Note")],
        };
        assert_eq!(group.key, "2026-08");
        assert_eq!(group.notes.len(), 1);
        assert_eq!(group.notes[0].title, "August Note");
    }

    /// #3568: GalleryGroup struct works.
    #[test]
    fn gallery_group_struct_works_3568() {
        let group = GalleryGroup {
            key: "design".to_string(),
            notes: vec![row("n1", "Design Doc"), row("n2", "UI Mock")],
        };
        assert_eq!(group.key, "design");
        assert_eq!(group.notes.len(), 2);
        assert_eq!(group.notes[0].title, "Design Doc");
        assert_eq!(group.notes[1].title, "UI Mock");
    }

    /// #3568: build_gallery_groups returns empty for empty input.
    #[test]
    fn build_gallery_groups_empty_input_3568() {
        let groups = build_gallery_groups(Vec::new());
        assert!(groups.is_empty());
    }

    /// #3568: build_gallery_groups preserves first-seen order.
    #[test]
    fn build_gallery_groups_first_seen_order_3568() {
        let pairs = vec![
            ("alpha".to_string(), row("n1", "a1")),
            ("beta".to_string(), row("n2", "b1")),
            ("alpha".to_string(), row("n3", "a2")),
            ("gamma".to_string(), row("n4", "g1")),
        ];
        let groups = build_gallery_groups(pairs);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].key, "alpha");
        assert_eq!(groups[0].notes.len(), 2);
        assert_eq!(groups[1].key, "beta");
        assert_eq!(groups[1].notes.len(), 1);
        assert_eq!(groups[2].key, "gamma");
        assert_eq!(groups[2].notes.len(), 1);
    }

    /// #3568: build_gallery_groups puts ungrouped last.
    #[test]
    fn build_gallery_groups_ungrouped_last_3568() {
        let pairs = vec![
            ("done".to_string(), row("n1", "Done task")),
            (DEFAULT_KANBAN_UNGROUPED.to_string(), row("n2", "No status")),
            ("todo".to_string(), row("n3", "Todo task")),
        ];
        let groups = build_gallery_groups(pairs);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].key, "done");
        assert_eq!(groups[1].key, "todo");
        assert_eq!(groups[2].key, DEFAULT_KANBAN_UNGROUPED);
    }

    /// #3568: CalendarGroup serializes to JSON.
    #[test]
    fn calendar_group_serializable_3568() {
        let group = CalendarGroup {
            key: "2026-08-01".to_string(),
            notes: vec![row("n1", "Note")],
        };
        let json = serde_json::to_string(&group).unwrap();
        assert!(json.contains("2026-08-01"), "missing date key: {}", json);
        assert!(json.contains("\"notes\""), "missing notes array");
    }

    /// #3568: GalleryGroup serializes to JSON.
    #[test]
    fn gallery_group_serializable_3568() {
        let group = GalleryGroup {
            key: "design".to_string(),
            notes: vec![row("n1", "Design Doc")],
        };
        let json = serde_json::to_string(&group).unwrap();
        assert!(json.contains("\"key\":\"design\""), "missing key: {}", json);
        assert!(json.contains("\"notes\""), "missing notes");
    }
}
