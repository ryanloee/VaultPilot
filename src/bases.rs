//! Bases — structured database views over vault notes (#3127).
//!
//! Inspired by Obsidian Bases (https://help.obsidian.md/bases), this module
//! turns the vault into a queryable database: notes are rows, their frontmatter
//! fields are columns, and a `.base` YAML file describes how to filter, sort,
//! and group them into table / cards / list / **kanban** views.
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
//! view: table           # or: cards | list | kanban
//! columns:
//!   - title
//!   - status
//!   - updated_at
//! # ── kanban-only (#3247) ──
//! # group_by: status                # default when view: kanban
//! # kanban_columns: [todo, doing, done]
//! ```
//!
//! ## Kanban view (#3247)
//! `view: kanban` buckets notes into side-by-side swimlanes keyed by a
//! frontmatter field.  By default the `status` field is used; pass
//! `group_by: <field>` to bucket on any supported NoteMeta column (e.g.
//! `tags`, `board`, `platform`).  `kanban_columns` declares display order;
//! unlisted values are appended after in first-seen order, and notes whose
//! `group_by` value is empty/missing land in a trailing `未分组` column.
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
    /// Side-by-side swimlanes grouped by a frontmatter field (#3247).
    /// Use `BaseConfig.group_by` (default: `status`) to choose the column
    /// field, and `BaseConfig.kanban_columns` to declare column order.
    Kanban,
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
    /// Kanban: the NoteMeta field used to bucket notes into swimlanes (#3247).
    ///
    /// Defaults to `status` when `view == Kanban` and this is `None`.  Any
    /// `field_str`-supported field works (e.g. `status`, `tags`, `board`,
    /// `platform`).  Notes whose value is empty/missing land in a single
    /// trailing bucket labelled by [`DEFAULT_KANBAN_UNGROUPED`].
    ///
    /// Note: the YAML/JSON key is explicitly `group_by` (snake_case) to match
    /// the rest of the `.base` file vocabulary (`filters`, `sort`, `columns`)
    /// rather than the struct's default camelCase mapping.
    #[serde(rename = "group_by", default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    /// Kanban: declare the column (group) order, e.g. `["todo", "doing",
    /// "done"]`.  Groups whose key is not listed here are appended after the
    /// declared columns in first-seen order.  When `None` or empty, groups
    /// are ordered by first occurrence.
    #[serde(
        rename = "kanban_columns",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub kanban_columns: Option<Vec<String>>,
}

impl Default for BaseConfig {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            sort: Vec::new(),
            view: BaseView::Table,
            columns: Vec::new(),
            group_by: None,
            kanban_columns: None,
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

/// One swimlane in a Kanban view (#3247).
///
/// `key` is the group_by value (e.g. `"todo"`, `"doing"`, `"done"`), and
/// `notes` are the projected rows in that column, already sorted by the
/// config's `sort` directives.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KanbanGroup {
    /// The group_by field value that labels this column.
    pub key: String,
    /// Rows in this swimlane, preserving the order produced by `sort`.
    pub notes: Vec<BaseRow>,
}

