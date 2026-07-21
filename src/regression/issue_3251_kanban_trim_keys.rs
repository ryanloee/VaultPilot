// Regression test for #3251: Kanban group keys not trimmed.
// Notes with whitespace in frontmatter values land in wrong columns.
//
// Before the fix, intake keys in `build_kanban_groups` were not trimmed,
// so "  done  " spawned a phantom "  done  " swimlane instead of matching
// the declared "done" column. The fix trims intake keys (and the
// multi-value elements produced by `run_base`) so padded values collapse
// into the correct declared column.

use crate::bases::{build_kanban_groups, BaseRow};

fn make_row(id: &str) -> BaseRow {
    BaseRow {
        note_id: id.into(),
        title: id.into(),
        values: vec![],
    }
}

/// A note whose group key has leading/trailing whitespace ("  done  ")
/// should be bucketed into the declared "done" column, not a separate
/// "  done  " column.
#[test]
fn test_kanban_trim_leading_trailing_whitespace_matches_declared() {
    let pairs = vec![("  done  ".to_string(), make_row("note-a"))];
    let order: Vec<String> = vec!["done".to_string(), "todo".to_string()];

    let groups = build_kanban_groups(pairs, Some(&order));

    // Declared "done" (with note) + "todo" (empty). No ungrouped bucket
    // because no note mapped to it.
    assert_eq!(groups.len(), 2, "expected 2 declared groups");

    let done_group = groups.iter().find(|g| g.key == "done").unwrap();
    assert_eq!(
        done_group.notes.len(),
        1,
        "'done' group should contain the trimmed note"
    );
    assert_eq!(done_group.notes[0].note_id, "note-a");

    let todo_group = groups.iter().find(|g| g.key == "todo").unwrap();
    assert!(todo_group.notes.is_empty(), "'todo' should be empty");

    // Crucially, there must be NO phantom "  done  " swimlane.
    assert!(
        groups.iter().all(|g| g.key.trim() == g.key),
        "no group key should retain surrounding whitespace"
    );
}

/// Without trimming, "  done  " and "done" would be two separate buckets.
/// After the fix they collapse into one.
#[test]
fn test_kanban_trim_collapses_equivalent_keys() {
    let pairs = vec![
        ("  done  ".to_string(), make_row("note-a")),
        ("done".to_string(), make_row("note-b")),
    ];
    let groups = build_kanban_groups(pairs, None);

    // Both notes should land in a single "done" group.
    assert_eq!(groups.len(), 1, "trimmed-equivalent keys must collapse");
    assert_eq!(groups[0].key, "done");
    assert_eq!(groups[0].notes.len(), 2, "both notes in one group");
}
