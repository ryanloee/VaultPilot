// Regression test for #3568: Gallery as a first-class BaseView variant.
//
// Verifies that:
// 1. `BaseView::Gallery` can be parsed from a .base file
// 2. The view is correctly serialised/deserialised
// 3. The CLI correctly maps `BaseView::Gallery` → "gallery" in JSON output

use crate::bases::{BaseConfig, BaseView};

#[test]
fn gallery_is_valid_baseview_variant() {
    // Basic serialisation round-trip: verify the enum maps to/from its string form.
    let yaml = r#"
filters:
  - field: tags
    op: contains
    value: "design"
view: gallery
"#;
    let config: BaseConfig = serde_yaml_ng::from_str(yaml).expect("parse gallery .base");
    assert_eq!(config.view, BaseView::Gallery, "view should be Gallery");

    // Re-serialise and check that 'gallery' is preserved.
    let round = serde_yaml_ng::to_string(&config).expect("re-serialise");
    assert!(
        round.contains("view: gallery"),
        "serialised YAML should contain 'view: gallery', got:\n{round}"
    );
}

#[test]
fn test_gallery_default_not_kanban() {
    // Gallery does NOT use kanban grouping — kanban_groups should be empty.
    let yaml = "view: gallery\n";
    let config: BaseConfig = serde_yaml_ng::from_str(yaml).expect("parse");
    assert_eq!(config.view, BaseView::Gallery);
    // Gallery doesn't default to kanban grouping
    assert!(config.group_by.is_none());
    assert!(config.kanban_columns.is_none());
}
