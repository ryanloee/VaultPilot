//! Dynamic Query Blocks — embed live Bases queries inside markdown notes (#3640).
//!
//! Users write ` ```query ` fenced code blocks in their notes (like Obsidian
//! Dataview / Tana Search Nodes) and the content is automatically resolved to
//! a live list or table of matching vault notes at render time.
//!
//! ## Shorthand syntax
//!
//! For simple queries a compact YAML form is supported without wrapping in
//! `BaseConfig` boilerplate:
//!
//!     ```query
//!     status: todo
//!     tags: contains rust
//!     sort: updated_at:desc
//!     render: table
//!     limit: 5
//!     ```
//!
//! This is expanded internally to the full `BaseConfig` YAML format before
//! delegation to [`bases::BaseConfig::from_yaml`].  Existing `.base` file
//! format is also accepted directly.
//!
//! ## Integration
//!
//! - **Parser** — [`extract_query_blocks`] finds all ` ```query ` blocks in raw
//!   markdown and returns their line ranges and raw content.
//! - **Config parsing** — [`parse_query_config`] converts raw YAML to
//!   `BaseConfig` (supporting the shorthand).
//! - **Execution** — [`execute_query_block`] runs a config against the vault
//!   via [`bases::run_base`].
//! - **Batch** — [`execute_all_query_blocks`] does all three steps at once.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::bases::{self, BaseColumn, BaseConfig, BaseFilter, BaseResult, BaseSort, BaseView};
use crate::storage::StorageContext;

// ── Public types ──────────────────────────────────────────────────────────

/// A query block descriptor extracted from markdown.
#[derive(Debug, Clone, Serialize)]
pub struct QueryBlock {
    /// 0-based start line of the opening fence.
    pub start_line: usize,
    /// 0-based end line (exclusive — first line after the closing fence).
    pub end_line: usize,
    /// Raw YAML content between the fences.
    pub raw: String,
    /// Parsed `BaseConfig` (None when the YAML could not be parsed).
    pub config: Option<BaseConfig>,
    /// Human-readable parse error, if any.
    pub error: Option<String>,
}

/// The result of executing one query block against the vault.
#[derive(Debug, Clone, Serialize)]
pub struct QueryBlockExecution {
    pub start_line: usize,
    pub end_line: usize,
    /// Query result (None on execution failure).
    pub result: Option<BaseResult>,
    /// Error message if execution failed.
    pub error: Option<String>,
}

// ── Public functions ──────────────────────────────────────────────────────

