//! Declarative settings schema (#2872).
//!
//! Inspired by Obsidian v1.13.0's declarative Settings API, this module lets
//! the Rust backend describe every user-facing setting as structured data
//! instead of hard-coding UI controls. The WinUI frontend can then *discover*
//! settings at runtime, render them dynamically, and expose global search over
//! their labels/descriptions — without the backend and frontend having to be
//! edited in lock-step every time a setting is added.
//!
//! Each [`SettingDefinition`] carries its type, default value, validation
//! constraints and an optional [`SettingVisibility`] predicate. The predicate
//! is serialized as data (not code), so the frontend can evaluate it against
//! the current settings to decide whether a control should be shown — exactly
//! like Obsidian's `visible` closures, but portable across the process
//! boundary.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::AppSettings;

/// Top-level grouping used to build collapsible category panels in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettingCategory {
    General,
    Provider,
    AutoWake,
    Context,
    ModelRouting,
    Sessions,
    Appearance,
}

/// The data type of a single setting, including constraints that drive both
/// validation and which UI control the frontend renders.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SettingType {
    /// Single-line free text.
    Text,
    /// Multi-line free text.
    TextArea,
    /// A filesystem path (the UI may offer a picker).
    Path,
    /// On/off toggle.
    Boolean,
    /// Numeric input with optional bounds.
    Number {
        #[serde(skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
    },
    /// Dropdown with a fixed set of string options.
    Select { options: Vec<String> },
}

/// A serializable visibility predicate evaluated against the current settings.
///
/// Mirrors Obsidian's `visible` closures but as portable data so the WinUI
/// frontend can decide whether to render a control without bespoke logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum SettingVisibility {
    /// Always shown.
    Always,
    /// Shown only when the named field (dotted camelCase path) equals `value`.
    FieldEquals { field: String, value: Value },
    /// Shown only when the named field does *not* equal `value`.
    FieldNotEquals { field: String, value: Value },
    /// Shown only when the named boolean field is `true`.
    FieldTruthy { field: String },
    /// Shown only when the named boolean field is `false` (or absent).
    FieldFalsy { field: String },
}

impl SettingVisibility {
    /// Evaluate this predicate against the current settings.
    pub fn is_visible(&self, settings: &AppSettings) -> bool {
        match self {
            SettingVisibility::Always => true,
            SettingVisibility::FieldEquals { field, value } => {
                get_field_value(settings, field) == Some(value.clone())
            }
            SettingVisibility::FieldNotEquals { field, value } => {
                get_field_value(settings, field) != Some(value.clone())
            }
            SettingVisibility::FieldTruthy { field } => {
                matches!(get_field_value(settings, field), Some(Value::Bool(true)))
            }
            SettingVisibility::FieldFalsy { field } => {
                !matches!(get_field_value(settings, field), Some(Value::Bool(true)))
            }
        }
    }
}

/// A declarative description of a single settings control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingDefinition {
    /// Dotted camelCase path identifying the field, e.g. `"provider.model"`.
    /// The frontend uses this key both to read the current value and to patch
    /// it back via the existing settings-save endpoint.
    pub key: String,
    /// Human-readable label shown in the UI.
    pub label: String,
    /// Longer description (also indexed for search).
    pub description: String,
    /// Category used for grouping.
    pub category: SettingCategory,
    /// Data type / control kind.
    #[serde(rename = "type")]
    pub setting_type: SettingType,
    /// Default value (derived from [`AppSettings::default`]).
    pub default: Value,
    /// Optional placeholder text for text/textarea controls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Optional visibility predicate; when absent the control is always shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<SettingVisibility>,
}

/// Resolve a dotted camelCase path (e.g. `"provider.baseUrl"`) against the
/// serialized form of `settings`. Returns `None` if any segment is missing.
fn get_field_value(settings: &AppSettings, path: &str) -> Option<Value> {
    let root = serde_json::to_value(settings).ok()?;
    let mut cur = &root;
    for segment in path.split('.') {
        cur = cur.get(segment)?;
    }
    Some(cur.clone())
}

