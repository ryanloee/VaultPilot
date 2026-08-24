// Regression test for #3547: Smart Paste setting — round-trips through
// serde JSON and defaults to true (enabled by default).
use crate::models::settings::AppSettings;

#[test]
fn smart_paste_setting_defaults_to_true() {
    let settings = AppSettings::default();
    assert!(
        settings.smart_paste_enabled,
        "Smart Paste must be enabled by default (issue #3547)"
    );
}

#[test]
fn smart_paste_setting_round_trips_through_json() {
    // Serialize with smart_paste_enabled = false.
    let settings = AppSettings {
        smart_paste_enabled: false,
        ..AppSettings::default()
    };
    let json = serde_json::to_string(&settings).expect("serialize");
    // Verify the field appears in the JSON.
    assert!(
        json.contains("\"smartPasteEnabled\""),
        "JSON must contain smartPasteEnabled field; got: {json}"
    );

    // Round-trip: deserialize and check the value is preserved.
    let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");
    assert!(
        !parsed.smart_paste_enabled,
        "smartPasteEnabled should be false after round-trip"
    );

    // Test default (field absent) → true.
    let json_without_field = r#"{"vaultDir":"/tmp/test"}"#;
    let parsed2: AppSettings =
        serde_json::from_str(json_without_field).expect("deserialize without field");
    assert!(
        parsed2.smart_paste_enabled,
        "smartPasteEnabled should default to true when field is absent"
    );
}