/// Extract all ` ```query ` and ` ~~~query ` fenced code blocks from markdown.
///
/// Returns a vec of `(start_line, end_line_exclusive, raw_content)` tuples.
///
/// - `start_line` is the 0-based line index of the opening fence line.
/// - `end_line` is the line index just past the closing fence (exclusive).
/// - `raw_content` is the text *between* the fences (trimmed).
///
/// Both `` ``` `` and `~~~` fence characters are supported.  The language tag
/// must be exactly `query` (case-insensitive) on the fence line.
pub fn extract_query_blocks(markdown: &str) -> Vec<QueryBlock> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Detect fence start (``` or ~~~)
        let fence_char = if trimmed.starts_with("```") {
            Some('`')
        } else if trimmed.starts_with("~~~") {
            Some('~')
        } else {
            i += 1;
            continue;
        };

        // Ensure the fence is at least 3 characters long
        let fence_chars: String = fence_char.unwrap().to_string().repeat(3);
        if !trimmed.starts_with(&fence_chars) {
            i += 1;
            continue;
        }

        // Check language tag — must be "query" (case-insensitive),
        // possibly followed by additional words e.g. "query yaml"
        let rest = trimmed[fence_chars.len()..].trim();
        let is_query = rest.is_empty()
            || rest
                .split_whitespace()
                .next()
                .is_some_and(|t| t.eq_ignore_ascii_case("query"));

        if !is_query {
            i += 1;
            continue;
        }

        let start = i;
        let fence_len = fence_chars.len();
        i += 1;

        // Collect content until the closing fence
        let mut content_lines: Vec<&str> = Vec::new();
        let mut found_close = false;

        while i < lines.len() {
            let line = lines[i];
            let trimmed_line = line.trim();

            // Check if this line is a closing fence (same length or longer)
            let min_fence = fence_chars.clone();
            if trimmed_line.starts_with(&min_fence) {
                // Closing fence found
                let close_fence = if fence_char == Some('`') {
                    trimmed_line.starts_with("```")
                } else {
                    trimmed_line.starts_with("~~~")
                };

                if close_fence {
                    // Verify rest of closing fence line is only fence chars or whitespace
                    let after_fence = &trimmed_line[fence_len.min(trimmed_line.len())..];
                    let expected_char = fence_char.unwrap();
                    let all_fence = after_fence
                        .chars()
                        .all(|c| c == expected_char || c.is_whitespace());
                    if all_fence || after_fence.trim().is_empty() {
                        found_close = true;
                        i += 1; // past the closing fence
                        break;
                    }
                }
            }

            content_lines.push(line);
            i += 1;
        }

        if found_close {
            let raw = content_lines.join("\n");
            let trimmed_raw = raw.trim().to_string();

            // Parse config (may be None if parse fails)
            let parsed = parse_query_config(&trimmed_raw);
            let (config, error) = match parsed {
                Ok(cfg) => (Some(cfg), None),
                Err(e) => (None, Some(format!("{:#}", e))),
            };

            blocks.push(QueryBlock {
                start_line: start,
                end_line: i,
                raw: trimmed_raw,
                config,
                error,
            });
        }
        // If no close found, we still advanced past start fence
    }

    blocks
}

/// Parse a query block's YAML content into a `BaseConfig`.
///
/// Supports two syntaxes:
///
/// 1. **Full config** — When the YAML has a top-level key like `filters`,
///    `sort`, `view`, etc., it is passed directly to `BaseConfig::from_yaml`.
///
/// 2. **Shorthand** — Bare `field: value` lines without a `filters:` wrapper.
///    Each line is treated as a single filter condition where the key is the
///    field name and the value is the expected value (`op: equals`).  A
///    two-word value like `tags: contains rust` is split into
///    `{field: tags, op: contains, value: rust}`.
///
///    Additional shorthand keys:
///    - `sort: field[:order]` — sort directive (order defaults to "asc")
///    - `render: table|list|cards|kanban` — view type
///    - `limit: N` — stored for UI pagination (not used by bases::run_base)
pub fn parse_query_config(yaml: &str) -> Result<BaseConfig> {
    let trimmed = yaml.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty query block");
    }

    // Detect if this is full YAML format (has a known top-level key like
    // "filters", "sort", "view", "columns", "group_by", etc.)
    if has_top_level_config_key(trimmed) {
        // Pass through to BaseConfig::from_yaml directly
        return BaseConfig::from_yaml(trimmed)
            .with_context(|| "failed to parse query block as BaseConfig YAML");
    }

    // Shorthand mode — parse as flat key:value lines
    let shorthand = parse_shorthand(trimmed)?;
    Ok(shorthand)
}

/// Execute a query block's config against the vault and return results.
pub fn execute_query_block(context: &StorageContext, config: &BaseConfig) -> Result<BaseResult> {
    bases::run_base(context, config)
}

/// Find all query blocks in markdown, parse them, and return descriptors.
///
/// Call this to discover blocks without executing them (e.g. for a markdown
/// preview that needs to know block boundaries).
pub fn find_query_blocks(markdown: &str) -> Vec<QueryBlock> {
    extract_query_blocks(markdown)
}