/// Fluent builder used by [`collect_setting_definitions`] to keep the catalog
/// readable while still deriving each default from [`AppSettings::default`].
struct DefBuilder {
    key: &'static str,
    label: &'static str,
    description: &'static str,
    category: SettingCategory,
    setting_type: SettingType,
    placeholder: Option<&'static str>,
    visible_when: Option<SettingVisibility>,
    /// Optional override for the derived default. When set, this value is used
    /// instead of looking up the key in `AppSettings::default()`. Useful for
    /// `Option` fields where `None` serialises as `null` but the schema expects
    /// a concrete default (e.g. Number: 0, Text: \"\").
    default_override: Option<Value>,
}

impl DefBuilder {
    fn new(
        key: &'static str,
        label: &'static str,
        description: &'static str,
        category: SettingCategory,
        setting_type: SettingType,
    ) -> Self {
        Self {
            key,
            label,
            description,
            category,
            setting_type,
            placeholder: None,
            visible_when: None,
            default_override: None,
        }
    }

    fn placeholder(mut self, p: &'static str) -> Self {
        self.placeholder = Some(p);
        self
    }

    fn visible_when(mut self, v: SettingVisibility) -> Self {
        self.visible_when = Some(v);
        self
    }

    /// Override the derived default value. When not called, the default is
    /// automatically derived from `AppSettings::default()` by looking up the
    /// dot-separated key path.
    fn default_value(mut self, v: Value) -> Self {
        self.default_override = Some(v);
        self
    }

    fn build(self, defaults: &AppSettings) -> SettingDefinition {
        let default = self
            .default_override
            .unwrap_or_else(|| get_field_value(defaults, self.key).unwrap_or(Value::Null));
        SettingDefinition {
            key: self.key.to_string(),
            label: self.label.to_string(),
            description: self.description.to_string(),
            category: self.category,
            setting_type: self.setting_type,
            default,
            placeholder: self.placeholder.map(String::from),
            visible_when: self.visible_when,
        }
    }
}