/// Label used for the bucket that collects notes whose `group_by` value is
/// empty, missing, or unrecognised.
pub const DEFAULT_KANBAN_UNGROUPED: &str = "未分组";

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
    /// Populated only when `view == Kanban`; empty for every other view.
    /// Order follows `BaseConfig.kanban_columns` (declared keys first), then
    /// first-seen order for any remaining keys, with the ungrouped bucket
    /// (`DEFAULT_KANBAN_UNGROUPED`) always last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kanban_groups: Vec<KanbanGroup>,
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

    let rows: Vec<BaseRow> = notes
        .iter()
        .map(|meta| BaseRow {
            note_id: meta.id.clone(),
            title: meta.title.clone(),
            values: columns.iter().map(|c| field_str(meta, &c.field)).collect(),
        })
        .collect();

    // Kanban grouping (#3247). Only populated when the view requests it; for
    // other views `kanban_groups` stays empty and is skipped at serialize time.
    let kanban_groups = if config.view == BaseView::Kanban {
        let group_field = config
            .group_by
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("status");
        // Pair each sorted row with its group key(s). For single-value fields
        // we use `field_str` to extract one key. For multi-value array fields
        // (`tags`, `keywords`, `collections`) we expand the note into one
        // (key, row) pair per element so it appears in every matching swimlane
        // (#3258). Empty values / empty arrays fall into the ungrouped bucket.
        let is_array = matches!(group_field, "tags" | "keywords" | "collections");
        let paired: Vec<(String, BaseRow)> = notes
            .iter()
            .zip(rows.iter())
            .flat_map(|(meta, row)| {
                let keys: Vec<String> = if is_array {
                    let arr: &[String] = match group_field {
                        "tags" => &meta.tags,
                        "keywords" => &meta.keywords,
                        "collections" => &meta.collections,
                        _ => unreachable!(),
                    };
                    if arr.is_empty() {
                        vec![DEFAULT_KANBAN_UNGROUPED.to_string()]
                    } else {
                        // #3251: trim each element so padded values match
                        // declared columns.
                        arr.iter()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    }
                } else {
                    // #3251: trim the key so whitespace-padded frontmatter
                    // values match declared columns instead of spawning
                    // phantom swimlanes.
                    let s = field_str(meta, group_field).trim().to_string();
                    if s.is_empty() {
                        vec![DEFAULT_KANBAN_UNGROUPED.to_string()]
                    } else {
                        vec![s]
                    }
                };
                if keys.is_empty() {
                    vec![(DEFAULT_KANBAN_UNGROUPED.to_string(), row.clone())]
                } else {
                    keys.into_iter().map(|k| (k, row.clone())).collect()
                }
            })
            .collect();
        build_kanban_groups(paired, config.kanban_columns.as_deref())
    } else {
        Vec::new()
    };

    Ok(BaseResult {
        view: config.view,
        columns,
        rows,
        matched: notes.len(),
        scanned,
        kanban_groups,
    })
}

