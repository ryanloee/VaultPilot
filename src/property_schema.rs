//! Property Type System — typed frontmatter fields (#3501, #3569).
//!
//! Inspired by Logseq DB `:asset`/`:date` property types and Obsidian Bases
//! typed properties.  This module lets users declare the type of custom
//! frontmatter fields in a vault-level `.vp/property-schema.yml` file so that
//! the Bases engine can do type-correct comparisons (numeric, date, boolean)
//! instead of always falling back to lexicographic string comparison.
//!
//! ## Schema file format (`.vp/property-schema.yml`)
//!
//! ```yaml
//! properties:
//!   due_date: date
//!   priority: number
//!   participants: tags
//!   attachment: asset
//!   is_archived: checkbox
//!   status: enum               # <-- Enum type for fixed-option fields
//! enum_options:
//!   status: [doing, done, blocked]  # Allowed values for the enum field
//! ```
//!
//! Fields not listed in the schema default to [`PropertyType::Text`].
//!
//! ## Built-in field defaults
//!
//! The following built-in NoteMeta fields have sensible default types that are
//! applied even without a schema file:
//!
//! | Field        | Default type |
//! |--------------|-------------|
//! | `tags`       | Tags        |
//! | `keywords`   | Tags        |
//! | `collections`| Tags        |
//! | `created_at` | Date        |
//! | `updated_at` | Date        |

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::models::NoteMeta;

// ── PropertyType enum ────────────────────────────────────────────────────

/// The semantic type of a frontmatter property.
///
/// Controls how the Bases filter/sort engine compares values for that field.
/// Unknown or un-declared fields are treated as [`PropertyType::Text`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PropertyType {
    /// Plain text — lexicographic string comparison (default).
    #[default]
    Text,
    /// Numeric — values are parsed as `f64` before comparison.
    ///
    /// Fixes the classic `"10" < "2"` lexicographic bug.
    Number,
    /// Date / datetime — values are normalized to a comparable date key.
    ///
    /// Supports ISO-8601 (`2026-07-27`), RFC-3339 timestamps, and slash
    /// separators (`2026/07/27`).
    Date,
    /// Boolean — `"true"`/`"false"`/`"yes"`/`"no"`/`"1"`/`"0"` (case-insensitive).
    Checkbox,
    /// Array of strings — `contains` matches any element.
    Tags,
    /// File/asset path — treated as text for comparison.
    Asset,
    /// Array of text values — like Tags but with different UI semantics.
    MultiText,
    /// A string value that must match one of a predefined set of options.
    ///
    /// Allowed values are stored in [`PropertySchema::enum_options`] under
    /// the field name.  When no options are defined, any string is accepted
    /// (treated as free-text, same as [`PropertyType::Text`]).
    Enum,
}

impl PropertyType {
    /// Returns `true` if this type should be compared as a number.
    pub fn is_numeric(self) -> bool {
        matches!(self, PropertyType::Number)
    }

    /// Returns `true` if this type should be compared as a date.
    pub fn is_date(self) -> bool {
        matches!(self, PropertyType::Date)
    }

    /// Returns `true` if this type is array-like (multiple values per note).
    pub fn is_array(self) -> bool {
        matches!(self, PropertyType::Tags | PropertyType::MultiText)
    }

    /// Returns `true` if this type is an enum (select with predefined options).
    pub fn is_enum(self) -> bool {
        matches!(self, PropertyType::Enum)
    }
}

impl std::fmt::Display for PropertyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PropertyType::Text => "text",
            PropertyType::Number => "number",
            PropertyType::Date => "date",
            PropertyType::Checkbox => "checkbox",
            PropertyType::Tags => "tags",
            PropertyType::Asset => "asset",
            PropertyType::MultiText => "multitext",
            PropertyType::Enum => "enum",
        };
        write!(f, "{s}")
    }
}

// ── PropertySchema ───────────────────────────────────────────────────────

/// A vault-level declaration of custom frontmatter field types.
///
/// Loaded from `.vp/property-schema.yml`.  Fields not present in the schema
/// get a default type determined by [`PropertySchema::builtin_type`].
///
/// Supports `Enum` fields with predefined options stored in
/// [`enum_options`](PropertySchema::enum_options).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PropertySchema {
    /// User-declared field → type mappings.
    #[serde(default)]
    pub properties: HashMap<String, PropertyType>,
    /// Allowed values for [`PropertyType::Enum`] fields.
    ///
    /// Keyed by field name.  When a field is declared as `enum` but has no
    /// entry here, any string value is accepted.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub enum_options: HashMap<String, Vec<String>>,
}