/// Build the full catalog of declarative setting definitions for the current
/// schema. Add new entries here whenever a field is added to [`AppSettings`];
/// the WinUI UI will pick them up automatically.
pub fn collect_setting_definitions() -> Vec<SettingDefinition> {
    let defaults = AppSettings::default();

    macro_rules! def {
        ($key:literal, $label:literal, $desc:literal, $cat:expr, $ty:expr) => {
            DefBuilder::new($key, $label, $desc, $cat, $ty)
        };
    }

    let truthy = |field: &'static str| SettingVisibility::FieldTruthy {
        field: field.into(),
    };

    vec![
        // ----- General -----
        def!(
            "vaultDir",
            "Vault directory",
            "Root folder of your markdown vault.",
            SettingCategory::General,
            SettingType::Path
        )
        .placeholder("~/Documents/MyVault")
        .build(&defaults),
        def!(
            "systemDirective",
            "Global system directive",
            "Persistent instruction prepended to every chat (tone, format, focus).",
            SettingCategory::General,
            SettingType::TextArea
        )
        .build(&defaults),
        def!(
            "responseStyle",
            "Response style",
            "Controls answer length and depth.",
            SettingCategory::General,
            SettingType::Select {
                options: vec![
                    "brief".to_string(),
                    "standard".to_string(),
                    "detailed".to_string(),
                ]
            }
        )
        .build(&defaults),
        def!(
            "autoCheckUpdates",
            "Automatically check for updates",
            "Periodically check whether a newer VaultPilot build is available.",
            SettingCategory::General,
            SettingType::Boolean
        )
        .build(&defaults),
        // ----- Provider -----
        def!(
            "provider.baseUrl",
            "Provider base URL",
            "API base URL for the active provider (e.g. https://api.openai.com/v1).",
            SettingCategory::Provider,
            SettingType::Text
        )
        .placeholder("https://api.openai.com/v1")
        .build(&defaults),
        def!(
            "provider.model",
            "Model",
            "Model name used for completions.",
            SettingCategory::Provider,
            SettingType::Text
        )
        .build(&defaults),
        def!(
            "provider.requestTimeoutMs",
            "Request timeout (ms)",
            "Maximum time to wait for a provider response.",
            SettingCategory::Provider,
            SettingType::Number {
                min: Some(1000.0),
                max: Some(600_000.0),
                step: Some(1000.0)
            }
        )
        .build(&defaults),
        def!(
            "provider.contextWindowTokens",
            "Context window (tokens)",
            "Token capacity of the model. Empty/0 means 'unknown' (no hard cap).",
            SettingCategory::Provider,
            SettingType::Number {
                min: Some(0.0),
                max: Some(2_000_000.0),
                step: Some(512.0)
            }
        )
        .default_value(serde_json::json!(0))
        .build(&defaults),
        def!(
            "activeProviderIndex",
            "Active provider index",
            "Index into the configured provider list.",
            SettingCategory::Provider,
            SettingType::Number {
                min: Some(0.0),
                max: Some(64.0),
                step: Some(1.0)
            }
        )
        .build(&defaults),
        // ----- Auto-wake -----
        def!(
            "autoWakeEnabled",
            "Enable auto-wake",
            "Let VaultPilot proactively run on a schedule.",
            SettingCategory::AutoWake,
            SettingType::Boolean
        )
        .build(&defaults),
        def!(
            "autoWakeIntervalMinutes",
            "Auto-wake interval (minutes)",
            "How often auto-wake fires.",
            SettingCategory::AutoWake,
            SettingType::Number {
                min: Some(1.0),
                max: Some(1440.0),
                step: Some(1.0)
            }
        )
        .visible_when(truthy("autoWakeEnabled"))
        .build(&defaults),
        def!(
            "autoWakeModel",
            "Auto-wake model",
            "Model used for scheduled auto-wake runs.",
            SettingCategory::AutoWake,
            SettingType::Text
        )
        .visible_when(truthy("autoWakeEnabled"))
        .build(&defaults),
        def!(
            "autoWakeStartTime",
            "Auto-wake start time",
            "HH:MM local time when auto-wake may begin.",
            SettingCategory::AutoWake,
            SettingType::Text
        )
        .visible_when(truthy("autoWakeEnabled"))
        .build(&defaults),
        def!(
            "autoWakeEndTime",
            "Auto-wake end time",
            "HH:MM local time when auto-wake must stop.",
            SettingCategory::AutoWake,
            SettingType::Text
        )
        .visible_when(truthy("autoWakeEnabled"))
        .build(&defaults),
        def!(
            "autoWakePrompt",
            "Auto-wake prompt",
            "Instruction executed each time auto-wake fires.",
            SettingCategory::AutoWake,
            SettingType::TextArea
        )
        .visible_when(truthy("autoWakeEnabled"))
        .build(&defaults),
        // ----- Context -----
        def!(
            "contextCompression",
            "Automatic context compression",
            "Summarize old conversation history once token usage is high.",
            SettingCategory::Context,
            SettingType::Boolean
        )
        .build(&defaults),
        def!(
            "compressionThreshold",
            "Compression threshold",
            "Fraction of the context window at which compression triggers (0.1–1.0).",
            SettingCategory::Context,
            SettingType::Number {
                min: Some(0.1),
                max: Some(1.0),
                step: Some(0.05)
            }
        )
        .visible_when(truthy("contextCompression"))
        .build(&defaults),
        // ----- Model routing -----
        def!(
            "modelRouting.enabled",
            "Enable model routing",
            "Route each request to a task-specific model instead of one model for all.",
            SettingCategory::ModelRouting,
            SettingType::Boolean
        )
        .build(&defaults),
        def!(
            "modelRouting.simpleTaskModel",
            "Simple-task model",
            "Model for short Q&A, translation and summaries. Empty = default.",
            SettingCategory::ModelRouting,
            SettingType::Text
        )
        .visible_when(truthy("modelRouting.enabled"))
        .default_value(serde_json::json!(""))
        .build(&defaults),
        def!(
            "modelRouting.complexTaskModel",
            "Complex-task model",
            "Model for long-form analysis and multi-step reasoning. Empty = default.",
            SettingCategory::ModelRouting,
            SettingType::Text
        )
        .visible_when(truthy("modelRouting.enabled"))
        .default_value(serde_json::json!(""))
        .build(&defaults),
        def!(
            "modelRouting.codeTaskModel",
            "Code-task model",
            "Model for code-related requests. Empty = default.",
            SettingCategory::ModelRouting,
            SettingType::Text
        )
        .visible_when(truthy("modelRouting.enabled"))
        .default_value(serde_json::json!(""))
        .build(&defaults),
        // ----- Sessions -----
        def!(
            "sessionExportEnabled",
            "Export sessions to vault",
            "Write chat history as markdown files into the vault after each save.",
            SettingCategory::Sessions,
            SettingType::Boolean
        )
        .build(&defaults),
        def!(
            "sessionExportPath",
            "Session export path",
            "Relative path inside the vault for exported sessions.",
            SettingCategory::Sessions,
            SettingType::Text
        )
        .visible_when(truthy("sessionExportEnabled"))
        .default_value(serde_json::json!(""))
        .build(&defaults),
    ]
}

