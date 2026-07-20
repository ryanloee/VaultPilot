//! Bases — structured database views over vault notes (#3127).
//!
//! Inspired by Obsidian Bases (https://help.obsidian.md/bases), this module
//! turns the vault into a queryable database: notes are rows, their frontmatter
//! fields are columns, and a `.base` YAML file describes how to filter, sort,
//! and group them into table / cards / list views.
//!
//! ## .base file format (YAML)
//!
//! ```yaml
//! filters:
//!   - field: status
//!     op: equals
//!     value: "in-progress"
//!   - field: tags
//!     op: contains
//!     value: "rust"
//! sort:
//!   - field: updated_at
//!     order: desc
//! view: table
//! columns:
//!   - title
//!   - status
//!   - updated_at
//! ```
//!
//! ## Supported filter operators
//! - `equals` / `not_equals` — exact string match
//! - `contains` — substring match (also matches any tag in the tags array)
//! - `starts_with` / `ends_with`
//! - `gt` / `lt` / `gte` / `lte` — lexicographic (works for ISO dates)
//! - `is_empty` / `is_not_empty`
//!
//! ## Property access
//! Fields map directly to [`crate::models::NoteMeta`] columns (title, tags,
//! status, platform, board, kernel, source, path, summary, created_at,
//! updated_at, collections).  `tags` and `collections` are treated as arrays
//! for `contains`; all other fields are strings.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::NoteMeta;
use crate::storage::{list_all_notes_with_context, StorageContext};

// ── Filter / Sort / View types ───────────────────────────────────────────

/// A single filter condition applied to a note property.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BaseFilter {
    /// The NoteMeta field name to test (e.g. "status", "tags", "title").
    pub field: String,
    /// Comparison operator (see module docs for the full list).
    pub op: String,
    /// The comparison value (ignored for `is_empty` / `is_not_empty`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_yaml_ng::Value>,
}

/// A single sort directive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BaseSort {
    pub field: String,
    /// "asc" or "desc" (default: asc).
    #[serde(default = "default_sort_order")]
    pub order: String,
}

fn default_sort_order() -> String {
    "asc".to_string()
}

/// The visual layout (purely declarative — rendering is UI-side).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BaseView {
    #[default]
    Table,
    Cards,
    List,
}

/// Column descriptor for table view.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BaseColumn {
    pub field: String,
    /// Optional display label (defaults to the field name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A parsed `.base` configuration file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BaseConfig {
    #[serde(default)]
    pub filters: Vec<BaseFilter>,
    #[serde(default)]
    pub sort: Vec<BaseSort>,
    #[serde(default)]
    pub view: BaseView,
    #[serde(default)]
    pub columns: Vec<BaseColumn>,
}

impl Default for BaseConfig {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            sort: Vec::new(),
            view: BaseView::Table,
            columns: Vec::new(),
        }
    }
}

impl BaseConfig {
    /// Parse a `.base` YAML config from a string.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        serde_yaml_ng::from_str(yaml)
            .with_context(|| "failed to parse .base config (expected YAML)")
    }

    /// Read and parse a `.base` file from disk.
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read .base file: {}", path.display()))?;
        Self::from_yaml(&content)
    }
}

// ── Filter evaluation ────────────────────────────────────────────────────

/// Extract the string value of a NoteMeta field by name.
///
/// Array fields (`tags`, `collections`, `keywords`) are joined with `, ` so
/// that `contains` can substring-match across them.
fn field_str(meta: &NoteMeta, field: &str) -> String {
    match field {
        "title" => meta.title.clone(),
        "tags" => meta.tags.join(", "),
        "keywords" => meta.keywords.join(", "),
        "collections" => meta.collections.join(", "),
        "platform" => meta.platform.clone(),
        "board" => meta.board.clone(),
        "kernel" => meta.kernel.clone(),
        "status" => meta.status.clone(),
        "source" => meta.source.clone(),
        "path" => meta.path.clone(),
        "summary" => meta.summary.clone(),
        "created_at" => meta.created_at.clone(),
        "updated_at" => meta.updated_at.clone(),
        "id" => meta.id.clone(),
        // Unknown field — empty string so filters gracefully exclude.
        _ => String::new(),
    }
}

/// Whether a NoteMeta array field contains a value (case-sensitive).
fn array_contains(meta: &NoteMeta, field: &str, needle: &str) -> bool {
    match field {
        "tags" => meta.tags.iter().any(|t| t == needle),
        "keywords" => meta.keywords.iter().any(|t| t == needle),
        "collections" => meta.collections.iter().any(|t| t == needle),
        // For non-array fields, fall back to substring containment.
        _ => field_str(meta, field).contains(needle),
    }
}