impl PropertySchema {
    /// Create an empty schema (all fields default to Text/builtins).
    pub fn empty() -> Self {
        Self {
            properties: HashMap::new(),
            enum_options: HashMap::new(),
        }
    }

    /// Parse a property schema from a YAML string.
    ///
    /// Expected format:
    /// ```yaml
    /// properties:
    ///   priority: number
    ///   due_date: date
    ///   status: enum
    /// enum_options:
    ///   status: [doing, done, blocked]
    /// ```
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        if yaml.trim().is_empty() {
            return Ok(Self::empty());
        }
        serde_yaml_ng::from_str(yaml)
            .with_context(|| "failed to parse property-schema.yml (expected YAML)")
    }

    /// Load the property schema from a vault root directory.
    ///
    /// Reads `.vp/property-schema.yml` relative to `vault_root`.  Returns an
    /// empty schema if the file does not exist (common case).
    pub fn load_from_vault(vault_root: &Path) -> Self {
        let schema_path = vault_root.join(".vp").join("property-schema.yml");
        match std::fs::read_to_string(&schema_path) {
            Ok(content) => Self::from_yaml(&content).unwrap_or_else(|e| {
                eprintln!(
                    "warning: failed to parse {}: {} — using defaults",
                    schema_path.display(),
                    e
                );
                Self::empty()
            }),
            Err(_) => Self::empty(),
        }
    }

    /// Declare a field type programmatically (used in tests and builders).
    pub fn with(mut self, field: &str, ty: PropertyType) -> Self {
        self.properties.insert(field.to_string(), ty);
        self
    }

    /// Declare a field type with enum options (convenience for tests/builders).
    ///
    /// Also sets the field's type to [`PropertyType::Enum`] if not already set.
    pub fn with_enum(mut self, field: &str, options: Vec<String>) -> Self {
        self.properties
            .entry(field.to_string())
            .or_insert(PropertyType::Enum);
        self.enum_options.insert(field.to_string(), options);
        self
    }

    /// Get the declared type for a field, falling back to built-in defaults.
    pub fn type_of(&self, field: &str) -> PropertyType {
        if let Some(ty) = self.properties.get(field) {
            return *ty;
        }
        Self::builtin_type(field)
    }

    /// Returns the allowed values for an Enum field, if defined.
    pub fn enum_options_for(&self, field: &str) -> Option<&[String]> {
        self.enum_options.get(field).map(|v| v.as_slice())
    }

    /// Returns `true` if `value` is valid for the given field according to
    /// its declared type.  Returns `None` when valid, or `Some(reason)` when
    /// invalid.  Empty/blank values are always accepted (field may be unset).
    pub fn validate_value(&self, field: &str, value: &str) -> Option<String> {
        if value.trim().is_empty() {
            return None;
        }
        let ty = self.type_of(field);
        match ty {
            PropertyType::Number => {
                if value.trim().parse::<f64>().is_err() {
                    Some(format!(
                        "field '{field}' is declared as `number` but '{value}' is not a valid number"
                    ))
                } else {
                    None
                }
            }
            PropertyType::Date => {
                let normalized = normalize_date(value);
                if normalized == value.trim() && !is_date_like(value) {
                    // Double-check: if normalize returned the input unchanged
                    // and it doesn't look like a date, it's suspicious.
                    if !looks_like_date(value) {
                        Some(format!(
                            "field '{field}' is declared as `date` but '{value}' does not look like a valid date (expected ISO-8601)"
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            PropertyType::Checkbox => {
                if parse_bool(value).is_none() {
                    Some(format!(
                        "field '{field}' is declared as `checkbox` but '{value}' is not a valid boolean (expected true/false/yes/no/1/0)"
                    ))
                } else {
                    None
                }
            }
            PropertyType::Enum => {
                if let Some(options) = self.enum_options.get(field) {
                    if !options.iter().any(|o| o.eq_ignore_ascii_case(value.trim())) {
                        Some(format!(
                            "field '{field}' is declared as `enum` with options {options:?} but '{value}' is not one of the allowed values"
                        ))
                    } else {
                        None
                    }
                } else {
                    // Enum without options → free-text, any value accepted.
                    None
                }
            }
            _ => None,
        }
    }

    /// Validate all declared frontmatter fields of a `NoteMeta` against the
    /// schema.  Returns a list of validation warnings (non-empty means at
    /// least one field had an invalid value).
    ///
    /// This is non-blocking: notes are always saved regardless of warnings.
    /// Callers may log, display, or ignore the returned warnings.
    pub fn validate_note_meta(&self, meta: &NoteMeta) -> Vec<String> {
        let mut warnings = Vec::new();

        // Only validate fields that are declared in the schema.
        for field in self.properties.keys() {
            let value = match field.as_str() {
                "title" => &meta.title,
                "status" => &meta.status,
                "platform" => &meta.platform,
                "board" => &meta.board,
                "kernel" => &meta.kernel,
                "source" => &meta.source,
                "summary" => &meta.summary,
                "created_at" => &meta.created_at,
                "updated_at" => &meta.updated_at,
                "id" => &meta.id,
                "path" => &meta.path,
                // Array fields are validated element-by-element.
                "tags" => {
                    for t in &meta.tags {
                        if let Some(warn) = self.validate_value(field, t) {
                            warnings.push(warn);
                        }
                    }
                    continue;
                }
                "keywords" => {
                    for t in &meta.keywords {
                        if let Some(warn) = self.validate_value(field, t) {
                            warnings.push(warn);
                        }
                    }
                    continue;
                }
                "collections" => {
                    for t in &meta.collections {
                        if let Some(warn) = self.validate_value(field, t) {
                            warnings.push(warn);
                        }
                    }
                    continue;
                }
                _ => {
                    // Unknown field — skip (may be a custom frontmatter field
                    // outside NoteMeta).  The Bases engine handles these via
                    // full frontmatter YAML parsing.
                    continue;
                }
            };

            if let Some(warn) = self.validate_value(field, value) {
                warnings.push(warn);
            }
        }

        warnings
    }

    /// The built-in default type for well-known NoteMeta fields.
    ///
    /// This provides sensible behaviour even without a schema file:
    /// - `tags`, `keywords`, `collections` → Tags
    /// - `created_at`, `updated_at` → Date
    /// - everything else → Text
    pub fn builtin_type(field: &str) -> PropertyType {
        match field {
            "tags" | "keywords" | "collections" => PropertyType::Tags,
            "created_at" | "updated_at" => PropertyType::Date,
            _ => PropertyType::Text,
        }
    }

    /// Returns `true` if the schema has no user-declared properties.
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }
}

// ── Helper: relaxed date check ───────────────────────────────────────────

/// Quick heuristic: does `s` look like it could be a date?
///
/// Accepts ISO-like patterns: `2026-07-27`, `2026/07/27`, `20260727`,
/// and timestamps like `2026-07-27T10:30:00Z`.
fn looks_like_date(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 8 {
        return false;
    }
    // Must start with a 4-digit year.
    let year_chars: String = s.chars().take(4).collect();
    if year_chars.len() < 4 || !year_chars.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    // Must contain '-' or '/' or be exactly 8 digits.
    if s.len() == 8 && s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    s.contains('-') || s.contains('/')
}

/// Check whether `s` is already in an expected date-like format.
/// This is a stricter check than `looks_like_date` — it rejects strings
/// that happen to contain a hyphen but are clearly not dates.
fn is_date_like(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    // Check known patterns via normalize_date round-trip.
    let normalized = normalize_date(s);
    !normalized.is_empty() && normalized != s
}

// ── Type-aware comparison ────────────────────────────────────────────────

/// Compare two string values according to a [`PropertyType`].
///
/// - **Number**: parse both as `f64` and compare numerically.  If either
///   fails to parse, fall back to lexicographic ordering (so that malformed
///   values still sort deterministically).
/// - **Date**: normalize both to a date key (ISO `YYYY-MM-DD[THH:MM:SS]`)
///   and compare lexicographically on the normalized form.
/// - **Checkbox**: compare boolean truthiness (`true > false`).
/// - **Enum**: case-insensitive lexicographic comparison (option values are
///   compared ignoring case, so "DONE" == "done").
/// - **Text / Tags / Asset / MultiText**: plain lexicographic comparison.
pub fn cmp_typed(ty: PropertyType, a: &str, b: &str) -> std::cmp::Ordering {
    match ty {
        PropertyType::Number => cmp_numbers(a, b),
        PropertyType::Date => {
            let na = normalize_date(a);
            let nb = normalize_date(b);
            na.as_str().cmp(nb.as_str())
        }
        PropertyType::Checkbox => {
            let ba = parse_bool(a).unwrap_or(false);
            let bb = parse_bool(b).unwrap_or(false);
            ba.cmp(&bb)
        }
        PropertyType::Enum => {
            // Case-insensitive comparison for enum values.
            a.to_lowercase().cmp(&b.to_lowercase())
        }
        _ => a.cmp(b),
    }
}

/// Compare two values as numbers.  Falls back to string comparison if either
/// fails to parse.
fn cmp_numbers(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(na), Ok(nb)) => na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.cmp(b),
    }
}

/// Parse a boolean from common string representations.
///
/// Accepts (case-insensitive): `true`, `false`, `yes`, `no`, `1`, `0`,
/// `on`, `off`.  Returns `None` for anything else.
pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

/// Normalize a date string to a lexicographically-comparable form.
///
/// Handles these input patterns:
/// - `2026-07-27` → `2026-07-27`
/// - `2026/07/27` → `2026-07-27`
/// - `2026-7-3`   → `2026-07-03` (zero-padded)
/// - `2026-07-27T10:30:00Z` → `2026-07-27T10:30:00Z`
/// - `2026-07-27T10:30:00+08:00` → `2026-07-27T10:30:00+08:00`
/// - `20260727` → `2026-07-27`
///
/// If the input doesn't match any known pattern, it is returned unchanged
/// (so that mixed normalized/raw values still sort deterministically).
pub fn normalize_date(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }

    // Compact form: YYYYMMDD (8 digits).
    if s.len() == 8 && s.chars().all(|c| c.is_ascii_digit()) {
        return format!("{}-{}-{}", &s[..4], &s[4..6], &s[6..8]);
    }

    // Slash-separated → dash-separated.
    if s.contains('/') {
        let dashified = s.replace('/', "-");
        return zero_pad_date_parts(&dashified);
    }

    // Already dash-separated but possibly without zero-padding.
    if s.contains('-') {
        return zero_pad_date_parts(s);
    }

    // Unknown format — return as-is.
    s.to_string()
}