/// Validate a candidate value against a definition's type and constraints.
/// Returns `Ok(())` when valid, or a human-readable error otherwise. This is
/// the single source of truth the frontend can mirror to give instant
/// feedback before a save round-trip.
pub fn validate_value(def: &SettingDefinition, value: &Value) -> Result<(), String> {
    match &def.setting_type {
        SettingType::Boolean => {
            if !value.is_boolean() {
                return Err(format!("'{}' must be a boolean", def.key));
            }
        }
        SettingType::Text | SettingType::TextArea | SettingType::Path => {
            if !value.is_string() {
                return Err(format!("'{}' must be a string", def.key));
            }
        }
        SettingType::Number { min, max, .. } => {
            let n = value
                .as_f64()
                .ok_or_else(|| format!("'{}' must be a number", def.key))?;
            if let Some(lo) = min {
                if n < *lo {
                    return Err(format!("'{}' must be >= {lo}", def.key));
                }
            }
            if let Some(hi) = max {
                if n > *hi {
                    return Err(format!("'{}' must be <= {hi}", def.key));
                }
            }
        }
        SettingType::Select { options } => {
            let s = value
                .as_str()
                .ok_or_else(|| format!("'{}' must be a string", def.key))?;
            if !options.iter().any(|o| o == s) {
                return Err(format!(
                    "'{}' must be one of: {}",
                    def.key,
                    options.join(", ")
                ));
            }
        }
    }
    Ok(())
}