/// Bucket pre-projected rows into ordered Kanban swimlanes (#3247).
///
/// Pure helper extracted from [`run_base`] so it can be unit-tested without a
/// storage context.  Invariant: groups follow `column_order` (declared keys
/// first, in the given order), then any remaining keys in first-seen order,
/// with [`DEFAULT_KANBAN_UNGROUPED`] always appended last when present.
pub fn build_kanban_groups(
    pairs: Vec<(String, BaseRow)>,
    column_order: Option<&[String]>,
) -> Vec<KanbanGroup> {
    if pairs.is_empty() {
        return Vec::new();
    }

    // Preserve first-seen order for keys not declared in `column_order`.
    let mut insertion_order: Vec<String> = Vec::new();
    let mut buckets: Vec<(String, Vec<BaseRow>)> = Vec::new();
    let index_of = |key: &str, slots: &mut Vec<(String, Vec<BaseRow>)>| -> usize {
        for (i, (k, _)) in slots.iter().enumerate() {
            if k == key {
                return i;
            }
        }
        slots.push((key.to_string(), Vec::new()));
        slots.len() - 1
    };

    for (raw_key, row) in pairs {
        // #3251: trim intake keys so whitespace-padded frontmatter values
        // ("  done  ") match declared columns instead of spawning phantom
        // swimlanes. Mirrors the declared-column trim below.
        let key = raw_key.trim();
        let idx = index_of(key, &mut buckets);
        buckets[idx].1.push(row);
        if !insertion_order.iter().any(|s| s == key) {
            insertion_order.push(key.to_string());
        }
    }

    // Resolve final group ordering.
    let mut ordered: Vec<KanbanGroup> = Vec::with_capacity(buckets.len());
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1) Declared columns first, in the user's order.
    if let Some(declared) = column_order {
        for key in declared.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if seen.insert(key.to_string()) {
                if let Some(pos) = buckets.iter().position(|(k, _)| k == key) {
                    let (k, notes) = std::mem::take(&mut buckets[pos]);
                    ordered.push(KanbanGroup { key: k, notes });
                }
                // A declared column with zero matching notes still shows up as
                // an empty swimlane — this is intentional for stable UI layout.
                else {
                    ordered.push(KanbanGroup {
                        key: key.to_string(),
                        notes: Vec::new(),
                    });
                }
            }
        }
    }

    // 2) Remaining keys in first-seen order, with the ungrouped bucket last.
    let mut ungrouped: Option<KanbanGroup> = None;
    for key in insertion_order {
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key.clone());
        if let Some(pos) = buckets.iter().position(|(k, _)| *k == key) {
            let (k, notes) = std::mem::take(&mut buckets[pos]);
            if k == DEFAULT_KANBAN_UNGROUPED {
                ungrouped = Some(KanbanGroup { key: k, notes });
            } else {
                ordered.push(KanbanGroup { key: k, notes });
            }
        }
    }
    if let Some(g) = ungrouped {
        ordered.push(g);
    }

    ordered
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

    // ── Kanban view (#3247) ──────────────────────────────────────────────

    fn row(id: &str, title: &str) -> BaseRow {
        BaseRow {
            note_id: id.into(),
            title: title.into(),
            values: Vec::new(),
        }
    }

    #[test]
    fn test_parse_kanban_base_config() {
        let yaml = r#"
view: kanban
group_by: status
kanban_columns: [todo, doing, done]
filters:
  - field: tags
    op: contains
    value: "task"
"#;
        let cfg = BaseConfig::from_yaml(yaml).expect("parse");
        assert_eq!(cfg.view, BaseView::Kanban);
        assert_eq!(cfg.group_by.as_deref(), Some("status"));
        assert_eq!(
            cfg.kanban_columns.as_deref(),
            Some(["todo".to_string(), "doing".to_string(), "done".to_string()].as_slice())
        );
        assert_eq!(cfg.filters.len(), 1);
    }

    #[test]
    fn test_kanban_default_config_has_no_group_by() {
        // Defaults are backward-compatible: existing `.base` files without
        // kanban settings must keep working (Table view, no kanban output).
        let cfg = BaseConfig::default();
        assert_eq!(cfg.view, BaseView::Table);
        assert!(cfg.group_by.is_none());
        assert!(cfg.kanban_columns.is_none());
    }

    #[test]
    fn test_build_kanban_groups_respects_declared_order() {
        // Notes arrive in sort order; declared columns must appear first.
        let pairs = vec![
            ("done".to_string(), row("n1", "A")),
            ("todo".to_string(), row("n2", "B")),
            ("doing".to_string(), row("n3", "C")),
            ("todo".to_string(), row("n4", "D")),
        ];
        let order = vec!["todo".to_string(), "doing".to_string(), "done".to_string()];
        let groups = build_kanban_groups(pairs, Some(&order));

        assert_eq!(groups.len(), 3, "three declared columns");
        assert_eq!(groups[0].key, "todo");
        assert_eq!(groups[0].notes.len(), 2);
        assert_eq!(
            groups[0].notes[0].note_id, "n2",
            "first-seen order preserved"
        );
        assert_eq!(groups[0].notes[1].note_id, "n4");
        assert_eq!(groups[1].key, "doing");
        assert_eq!(groups[1].notes.len(), 1);
        assert_eq!(groups[2].key, "done");
        assert_eq!(groups[2].notes.len(), 1);
    }

    #[test]
    fn test_build_kanban_groups_declared_empty_column_shows_up() {
        // A declared column with zero matching notes still renders as an empty
        // swimlane so the UI layout stays stable across vaults.
        let pairs = vec![("todo".to_string(), row("n1", "A"))];
        let order = vec!["todo".to_string(), "doing".to_string(), "done".to_string()];
        let groups = build_kanban_groups(pairs, Some(&order));

        assert_eq!(groups.len(), 3);
        assert_eq!(groups[1].key, "doing");
        assert!(
            groups[1].notes.is_empty(),
            "declared-but-empty column shows as empty"
        );
        assert_eq!(groups[2].key, "done");
        assert!(groups[2].notes.is_empty());
    }

    #[test]
    fn test_build_kanban_groups_unknown_keys_append_in_first_seen_order() {
        let pairs = vec![
            ("blocked".to_string(), row("n1", "A")),
            ("todo".to_string(), row("n2", "B")),
            ("wishlist".to_string(), row("n3", "C")),
            ("blocked".to_string(), row("n4", "D")),
        ];
        let order = vec!["todo".to_string(), "done".to_string()];
        let groups = build_kanban_groups(pairs, Some(&order));

        // Declared todo (1 note) + declared-but-empty done, then unknown keys
        // in first-seen order: blocked (2 notes), wishlist (1 note).
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0].key, "todo");
        assert_eq!(groups[1].key, "done");
        assert!(groups[1].notes.is_empty());
        assert_eq!(groups[2].key, "blocked");
        assert_eq!(groups[2].notes.len(), 2);
        assert_eq!(groups[3].key, "wishlist");
        assert_eq!(groups[3].notes.len(), 1);
    }

    #[test]
    fn test_build_kanban_groups_ungrouped_always_last() {
        // Empty/missing group_by values are labelled DEFAULT_KANBAN_UNGROUPED
        // and must always sort last — never between declared columns.
        let pairs = vec![
            (DEFAULT_KANBAN_UNGROUPED.to_string(), row("n1", "No status")),
            ("todo".to_string(), row("n2", "B")),
            (
                DEFAULT_KANBAN_UNGROUPED.to_string(),
                row("n3", "Also no status"),
            ),
            ("done".to_string(), row("n4", "D")),
        ];
        let order = vec!["todo".to_string(), "doing".to_string(), "done".to_string()];
        let groups = build_kanban_groups(pairs, Some(&order));

        assert_eq!(groups.last().unwrap().key, DEFAULT_KANBAN_UNGROUPED);
        assert_eq!(groups.last().unwrap().notes.len(), 2);
        assert_eq!(groups.len(), 4);
    }

    #[test]
    fn test_build_kanban_groups_no_column_order_uses_first_seen() {
        // Without kanban_columns, groups appear in first-seen order, but the
        // ungrouped bucket is still pushed last.
        let pairs = vec![
            ("doing".to_string(), row("n1", "A")),
            ("todo".to_string(), row("n2", "B")),
            (DEFAULT_KANBAN_UNGROUPED.to_string(), row("n3", "C")),
            ("doing".to_string(), row("n4", "D")),
        ];
        let groups = build_kanban_groups(pairs, None);

        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].key, "doing");
        assert_eq!(groups[0].notes.len(), 2);
        assert_eq!(groups[1].key, "todo");
        assert_eq!(
            groups[2].key, DEFAULT_KANBAN_UNGROUPED,
            "ungrouped still last without order"
        );
    }

    #[test]
    fn test_build_kanban_groups_empty_input() {
        let groups = build_kanban_groups(Vec::new(), Some(&["todo".to_string()]));
        assert!(groups.is_empty());
    }

    #[test]
    fn test_default_kanban_ungrouped_label_is_localised() {
        // The label is user-visible; lock it down so a refactor doesn't
        // silently break the UI contract documented in BaseConfig.group_by.
        assert_eq!(DEFAULT_KANBAN_UNGROUPED, "未分组");
        assert!(!DEFAULT_KANBAN_UNGROUPED.is_empty());
    }
}