/// Zero-pad the month and day components of a date string.
///
/// `2026-7-3` → `2026-07-03`, `2026-07-27T10:30:00Z` → unchanged.
fn zero_pad_date_parts(s: &str) -> String {
    // Split off any time/timezone portion after the first 'T' or ' '.
    let (date_part, rest) = if let Some(pos) = s.find(['T', ' '].as_ref()) {
        (&s[..pos], &s[pos..])
    } else {
        (s, "")
    };

    let parts: Vec<&str> = date_part.split('-').collect();
    if parts.len() < 3 {
        return s.to_string();
    }

    // Pad month and day (parts[1] and parts[2]) to 2 digits.
    let padded_month = pad2(parts[1]);
    let padded_day = pad2(parts[2]);

    format!("{}-{}-{}{}", parts[0], padded_month, padded_day, rest)
}

/// Left-pad a numeric string to width 2 with '0'.
fn pad2(s: &str) -> String {
    if s.len() >= 2 {
        s.to_string()
    } else {
        format!("0{s}")
    }
}

/// Check type-aware equality of two values.
///
/// - Number: `parse(a) == parse(b)` (returns `false` if either fails to parse)
/// - Checkbox: `parse_bool(a) == parse_bool(b)`
/// - Enum: case-insensitive string equality
/// - Everything else: string equality
pub fn typed_equals(ty: PropertyType, a: &str, b: &str) -> bool {
    match ty {
        PropertyType::Number => {
            matches!(
                (a.trim().parse::<f64>(), b.trim().parse::<f64>()),
                (Ok(na), Ok(nb)) if (na - nb).abs() < f64::EPSILON
            )
        }
        PropertyType::Checkbox => parse_bool(a) == parse_bool(b) && parse_bool(a).is_some(),
        PropertyType::Date => normalize_date(a) == normalize_date(b),
        PropertyType::Enum => a.eq_ignore_ascii_case(b.trim()),
        _ => a == b,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── PropertyType basics ──

    #[test]
    fn test_property_type_display() {
        assert_eq!(PropertyType::Text.to_string(), "text");
        assert_eq!(PropertyType::Number.to_string(), "number");
        assert_eq!(PropertyType::Date.to_string(), "date");
        assert_eq!(PropertyType::Checkbox.to_string(), "checkbox");
        assert_eq!(PropertyType::Tags.to_string(), "tags");
        assert_eq!(PropertyType::Asset.to_string(), "asset");
        assert_eq!(PropertyType::MultiText.to_string(), "multitext");
        assert_eq!(PropertyType::Enum.to_string(), "enum");
    }

    #[test]
    fn test_property_type_predicates() {
        assert!(PropertyType::Number.is_numeric());
        assert!(!PropertyType::Text.is_numeric());
        assert!(PropertyType::Date.is_date());
        assert!(PropertyType::Tags.is_array());
        assert!(PropertyType::MultiText.is_array());
        assert!(!PropertyType::Text.is_array());
        assert!(PropertyType::Enum.is_enum());
        assert!(!PropertyType::Text.is_enum());
    }

    // ── Schema parsing ──

    #[test]
    fn test_schema_from_yaml() {
        let yaml = r#"
properties:
  priority: number
  due_date: date
  is_done: checkbox
"#;
        let schema = PropertySchema::from_yaml(yaml).unwrap();
        assert_eq!(schema.type_of("priority"), PropertyType::Number);
        assert_eq!(schema.type_of("due_date"), PropertyType::Date);
        assert_eq!(schema.type_of("is_done"), PropertyType::Checkbox);
    }

    #[test]
    fn test_schema_from_yaml_with_enum() {
        let yaml = r#"
properties:
  status: enum
enum_options:
  status: [doing, done, blocked]
"#;
        let schema = PropertySchema::from_yaml(yaml).unwrap();
        assert_eq!(schema.type_of("status"), PropertyType::Enum);
        assert_eq!(
            schema.enum_options_for("status"),
            Some(
                vec![
                    "doing".to_string(),
                    "done".to_string(),
                    "blocked".to_string()
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn test_schema_from_yaml_with_multiple_enums() {
        let yaml = r#"
properties:
  priority: enum
  status: enum
  title: text
enum_options:
  priority: [p0, p1, p2, p3]
  status: [todo, doing, done, blocked]
"#;
        let schema = PropertySchema::from_yaml(yaml).unwrap();
        assert_eq!(schema.type_of("priority"), PropertyType::Enum);
        assert_eq!(schema.type_of("status"), PropertyType::Enum);
        assert_eq!(schema.type_of("title"), PropertyType::Text);
        assert_eq!(
            schema.enum_options_for("priority"),
            Some(
                vec![
                    "p0".to_string(),
                    "p1".to_string(),
                    "p2".to_string(),
                    "p3".to_string()
                ]
                .as_slice()
            )
        );
        assert_eq!(
            schema.enum_options_for("status"),
            Some(
                vec![
                    "todo".to_string(),
                    "doing".to_string(),
                    "done".to_string(),
                    "blocked".to_string()
                ]
                .as_slice()
            )
        );
        assert!(schema.enum_options_for("title").is_none());
    }

    #[test]
    fn test_schema_empty_yaml() {
        let schema = PropertySchema::from_yaml("").unwrap();
        assert!(schema.is_empty());
        assert_eq!(schema.type_of("anything"), PropertyType::Text);
    }

    #[test]
    fn test_schema_builtin_defaults() {
        let schema = PropertySchema::empty();
        assert_eq!(schema.type_of("tags"), PropertyType::Tags);
        assert_eq!(schema.type_of("keywords"), PropertyType::Tags);
        assert_eq!(schema.type_of("collections"), PropertyType::Tags);
        assert_eq!(schema.type_of("created_at"), PropertyType::Date);
        assert_eq!(schema.type_of("updated_at"), PropertyType::Date);
        assert_eq!(schema.type_of("title"), PropertyType::Text);
    }

    #[test]
    fn test_schema_builder() {
        let schema = PropertySchema::empty()
            .with("priority", PropertyType::Number)
            .with("due_date", PropertyType::Date);
        assert_eq!(schema.type_of("priority"), PropertyType::Number);
        assert_eq!(schema.type_of("due_date"), PropertyType::Date);
        assert_eq!(schema.type_of("unknown"), PropertyType::Text);
    }

    #[test]
    fn test_schema_builder_with_enum() {
        let schema = PropertySchema::empty().with_enum(
            "status",
            vec!["todo".into(), "done".into(), "blocked".into()],
        );
        assert_eq!(schema.type_of("status"), PropertyType::Enum);
        assert_eq!(
            schema.enum_options_for("status"),
            Some(
                vec![
                    "todo".to_string(),
                    "done".to_string(),
                    "blocked".to_string()
                ]
                .as_slice()
            )
        );
    }

    // ── Value validation ──

    #[test]
    fn test_validate_number_ok() {
        let schema = PropertySchema::empty().with("score", PropertyType::Number);
        assert!(schema.validate_value("score", "42").is_none());
        assert!(schema.validate_value("score", "3.14").is_none());
        assert!(schema.validate_value("score", "-7").is_none());
        assert!(schema.validate_value("score", "0").is_none());
        // Empty is always accepted
        assert!(schema.validate_value("score", "").is_none());
    }

    #[test]
    fn test_validate_number_fail() {
        let schema = PropertySchema::empty().with("score", PropertyType::Number);
        assert!(schema.validate_value("score", "abc").is_some());
        assert!(schema.validate_value("score", "12a").is_some());
    }

    #[test]
    fn test_validate_checkbox_ok() {
        let schema = PropertySchema::empty().with("done", PropertyType::Checkbox);
        assert!(schema.validate_value("done", "true").is_none());
        assert!(schema.validate_value("done", "false").is_none());
        assert!(schema.validate_value("done", "yes").is_none());
        assert!(schema.validate_value("done", "no").is_none());
        assert!(schema.validate_value("done", "1").is_none());
        assert!(schema.validate_value("done", "0").is_none());
        assert!(schema.validate_value("done", "on").is_none());
        assert!(schema.validate_value("done", "off").is_none());
        assert!(schema.validate_value("done", "").is_none());
    }

    #[test]
    fn test_validate_checkbox_fail() {
        let schema = PropertySchema::empty().with("done", PropertyType::Checkbox);
        assert!(schema.validate_value("done", "maybe").is_some());
        assert!(schema.validate_value("done", "2").is_some());
    }

    #[test]
    fn test_validate_enum_ok() {
        let schema = PropertySchema::empty()
            .with_enum("status", vec!["todo".into(), "doing".into(), "done".into()]);
        assert!(schema.validate_value("status", "todo").is_none());
        assert!(schema.validate_value("status", "doing").is_none());
        assert!(schema.validate_value("status", "done").is_none());
        // Case-insensitive match
        assert!(schema.validate_value("status", "TODO").is_none());
        assert!(schema.validate_value("status", "Doing").is_none());
        // Empty is always accepted
        assert!(schema.validate_value("status", "").is_none());
    }

    #[test]
    fn test_validate_enum_fail() {
        let schema = PropertySchema::empty()
            .with_enum("status", vec!["todo".into(), "doing".into(), "done".into()]);
        assert!(schema.validate_value("status", "invalid").is_some());
        assert!(schema.validate_value("status", "in_progress").is_some());
    }

    #[test]
    fn test_validate_enum_no_options_accepts_any() {
        // Enum without options defined should accept any value (treated as free-text).
        let schema = PropertySchema::empty().with("status", PropertyType::Enum);
        assert!(schema.validate_value("status", "anything").is_none());
        assert!(schema.validate_value("status", "goes").is_none());
    }

    #[test]
    fn test_validate_date_ok() {
        let schema = PropertySchema::empty().with("created", PropertyType::Date);
        assert!(schema.validate_value("created", "2026-07-27").is_none());
        assert!(schema.validate_value("created", "2026/07/27").is_none());
        assert!(schema.validate_value("created", "20260727").is_none());
        assert!(schema.validate_value("created", "2026-7-3").is_none());
        assert!(schema.validate_value("created", "").is_none());
    }

    #[test]
    fn test_validate_date_suspicious() {
        let schema = PropertySchema::empty().with("created", PropertyType::Date);
        // Clearly not a date
        assert!(schema.validate_value("created", "hello").is_some());
        assert!(schema.validate_value("created", "abc-def-ghi").is_some());
    }

    // Regression for #3583: verify that removing the dead
    // `normalized.is_empty()` branch doesn't change behavior —
    // non-empty input that normalizes to a recognizable date is accepted.
    #[test]
    fn test_validate_date_nonempty_branch_coverage_3583() {
        let schema = PropertySchema::empty().with("created", PropertyType::Date);
        // Non-empty input with whitespace should still be processed
        assert!(schema.validate_value("created", "  2026-07-27  ").is_none());
        // Non-empty input that isn't date-like is rejected (dead branch removal
        // must not affect this)
        assert!(schema.validate_value("created", "12345").is_some());
        // Non-empty input that is a valid date is accepted
        assert!(schema.validate_value("created", "2026-07-27").is_none());
    }

    // ── validate_note_meta ──

    #[test]
    fn test_validate_note_meta_no_warnings() {
        let schema = PropertySchema::empty()
            .with_enum("status", vec!["active".into(), "archived".into()])
            .with("priority", PropertyType::Number);
        let meta = NoteMeta {
            status: "active".into(),
            ..Default::default()
        };
        assert!(schema.validate_note_meta(&meta).is_empty());
    }

    #[test]
    fn test_validate_note_meta_with_warnings() {
        let schema = PropertySchema::empty()
            .with_enum("status", vec!["active".into(), "archived".into()])
            .with("summary", PropertyType::Number);
        let meta = NoteMeta {
            status: "invalid_status".into(),
            summary: "not_a_number".into(),
            ..Default::default()
        };
        let warnings = schema.validate_note_meta(&meta);
        assert_eq!(warnings.len(), 2, "should have 2 warnings: {warnings:?}");
    }

    #[test]
    fn test_validate_note_meta_array_fields() {
        let schema = PropertySchema::empty().with_enum(
            "tags",
            vec!["rust".into(), "typescript".into(), "docs".into()],
        );
        let meta = NoteMeta {
            tags: vec!["rust".into(), "invalid_tag".into()],
            ..Default::default()
        };
        let warnings = schema.validate_note_meta(&meta);
        assert_eq!(
            warnings.len(),
            1,
            "invalid tag should produce 1 warning: {warnings:?}"
        );
    }

    #[test]
    fn test_validate_note_meta_skips_undeclared_fields() {
        // Fields not in the schema should not be validated.
        let schema = PropertySchema::empty().with("status", PropertyType::Number); // only status is declared
        let meta = NoteMeta {
            status: "42".into(),
            title: "anything".into(), // not in schema
            ..Default::default()
        };
        assert!(schema.validate_note_meta(&meta).is_empty());
    }

    #[test]
    fn test_validate_note_meta_accepts_blank_fields() {
        let schema = PropertySchema::empty()
            .with_enum("status", vec!["active".into(), "archived".into()])
            .with("priority", PropertyType::Number);
        let meta = NoteMeta::default(); // all fields empty
        assert!(schema.validate_note_meta(&meta).is_empty());
    }

    // ── Numeric comparison ──

    #[test]
    fn test_cmp_numbers() {
        assert_eq!(
            cmp_typed(PropertyType::Number, "10", "2"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_typed(PropertyType::Number, "2", "10"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            cmp_typed(PropertyType::Number, "3.14", "2.71"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_typed(PropertyType::Number, "5", "5"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_cmp_numbers_negative() {
        assert_eq!(
            cmp_typed(PropertyType::Number, "-5", "3"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            cmp_typed(PropertyType::Number, "-10", "-2"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_cmp_numbers_fallback_on_parse_error() {
        // Non-numeric values fall back to lexicographic
        assert_eq!(
            cmp_typed(PropertyType::Number, "abc", "def"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_cmp_numbers_with_whitespace() {
        assert_eq!(
            cmp_typed(PropertyType::Number, " 42 ", " 7 "),
            std::cmp::Ordering::Greater
        );
    }

    // ── Date normalization ──

    #[test]
    fn test_normalize_date_iso() {
        assert_eq!(normalize_date("2026-07-27"), "2026-07-27");
    }

    #[test]
    fn test_normalize_date_slash() {
        assert_eq!(normalize_date("2026/07/27"), "2026-07-27");
    }

    #[test]
    fn test_normalize_date_unpadded() {
        assert_eq!(normalize_date("2026-7-3"), "2026-07-03");
    }

    #[test]
    fn test_normalize_date_compact() {
        assert_eq!(normalize_date("20260727"), "2026-07-27");
    }

    #[test]
    fn test_normalize_date_with_time() {
        assert_eq!(
            normalize_date("2026-07-27T10:30:00Z"),
            "2026-07-27T10:30:00Z"
        );
        assert_eq!(normalize_date("2026-7-3T10:30:00Z"), "2026-07-03T10:30:00Z");
    }

    #[test]
    fn test_normalize_date_empty() {
        assert_eq!(normalize_date(""), "");
        assert_eq!(normalize_date("  "), "");
    }

    // ── Date comparison ──

    #[test]
    fn test_cmp_dates() {
        assert_eq!(
            cmp_typed(PropertyType::Date, "2026-07-27", "2026-07-28"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            cmp_typed(PropertyType::Date, "2026-07-28", "2026-07-27"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_typed(PropertyType::Date, "2026-07-27", "2026-07-27"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_cmp_dates_unpadded() {
        // "2026-7-3" should normalize to "2026-07-03" and compare correctly
        assert_eq!(
            cmp_typed(PropertyType::Date, "2026-7-3", "2026-7-20"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_cmp_dates_mixed_formats() {
        assert_eq!(
            cmp_typed(PropertyType::Date, "2026/07/27", "2026-07-27"),
            std::cmp::Ordering::Equal
        );
    }

    // ── Checkbox comparison ──

    #[test]
    fn test_cmp_checkbox() {
        assert_eq!(
            cmp_typed(PropertyType::Checkbox, "true", "false"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_typed(PropertyType::Checkbox, "yes", "no"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_typed(PropertyType::Checkbox, "1", "0"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_typed(PropertyType::Checkbox, "true", "true"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_parse_bool_variants() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("True"), Some(true));
        assert_eq!(parse_bool("TRUE"), Some(true));
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("YES"), Some(true));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("on"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("no"), Some(false));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("off"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
        assert_eq!(parse_bool(""), None);
    }

    // ── Enum comparison ──

    #[test]
    fn test_cmp_enum_case_insensitive() {
        // Enum comparison should be case-insensitive
        assert_eq!(
            cmp_typed(PropertyType::Enum, "TODO", "todo"),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            cmp_typed(PropertyType::Enum, "DONE", "doing"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn test_typed_equals_enum_case_insensitive() {
        assert!(typed_equals(PropertyType::Enum, "TODO", "todo"));
        assert!(typed_equals(PropertyType::Enum, "DONE", "done"));
        assert!(!typed_equals(PropertyType::Enum, "todo", "doing"));
    }

    // ── Text comparison (unchanged behaviour) ──

    #[test]
    fn test_cmp_text_unchanged() {
        assert_eq!(
            cmp_typed(PropertyType::Text, "apple", "banana"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            cmp_typed(PropertyType::Text, "10", "2"),
            std::cmp::Ordering::Less
        ); // lexicographic
    }

    // ── typed_equals ──

    #[test]
    fn test_typed_equals_number() {
        assert!(typed_equals(PropertyType::Number, "42", "42"));
        assert!(typed_equals(PropertyType::Number, "42", "42.0"));
        assert!(!typed_equals(PropertyType::Number, "42", "43"));
    }

    #[test]
    fn test_typed_equals_checkbox() {
        assert!(typed_equals(PropertyType::Checkbox, "true", "yes"));
        assert!(typed_equals(PropertyType::Checkbox, "false", "no"));
        assert!(!typed_equals(PropertyType::Checkbox, "true", "false"));
    }

    #[test]
    fn test_typed_equals_date() {
        assert!(typed_equals(PropertyType::Date, "2026-07-27", "2026/07/27"));
        assert!(!typed_equals(
            PropertyType::Date,
            "2026-07-27",
            "2026-07-28"
        ));
    }

    // ── Integration: schema-driven comparison ──

    #[test]
    fn test_schema_driven_comparison() {
        let schema = PropertySchema::empty()
            .with("priority", PropertyType::Number)
            .with("due_date", PropertyType::Date);

        // Numeric: 10 > 2 (not lexicographic "10" < "2")
        let ty = schema.type_of("priority");
        assert_eq!(cmp_typed(ty, "10", "2"), std::cmp::Ordering::Greater);

        // Date: normalized comparison
        let ty = schema.type_of("due_date");
        assert_eq!(
            cmp_typed(ty, "2026/07/27", "2026-07-28"),
            std::cmp::Ordering::Less
        );

        // Untyped field: lexicographic (backward compatible)
        let ty = schema.type_of("title");
        assert_eq!(cmp_typed(ty, "10", "2"), std::cmp::Ordering::Less);
    }

    // ── Regression test for #3501 ──

    #[test]
    fn test_regression_3501_numeric_sort() {
        // Before #3501: "10" < "2" lexicographically (wrong)
        // After #3501: 10 > 2 numerically (correct)
        assert_eq!(
            cmp_typed(PropertyType::Number, "10", "2"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_typed(PropertyType::Number, "100", "20"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_typed(PropertyType::Number, "9", "10"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_regression_3501_date_normalization() {
        // Before #3501: "2026-7-3" vs "2026-07-20" → lexicographic "2026-7-3" > "2026-07-20"
        // After #3501: normalized "2026-07-03" < "2026-07-20"
        assert_eq!(
            cmp_typed(PropertyType::Date, "2026-7-3", "2026-07-20"),
            std::cmp::Ordering::Less
        );
    }
}
