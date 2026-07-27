//! Property Type System — typed frontmatter fields (#3501).
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
        };
        write!(f, "{s}")
    }
}

// ── PropertySchema ───────────────────────────────────────────────────────

/// A vault-level declaration of custom frontmatter field types.
///
/// Loaded from `.vp/property-schema.yml`.  Fields not present in the schema
/// get a default type determined by [`PropertySchema::builtin_type`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PropertySchema {
    /// User-declared field → type mappings.
    #[serde(default)]
    pub properties: HashMap<String, PropertyType>,
}

impl PropertySchema {
    /// Create an empty schema (all fields default to Text/builtins).
    pub fn empty() -> Self {
        Self {
            properties: HashMap::new(),
        }
    }

    /// Parse a property schema from a YAML string.
    ///
    /// Expected format:
    /// ```yaml
    /// properties:
    ///   priority: number
    ///   due_date: date
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

    /// Get the declared type for a field, falling back to built-in defaults.
    pub fn type_of(&self, field: &str) -> PropertyType {
        if let Some(ty) = self.properties.get(field) {
            return *ty;
        }
        Self::builtin_type(field)
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

// ── Type-aware comparison ────────────────────────────────────────────────

/// Compare two string values according to a [`PropertyType`].
///
/// - **Number**: parse both as `f64` and compare numerically.  If either
///   fails to parse, fall back to lexicographic ordering (so that malformed
///   values still sort deterministically).
/// - **Date**: normalize both to a date key (ISO `YYYY-MM-DD[THH:MM:SS]`)
///   and compare lexicographically on the normalized form.
/// - **Checkbox**: compare boolean truthiness (`true > false`).
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
    }

    #[test]
    fn test_property_type_predicates() {
        assert!(PropertyType::Number.is_numeric());
        assert!(!PropertyType::Text.is_numeric());
        assert!(PropertyType::Date.is_date());
        assert!(PropertyType::Tags.is_array());
        assert!(PropertyType::MultiText.is_array());
        assert!(!PropertyType::Text.is_array());
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