/// Compare two strings using ISO-8601 / lexicographic ordering.
fn cmp_strings(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

/// Evaluate a single filter against a note.  Unknown operators → no match.
fn matches_filter(meta: &NoteMeta, filter: &BaseFilter) -> bool {
    let actual = field_str(meta, &filter.field);
    let value_str = filter
        .value
        .as_ref()
        .map(|v| match v {
            serde_yaml_ng::Value::String(s) => s.clone(),
            serde_yaml_ng::Value::Bool(b) => b.to_string(),
            serde_yaml_ng::Value::Number(n) => n.to_string(),
            other => serde_yaml_ng::to_string(other)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string(),
        })
        .unwrap_or_default();

    match filter.op.as_str() {
        "equals" | "eq" | "=" => actual == value_str,
        "not_equals" | "ne" | "!=" => actual != value_str,
        "contains" => {
            array_contains(meta, &filter.field, &value_str) || actual.contains(&value_str)
        }
        "starts_with" => actual.starts_with(&value_str),
        "ends_with" => actual.ends_with(&value_str),
        "gt" => cmp_strings(&actual, &value_str) == std::cmp::Ordering::Greater,
        "lt" => cmp_strings(&actual, &value_str) == std::cmp::Ordering::Less,
        "gte" | "ge" => {
            matches!(
                cmp_strings(&actual, &value_str),
                std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
            )
        }
        "lte" | "le" => {
            matches!(
                cmp_strings(&actual, &value_str),
                std::cmp::Ordering::Less | std::cmp::Ordering::Equal
            )
        }
        "is_empty" => actual.trim().is_empty(),
        "is_not_empty" => !actual.trim().is_empty(),
        _ => false,
    }
}

/// Evaluate all filters (AND logic).  Empty filter list → match all.
fn matches_all_filters(meta: &NoteMeta, filters: &[BaseFilter]) -> bool {
    filters.iter().all(|f| matches_filter(meta, f))
}

// ── Sort ────────────────────────────────────────────────────────────────

/// Apply the sort directives to a list of notes (stable, multi-key).
fn sort_notes(notes: &mut [NoteMeta], sort: &[BaseSort]) {
    // Sort by each key in reverse order so the primary key wins.
    for directive in sort.iter().rev() {
        let field = directive.field.clone();
        let descending = directive.order.eq_ignore_ascii_case("desc");
        notes.sort_by(|a, b| {
            let va = field_str(a, &field);
            let vb = field_str(b, &field);
            let ord = cmp_strings(&va, &vb);
            if descending {
                ord.reverse()
            } else {
                ord
            }
        });
    }
}

// ── Query engine ─────────────────────────────────────────────────────────

/// A row in the resulting database view — the note id plus the requested
/// column values.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BaseRow {
    pub note_id: String,
    pub title: String,
    pub values: Vec<String>,
}

/// The result of running a Base query.
#[derive(Debug, Clone, Serialize)]
pub struct BaseResult {
    pub view: BaseView,
    pub columns: Vec<BaseColumn>,
    pub rows: Vec<BaseRow>,
    /// Total notes matched (before any UI pagination).
    pub matched: usize,
    /// Total notes scanned.
    pub scanned: usize,
}

/// Run a Base query against the vault and return the materialized rows.
///
/// This is the entry point for both the CLI (`vaultpilot bases run`) and
/// future WinUI/Mobile integrations.  It loads all notes, applies the filter
/// chain, sorts, and projects the configured columns.
pub fn run_base(context: &StorageContext, config: &BaseConfig) -> Result<BaseResult> {
    let mut notes = list_all_notes_with_context(context)?;
    let scanned = notes.len();

    // Filter
    notes.retain(|meta| matches_all_filters(meta, &config.filters));

    // Sort
    sort_notes(&mut notes, &config.sort);

    // Project columns (default: title + id)
    let columns: Vec<BaseColumn> = if config.columns.is_empty() {
        vec![
            BaseColumn {
                field: "title".into(),
                label: None,
            },
            BaseColumn {
                field: "updated_at".into(),
                label: None,
            },
        ]
    } else {
        config.columns.clone()
    };

    let rows = notes
        .iter()
        .map(|meta| BaseRow {
            note_id: meta.id.clone(),
            title: meta.title.clone(),
            values: columns.iter().map(|c| field_str(meta, &c.field)).collect(),
        })
        .collect();

    Ok(BaseResult {
        view: config.view,
        columns,
        rows,
        matched: notes.len(),
        scanned,
    })
}