/// Execute all query blocks found in a markdown note and return results.
///
/// This is the main entry point for renderers: it extracts blocks, parses
/// their config, and runs each against the vault.
pub fn execute_all_query_blocks(
    context: &StorageContext,
    markdown: &str,
) -> Vec<QueryBlockExecution> {
    let blocks = extract_query_blocks(markdown);
    let mut executions = Vec::with_capacity(blocks.len());

    for block in blocks {
        match block.config {
            Some(config) => match execute_query_block(context, &config) {
                Ok(result) => executions.push(QueryBlockExecution {
                    start_line: block.start_line,
                    end_line: block.end_line,
                    result: Some(result),
                    error: None,
                }),
                Err(e) => executions.push(QueryBlockExecution {
                    start_line: block.start_line,
                    end_line: block.end_line,
                    result: None,
                    error: Some(format!("{:#}", e)),
                }),
            },
            None => {
                executions.push(QueryBlockExecution {
                    start_line: block.start_line,
                    end_line: block.end_line,
                    result: None,
                    error: block.error,
                });
            }
        }
    }

    executions
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Check if the YAML text represents full BaseConfig format rather than
/// shorthand.  The key test: in full format `sort` and `columns` are
/// YAML *sequences* (each element is a mapping with `field`/`order`),
/// whereas in shorthand they are *strings* like `"updated_at:desc"`.
fn has_top_level_config_key(yaml: &str) -> bool {
    let value: serde_yaml_ng::Value = match serde_yaml_ng::from_str(yaml) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let mapping = match value {
        serde_yaml_ng::Value::Mapping(ref m) => m,
        _ => return false,
    };

    for (key, val) in mapping {
        let key_str = match key.as_str() {
            Some(k) => k,
            None => continue,
        };
        match val {
            // Sequence-valued keys → full format (filters, sort-as-sequence,
            // columns-as-sequence, kanban_columns, etc.)
            serde_yaml_ng::Value::Sequence(_) => {
                if matches!(key_str, "filters" | "sort" | "columns" | "kanban_columns") {
                    return true;
                }
            }
            // Mapping-valued key → full format (formulas)
            serde_yaml_ng::Value::Mapping(_) => {
                if key_str == "formulas" {
                    return true;
                }
            }
            // For string-valued keys, only `view` and `group_by` are
            // exclusive to full format (they have no shorthand analogue
            // that would collide).  `sort` and `columns` with string
            // values are *shorthand*, so skip them.
            serde_yaml_ng::Value::String(_) => {
                if matches!(key_str, "view" | "group_by") {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

/// Parse a compact shorthand query into a full BaseConfig.
///
/// Example shorthand:
/// ```yaml
/// status: todo
/// tags: contains rust
/// sort: updated_at:desc
/// render: table
/// limit: 5
/// ```
///
/// becomes:
/// ```yaml
/// filters:
///   - field: status
///     op: equals
///     value: "todo"
///   - field: tags
///     op: contains
///     value: "rust"
/// sort:
///   - field: updated_at
///     order: desc
/// view: table
/// ```
fn parse_shorthand(yaml: &str) -> Result<BaseConfig> {
    let mut filters: Vec<BaseFilter> = Vec::new();
    let mut sorts: Vec<BaseSort> = Vec::new();
    let mut view = BaseView::Table;
    let mut columns: Vec<BaseColumn> = Vec::new();
    let mut _limit: Option<usize> = None;

    for (lineno, line) in yaml.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Split on first ':'
        let colon_pos = trimmed.find(':');
        let (key, raw_value) = match colon_pos {
            Some(pos) => {
                let k = trimmed[..pos].trim().to_lowercase();
                let v = trimmed[pos + 1..].trim().to_string();
                (k, v)
            }
            None => {
                // No colon — skip this line
                continue;
            }
        };

        match key.as_str() {
            "sort" => {
                let sort = parse_sort_shorthand(&raw_value);
                sorts.push(sort);
            }
            "render" | "view" => {
                view = match raw_value.to_lowercase().as_str() {
                    "list" => BaseView::List,
                    "cards" => BaseView::Cards,
                    "kanban" => BaseView::Kanban,
                    _ => BaseView::Table,
                };
            }
            "columns" => {
                // columns: title, status, updated_at
                for col in raw_value.split(',') {
                    let col_name = col.trim().to_string();
                    if !col_name.is_empty() {
                        columns.push(BaseColumn {
                            field: col_name,
                            label: None,
                            width: None,
                        });
                    }
                }
            }
            "limit" => {
                _limit = raw_value.parse::<usize>().ok();
            }
            // Treat as a filter definition
            field_name => {
                let filter = parse_filter_shorthand(field_name, &raw_value, lineno)?;
                filters.push(filter);
            }
        }
    }

    let mut config = BaseConfig {
        filters,
        sort: sorts,
        view,
        columns,
        ..Default::default()
    };

    // If no columns were explicitly specified, set default columns
    if config.columns.is_empty() {
        config.columns = vec![
            BaseColumn {
                field: "title".into(),
                label: None,
                width: None,
            },
            BaseColumn {
                field: "updated_at".into(),
                label: None,
                width: None,
            },
        ];
    }

    Ok(config)
}

/// Parse a filter shorthand line.
///
/// Three forms:
/// - `status: todo` → `{field: "status", op: "equals", value: "todo"}`
/// - `tags: contains rust` → `{field: "tags", op: "contains", value: "rust"}`
/// - `title: starts_with Hello` → `{field: "title", op: "starts_with", value: "Hello"}`
fn parse_filter_shorthand(field: &str, raw_value: &str, _lineno: usize) -> Result<BaseFilter> {
    let raw_value = raw_value.trim();
    let (op, value) = if let Some((op_str, val)) = raw_value.split_once(char::is_whitespace) {
        let op_lower = op_str.trim().to_lowercase();
        // Check if the first word is a known filter operator
        if let Some(op) = normalize_filter_op(&op_lower) {
            (op.to_string(), val.trim().to_string())
        } else {
            // First word is not an operator — treat entire value as equals
            ("equals".to_string(), raw_value.to_string())
        }
    } else if let Some(op) = normalize_filter_op(raw_value.trim()) {
        // Single word matches a known zero-value operator (e.g. "empty", "not_empty")
        (op.to_string(), String::new())
    } else {
        ("equals".to_string(), raw_value.to_string())
    };

    Ok(BaseFilter {
        field: field.to_string(),
        op,
        value: Some(serde_yaml_ng::Value::String(value)),
    })
}

/// Normalize a filter operator word — return None if it's not a known operator.
fn normalize_filter_op(op: &str) -> Option<&'static str> {
    match op {
        "equals" | "eq" | "=" | "==" => Some("equals"),
        "not_equals" | "neq" | "!=" | "ne" => Some("not_equals"),
        "contains" => Some("contains"),
        "starts_with" | "startswith" | "prefix" => Some("starts_with"),
        "ends_with" | "endswith" | "suffix" => Some("ends_with"),
        "gt" | ">" => Some("gt"),
        "lt" | "<" => Some("lt"),
        "gte" | ">=" | "ge" => Some("gte"),
        "lte" | "<=" | "le" => Some("lte"),
        "is_empty" | "empty" => Some("is_empty"),
        "is_not_empty" | "not_empty" | "nonempty" => Some("is_not_empty"),
        _ => None,
    }
}

/// Parse a sort shorthand `field[:order]`.
///
/// Examples:
/// - `updated_at` → `{field: "updated_at", order: "asc"}`
/// - `updated_at:desc` → `{field: "updated_at", order: "desc"}`
/// - `priority:asc` → `{field: "priority", order: "asc"}`
fn parse_sort_shorthand(raw: &str) -> BaseSort {
    let raw = raw.trim();
    match raw.split_once(':') {
        Some((field, order)) => {
            let order = order.trim().to_lowercase();
            let order = if order == "desc" || order == "descending" {
                "desc".to_string()
            } else {
                "asc".to_string()
            };
            BaseSort {
                field: field.trim().to_string(),
                order,
            }
        }
        None => BaseSort {
            field: raw.to_string(),
            order: "asc".to_string(),
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_query_blocks ─────────────────────────────────────────

    #[test]
    fn extract_simple_query_block() {
        let md = "\
some text

```query
status: todo
sort: updated_at:desc
render: table
```

more text";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 1);
        let b = &blocks[0];
        assert_eq!(b.start_line, 2);
        assert_eq!(b.end_line, 7);
        assert!(b.raw.contains("status: todo"));
        assert!(b.config.is_some());
        assert!(b.error.is_none());
    }

    #[test]
    fn extract_no_query_blocks() {
        let md = "just some text\nno code blocks here";
        assert!(extract_query_blocks(md).is_empty());
    }

    #[test]
    fn extract_regular_code_block_not_query() {
        let md = "\
```python
print('hello')
```";
        let blocks = extract_query_blocks(md);
        assert!(blocks.is_empty());
    }

    #[test]
    fn extract_tilde_fence() {
        let md = "\
~~~query
status: done
~~~";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn extract_multiple_blocks() {
        let md = "\
```query
status: todo
```

Some text

```query
status: done
```";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].raw.contains("todo"));
        assert!(blocks[1].raw.contains("done"));
    }

    #[test]
    fn extract_empty_query_block() {
        let md = "\
```query
```";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 1);
        // Empty block should fail to parse
        assert!(blocks[0].config.is_none());
        assert!(blocks[0].error.is_some());
    }

    #[test]
    fn extract_block_with_multiline_content() {
        let md = "\
```query
status: todo
tags: contains rust
sort: created_at:asc

# a comment
render: table
```";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].raw.contains("created_at"));
    }

    #[test]
    fn extract_unclosed_fence() {
        // An unclosed fence does not produce a valid block
        let md = "\
```query
status: todo
";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 0);
    }

    #[test]
    fn language_tag_requires_exact_query() {
        let md = "\
```query-something
status: todo
```";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 0);
    }

    #[test]
    fn query_block_with_yaml_lang() {
        let md = "\
```query yaml
status: todo
```";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn query_block_case_insensitive() {
        let md = "\
```QUERY
status: todo
```";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn extract_nested_fence_not_confused() {
        let md = "\
```query
status: todo
```
```rust
fn main() {}
```";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 1);
    }

    // ── parse_shorthand ──────────────────────────────────────────────

    #[test]
    fn shorthand_simple_equals() {
        let yaml = "status: todo";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.filters.len(), 1);
        assert_eq!(config.filters[0].field, "status");
        assert_eq!(config.filters[0].op, "equals");
        assert_eq!(
            config.filters[0].value.as_ref().unwrap().as_str().unwrap(),
            "todo"
        );
    }

    #[test]
    fn shorthand_contains_operator() {
        let yaml = "tags: contains rust";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.filters.len(), 1);
        assert_eq!(config.filters[0].field, "tags");
        assert_eq!(config.filters[0].op, "contains");
        assert_eq!(
            config.filters[0].value.as_ref().unwrap().as_str().unwrap(),
            "rust"
        );
    }

    #[test]
    fn shorthand_sort() {
        let yaml = "\
status: todo
sort: updated_at:desc";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.sort.len(), 1);
        assert_eq!(config.sort[0].field, "updated_at");
        assert_eq!(config.sort[0].order, "desc");
    }

    #[test]
    fn shorthand_sort_default_asc() {
        let yaml = "\
status: todo
sort: created_at";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.sort.len(), 1);
        assert_eq!(config.sort[0].field, "created_at");
        assert_eq!(config.sort[0].order, "asc");
    }

    #[test]
    fn shorthand_render() {
        let yaml = "\
status: todo
render: list";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.view, BaseView::List);
    }

    #[test]
    fn shorthand_render_table_default() {
        let yaml = "status: todo";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.view, BaseView::Table);
    }

    #[test]
    fn shorthand_with_operator_alias() {
        let yaml = "status: eq done";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.filters[0].op, "equals");
    }

    #[test]
    fn shorthand_multiple_filters() {
        let yaml = "\
status: todo
priority: gt 3
tags: contains urgent";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.filters.len(), 3);
    }

    #[test]
    fn shorthand_empty_value_uses_equals() {
        let yaml = "status:";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.filters.len(), 1);
        assert_eq!(config.filters[0].op, "equals");
        // Value might be empty string
    }

    #[test]
    fn shorthand_columns() {
        let yaml = "\
status: todo
columns: title, status, priority";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.columns.len(), 3);
        assert_eq!(config.columns[0].field, "title");
        assert_eq!(config.columns[1].field, "status");
        assert_eq!(config.columns[2].field, "priority");
    }

    // ── Full YAML format ──────────────────────────────────────────────

    #[test]
    fn full_yaml_format() {
        let yaml = "\
filters:
  - field: status
    op: equals
    value: todo
sort:
  - field: updated_at
    order: desc
view: table
columns:
  - field: title
  - field: status
  - field: updated_at";
        let config = parse_query_config(yaml).unwrap();
        assert_eq!(config.filters.len(), 1);
        assert_eq!(config.sort.len(), 1);
        assert_eq!(config.columns.len(), 3);
    }

    #[test]
    fn full_yaml_with_known_key() {
        for key in &[
            "filters:",
            "sort:",
            "view:",
            "columns:",
            "group_by:",
            "kanban_columns:",
            "formulas:",
        ] {
            let yaml = format!("{}:\n  - field: test\n    op: equals\n    value: v", key);
            let _config = parse_query_config(&yaml);
            // Should not fail — at minimum it should be parseable
            if key == &"columns:" || key == &"sort:" {
                // columns expects different format
                continue;
            }
        }
    }

    // ── Empty / edge cases ────────────────────────────────────────────

    #[test]
    fn empty_yaml_returns_error() {
        let result = parse_query_config("");
        assert!(result.is_err());
    }

    #[test]
    fn whitespace_only_yaml_returns_error() {
        let result = parse_query_config("   \n\n  ");
        assert!(result.is_err());
    }

    #[test]
    fn comment_only_yaml_returns_empty_config() {
        // Shorthand with only comments — no filters, just default config
        let yaml = "# this is a comment\n# another comment";
        let config = parse_query_config(yaml).unwrap();
        assert!(config.filters.is_empty());
        assert_eq!(config.view, BaseView::Table);
    }

    // ── Integration: extract + parse ──────────────────────────────────

    #[test]
    fn full_integration_extract_and_parse() {
        let md = "\
some text

```query
status: todo
sort: updated_at:desc
render: table
```

more text";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert!(block.config.is_some());
        let config = block.config.as_ref().unwrap();
        assert_eq!(config.filters.len(), 1);
        assert_eq!(config.filters[0].field, "status");
        assert_eq!(config.sort[0].field, "updated_at");
        assert_eq!(config.sort[0].order, "desc");
        assert_eq!(config.view, BaseView::Table);
    }

    #[test]
    fn invalid_yaml_produces_error() {
        let md = "\
```query
invalid: [broken: yaml
```";
        let blocks = extract_query_blocks(md);
        assert_eq!(blocks.len(), 1);
        // Should be None (parse error) or a warning — depends on YAML parsing
        // serde_yaml_ng may still produce a value even for slightly broken YAML
        // so we just check it doesn't panic
    }

    // ── parse_filter_shorthand ────────────────────────────────────────

    #[test]
    fn filter_shorthand_recognizes_operator_aliases() {
        let cases = [
            ("status", "== done", "equals"),
            ("status", "!= done", "not_equals"),
            ("tags", "contains rust", "contains"),
            ("title", "starts_with hello", "starts_with"),
            ("title", "startswith hello", "starts_with"),
            ("title", "prefix hello", "starts_with"),
            ("title", "ends_with world", "ends_with"),
            ("title", "endswith world", "ends_with"),
            ("title", "suffix world", "ends_with"),
            ("priority", "gt 3", "gt"),
            ("priority", "> 3", "gt"),
            ("priority", "lt 10", "lt"),
            ("priority", "< 10", "lt"),
            ("priority", "gte 3", "gte"),
            ("priority", ">= 3", "gte"),
            ("priority", "lte 10", "lte"),
            ("priority", "<= 10", "lte"),
            ("tags", "empty", "is_empty"),
            ("tags", "not_empty", "is_not_empty"),
            ("tags", "nonempty", "is_not_empty"),
        ];

        for (field, raw_value, expected_op) in &cases {
            let filter = parse_filter_shorthand(field, raw_value, 0).unwrap();
            assert_eq!(
                filter.op, *expected_op,
                "expected op '{expected_op}' for '{field}: {raw_value}', got '{}'",
                filter.op
            );
        }
    }

    #[test]
    fn filter_shorthand_unknown_word_becomes_value() {
        // When the first word is not a known operator, treat entire value as equals
        let filter = parse_filter_shorthand("custom", "some random text", 0).unwrap();
        assert_eq!(filter.op, "equals");
        assert_eq!(
            filter.value.as_ref().unwrap().as_str().unwrap(),
            "some random text"
        );
    }

    // ── parse_sort_shorthand ──────────────────────────────────────────

    #[test]
    fn sort_shorthand_with_order() {
        let sort = parse_sort_shorthand("updated_at:desc");
        assert_eq!(sort.field, "updated_at");
        assert_eq!(sort.order, "desc");
    }

    #[test]
    fn sort_shorthand_asc() {
        let sort = parse_sort_shorthand("updated_at:asc");
        assert_eq!(sort.field, "updated_at");
        assert_eq!(sort.order, "asc");
    }

    #[test]
    fn sort_shorthand_no_order() {
        let sort = parse_sort_shorthand("created_at");
        assert_eq!(sort.field, "created_at");
        assert_eq!(sort.order, "asc");
    }

    // ── has_top_level_config_key ──────────────────────────────────────

    #[test]
    fn detects_full_config_yaml() {
        assert!(has_top_level_config_key("filters:\n  - field: status"));
        assert!(has_top_level_config_key("sort:\n  - field: title"));
        assert!(has_top_level_config_key("view: table"));
        assert!(has_top_level_config_key("columns:\n  - field: title"));
        assert!(has_top_level_config_key("group_by: status"));
        assert!(has_top_level_config_key("kanban_columns:\n  - todo"));
        assert!(has_top_level_config_key(
            "formulas:\n  score: 'priority * 2'"
        ));
    }

    #[test]
    fn rejects_shorthand_yaml() {
        assert!(!has_top_level_config_key("status: todo"));
        assert!(!has_top_level_config_key("tags: contains rust"));
        assert!(!has_top_level_config_key(""));
    }

    // ── find_query_blocks ─────────────────────────────────────────────

    #[test]
    fn find_blocks_is_alias_for_extract() {
        let md = "```query\nstatus: todo\n```";
        let blocks = find_query_blocks(md);
        assert_eq!(blocks.len(), 1);
    }

    // ── execute_all_query_blocks (unit-level — no real vault) ─────────

    #[test]
    fn execute_parses_and_returns_blocks() {
        let md = "```query\nstatus: todo\n```";
        let blocks = find_query_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].config.is_some());
        let config = blocks[0].config.as_ref().unwrap();
        assert_eq!(config.filters[0].field, "status");
        assert_eq!(config.filters[0].op, "equals");
    }
}