/// Return only the definitions whose visibility predicate passes for the given
/// settings. Useful for server-side filtering or for tests.
pub fn visible_definitions<'a>(
    defs: &'a [SettingDefinition],
    settings: &AppSettings,
) -> Vec<&'a SettingDefinition> {
    defs.iter()
        .filter(|d| {
            d.visible_when
                .as_ref()
                .is_none_or(|v| v.is_visible(settings))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def_by_key<'a>(defs: &'a [SettingDefinition], key: &str) -> &'a SettingDefinition {
        defs.iter()
            .find(|d| d.key == key)
            .expect("definition present")
    }

    #[test]
    fn catalog_is_non_empty_and_has_expected_keys() {
        let defs = collect_setting_definitions();
        assert!(!defs.is_empty());
        for key in [
            "vaultDir",
            "provider.model",
            "provider.baseUrl",
            "autoWakeEnabled",
            "contextCompression",
            "modelRouting.enabled",
            "sessionExportEnabled",
            "responseStyle",
        ] {
            assert!(
                defs.iter().any(|d| d.key == key),
                "missing definition for {key}"
            );
        }
    }

    #[test]
    fn defaults_are_derived_from_appsettings_default() {
        let defs = collect_setting_definitions();
        // provider.model default should match AppSettings::default().provider.model
        let model = def_by_key(&defs, "provider.model");
        let expected = serde_json::to_value(AppSettings::default())
            .unwrap()
            .get("provider")
            .unwrap()
            .get("model")
            .cloned()
            .unwrap();
        assert_eq!(model.default, expected);
        // responseStyle is a select with the three known options.
        let style = def_by_key(&defs, "responseStyle");
        match &style.setting_type {
            SettingType::Select { options } => {
                assert_eq!(
                    options,
                    &vec![
                        "brief".to_string(),
                        "standard".to_string(),
                        "detailed".to_string()
                    ]
                );
            }
            other => panic!("responseStyle should be Select, got {other:?}"),
        }
    }

    #[test]
    fn every_select_default_is_in_options() {
        let defs = collect_setting_definitions();
        for def in &defs {
            if let SettingType::Select { options } = &def.setting_type {
                let default_str = def.default.as_str().unwrap_or("");
                assert!(
                    options.contains(&default_str.to_string()),
                    "Select '{}' default '{}' is not in options {:?}",
                    def.key,
                    default_str,
                    options
                );
            }
        }
    }

    /// Regression: every definition's derived default must pass validate_value.
    /// This catches Option fields that serialise as `null` but are typed
    /// Number/Text/Select — the frontend would fail to set the default back.
    /// Fix: use `.default_value(...)` on the definition to provide a valid default.
    #[test]
    fn every_default_passes_validate_value() {
        let defs = collect_setting_definitions();
        for def in &defs {
            let result = validate_value(def, &def.default);
            assert!(
                result.is_ok(),
                "Default for '{}' ({:?}) fails validate_value: {}",
                def.key,
                def.default,
                result.unwrap_err()
            );
        }
    }

    #[test]
    fn dotted_path_resolution_matches_nested_fields() {
        let s = AppSettings::default();
        let v = get_field_value(&s, "provider.baseUrl").unwrap();
        assert_eq!(v, json!(s.provider.base_url));
        let missing = get_field_value(&s, "provider.doesNotExist");
        assert!(missing.is_none());
    }

    #[test]
    fn conditional_settings_hidden_until_parent_enabled() {
        let defs = collect_setting_definitions();
        let settings_off = AppSettings::default(); // autoWakeEnabled = false
        let settings_on = AppSettings {
            auto_wake_enabled: true,
            ..AppSettings::default()
        };
        let interval_off = def_by_key(&defs, "autoWakeIntervalMinutes");
        assert!(!interval_off
            .visible_when
            .as_ref()
            .unwrap()
            .is_visible(&settings_off));
        assert!(interval_off
            .visible_when
            .as_ref()
            .unwrap()
            .is_visible(&settings_on));

        // When autoWake is disabled, the interval definition should be filtered out.
        let visible_off = visible_definitions(&defs, &settings_off);
        assert!(!visible_off
            .iter()
            .any(|d| d.key == "autoWakeIntervalMinutes"));
        let visible_on = visible_definitions(&defs, &settings_on);
        assert!(visible_on
            .iter()
            .any(|d| d.key == "autoWakeIntervalMinutes"));
    }

    #[test]
    fn model_routing_children_follow_parent() {
        let defs = collect_setting_definitions();
        let off = AppSettings::default();
        let on = AppSettings {
            model_routing: crate::models::ModelRoutingConfig {
                enabled: true,
                ..Default::default()
            },
            ..AppSettings::default()
        };
        assert!(!visible_definitions(&defs, &off)
            .iter()
            .any(|d| d.key == "modelRouting.simpleTaskModel"));
        assert!(visible_definitions(&defs, &on)
            .iter()
            .any(|d| d.key == "modelRouting.simpleTaskModel"));
    }

    #[test]
    fn validate_value_enforces_number_bounds() {
        let defs = collect_setting_definitions();
        let timeout = def_by_key(&defs, "provider.requestTimeoutMs");
        assert!(validate_value(timeout, &json!(30_000)).is_ok());
        assert!(validate_value(timeout, &json!(500)).is_err()); // below min
        assert!(validate_value(timeout, &json!(1_000_000)).is_err()); // above max
        assert!(validate_value(timeout, &json!("nope")).is_err()); // not a number
    }

    #[test]
    fn validate_value_enforces_select_options() {
        let defs = collect_setting_definitions();
        let style = def_by_key(&defs, "responseStyle");
        assert!(validate_value(style, &json!("standard")).is_ok());
        assert!(validate_value(style, &json!("verbose")).is_err());
    }

    #[test]
    fn validate_value_enforces_boolean() {
        let defs = collect_setting_definitions();
        let aw = def_by_key(&defs, "autoWakeEnabled");
        assert!(validate_value(aw, &json!(true)).is_ok());
        assert!(validate_value(aw, &json!("yes")).is_err());
    }

    #[test]
    fn definitions_serialize_round_trip() {
        let defs = collect_setting_definitions();
        let v = serde_json::to_value(&defs).unwrap();
        let back: Vec<SettingDefinition> = serde_json::from_value(v).unwrap();
        assert_eq!(back.len(), defs.len());
        assert_eq!(back[0].key, defs[0].key);
    }

    #[test]
    fn visibility_predicates_serialize_as_tagged_data() {
        let vis = SettingVisibility::FieldTruthy {
            field: "autoWakeEnabled".into(),
        };
        let v = serde_json::to_value(&vis).unwrap();
        assert_eq!(v.get("op").unwrap(), &json!("fieldTruthy"));
        assert_eq!(v.get("field").unwrap(), &json!("autoWakeEnabled"));
    }

    #[test]
    fn placeholder_is_serialized_when_present() {
        let defs = collect_setting_definitions();
        let vault = def_by_key(&defs, "vaultDir");
        assert_eq!(vault.placeholder.as_deref(), Some("~/Documents/MyVault"));
    }
}