// ── CLI helpers ──────────────────────────────────────────────────────────

/// Parse a single inline filter arg like `"status = in-progress"` or
/// `"tags contains rust"` or `"updated_at gt 2026-01-01"` into a BaseFilter.
///
/// Format: `field op [value]`.  Operators with no value (is_empty, is_not_empty)
/// omit the third token.  Unknown operators are still parsed — the filter engine
/// will simply never match.
pub fn base_filter_from_arg(arg: &str) -> BaseFilter {
    let parts: Vec<&str> = arg.splitn(3, ' ').collect();
    let field = parts.first().unwrap_or(&"").to_string();
    let op = parts.get(1).unwrap_or(&"equals").to_string();
    let is_valueless = matches!(op.as_str(), "is_empty" | "is_not_empty");
    let value = if is_valueless || parts.len() < 3 {
        None
    } else {
        Some(serde_yaml_ng::Value::String(parts[2].to_string()))
    };
    BaseFilter { field, op, value }
}

/// Parse an inline sort directive like `"updated_at:desc"` into a BaseSort.
///
/// Format: `field[:order]` — order defaults to `asc`.
pub fn base_sort_from_arg(arg: &str) -> BaseSort {
    let mut parts = arg.splitn(2, ':');
    let field = parts.next().unwrap_or("").to_string();
    let order = parts.next().unwrap_or("asc").to_string();
    BaseSort {
        field,
        order: if order.is_empty() {
            "asc".into()
        } else {
            order
        },
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, title: &str, status: &str, tags: &[&str], updated: &str) -> NoteMeta {
        NoteMeta {
            id: id.into(),
            title: title.into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            status: status.into(),
            updated_at: updated.into(),
            ..Default::default()
        }
    }

    #[test]
    fn test_parse_minimal_base_config() {
        let yaml = "view: table\n";
        let cfg = BaseConfig::from_yaml(yaml).expect("parse");
        assert_eq!(cfg.view, BaseView::Table);
        assert!(cfg.filters.is_empty());
        assert!(cfg.sort.is_empty());
    }

    #[test]
    fn test_parse_full_base_config() {
        let yaml = r#"
filters:
  - field: status
    op: equals
    value: "in-progress"
  - field: tags
    op: contains
    value: "rust"
sort:
  - field: updated_at
    order: desc
view: cards
columns:
  - field: title
    label: "Title"
  - field: status
"#;
        let cfg = BaseConfig::from_yaml(yaml).expect("parse");
        assert_eq!(cfg.filters.len(), 2);
        assert_eq!(cfg.filters[0].field, "status");
        assert_eq!(cfg.filters[0].op, "equals");
        assert_eq!(cfg.sort.len(), 1);
        assert_eq!(cfg.sort[0].field, "updated_at");
        assert_eq!(cfg.sort[0].order, "desc");
        assert_eq!(cfg.view, BaseView::Cards);
        assert_eq!(cfg.columns.len(), 2);
        assert_eq!(cfg.columns[0].label.as_deref(), Some("Title"));
    }

    #[test]
    fn test_filter_equals() {
        let m = meta("n1", "Note A", "done", &[], "2026-01-01");
        let f = BaseFilter {
            field: "status".into(),
            op: "equals".into(),
            value: Some(serde_yaml_ng::Value::String("done".into())),
        };
        assert!(matches_filter(&m, &f));

        let f2 = BaseFilter {
            field: "status".into(),
            op: "equals".into(),
            value: Some(serde_yaml_ng::Value::String("todo".into())),
        };
        assert!(!matches_filter(&m, &f2));
    }

    #[test]
    fn test_filter_tags_contains() {
        let m = meta("n1", "Note", "x", &["rust", "async"], "2026-01-01");
        let f = BaseFilter {
            field: "tags".into(),
            op: "contains".into(),
            value: Some(serde_yaml_ng::Value::String("rust".into())),
        };
        assert!(matches_filter(&m, &f));

        let f2 = BaseFilter {
            field: "tags".into(),
            op: "contains".into(),
            value: Some(serde_yaml_ng::Value::String("python".into())),
        };
        assert!(!matches_filter(&m, &f2));
    }

    #[test]
    fn test_filter_is_empty() {
        let m = meta("n1", "Note", "", &[], "2026-01-01");
        let f = BaseFilter {
            field: "status".into(),
            op: "is_empty".into(),
            value: None,
        };
        assert!(matches_filter(&m, &f));

        let m2 = meta("n2", "Note", "active", &[], "2026-01-01");
        assert!(!matches_filter(&m2, &f));
    }

    #[test]
    fn test_filter_unknown_op_never_matches() {
        let m = meta("n1", "Note", "x", &[], "2026-01-01");
        let f = BaseFilter {
            field: "status".into(),
            op: "regex".into(),
            value: Some(serde_yaml_ng::Value::String(".*".into())),
        };
        assert!(!matches_filter(&m, &f));
    }

    #[test]
    fn test_sort_by_updated_at_desc() {
        let mut notes = vec![
            meta("n1", "A", "x", &[], "2026-01-01"),
            meta("n2", "B", "x", &[], "2026-03-01"),
            meta("n3", "C", "x", &[], "2026-02-01"),
        ];
        sort_notes(
            &mut notes,
            &[BaseSort {
                field: "updated_at".into(),
                order: "desc".into(),
            }],
        );
        assert_eq!(notes[0].id, "n2"); // 2026-03
        assert_eq!(notes[1].id, "n3"); // 2026-02
        assert_eq!(notes[2].id, "n1"); // 2026-01
    }

    #[test]
    fn test_sort_multi_key() {
        let mut notes = vec![
            meta("n1", "B", "todo", &[], "2026-01-02"),
            meta("n2", "A", "todo", &[], "2026-01-01"),
            meta("n3", "C", "done", &[], "2026-01-03"),
        ];
        // Primary: status asc, secondary: updated_at asc
        sort_notes(
            &mut notes,
            &[
                BaseSort {
                    field: "status".into(),
                    order: "asc".into(),
                },
                BaseSort {
                    field: "updated_at".into(),
                    order: "asc".into(),
                },
            ],
        );
        assert_eq!(notes[0].id, "n3"); // done first
        assert_eq!(notes[1].id, "n2"); // todo, 01-01
        assert_eq!(notes[2].id, "n1"); // todo, 01-02
    }

    #[test]
    fn test_matches_all_filters_and_logic() {
        let m = meta("n1", "Note", "in-progress", &["rust"], "2026-01-01");
        let filters = vec![
            BaseFilter {
                field: "status".into(),
                op: "equals".into(),
                value: Some(serde_yaml_ng::Value::String("in-progress".into())),
            },
            BaseFilter {
                field: "tags".into(),
                op: "contains".into(),
                value: Some(serde_yaml_ng::Value::String("rust".into())),
            },
        ];
        assert!(matches_all_filters(&m, &filters));

        // Failing one filter → false
        let filters2 = vec![
            BaseFilter {
                field: "status".into(),
                op: "equals".into(),
                value: Some(serde_yaml_ng::Value::String("done".into())),
            },
            BaseFilter {
                field: "tags".into(),
                op: "contains".into(),
                value: Some(serde_yaml_ng::Value::String("rust".into())),
            },
        ];
        assert!(!matches_all_filters(&m, &filters2));
    }

    #[test]
    fn test_base_filter_from_arg_simple() {
        let f = base_filter_from_arg("status = in-progress");
        assert_eq!(f.field, "status");
        assert_eq!(f.op, "=");
        assert_eq!(
            f.value,
            Some(serde_yaml_ng::Value::String("in-progress".into()))
        );
    }

    #[test]
    fn test_base_filter_from_arg_contains() {
        let f = base_filter_from_arg("tags contains rust");
        assert_eq!(f.field, "tags");
        assert_eq!(f.op, "contains");
        assert_eq!(f.value, Some(serde_yaml_ng::Value::String("rust".into())));
    }

    #[test]
    fn test_base_filter_from_arg_valueless() {
        let f = base_filter_from_arg("summary is_empty");
        assert_eq!(f.field, "summary");
        assert_eq!(f.op, "is_empty");
        assert!(f.value.is_none());
    }

    #[test]
    fn test_base_sort_from_arg() {
        let s = base_sort_from_arg("updated_at:desc");
        assert_eq!(s.field, "updated_at");
        assert_eq!(s.order, "desc");

        let s2 = base_sort_from_arg("title");
        assert_eq!(s2.field, "title");
        assert_eq!(s2.order, "asc");
    }
}
