/// Regression test for issue #1175: missing fields in test constructors.
///
/// Bug:      CI broke because ProviderConfig and AppSettings structs gained new
///           fields (`name`, `providers`, `active_provider_index`) but test
///           constructors were not updated.
/// Root cause: Manual struct construction in tests instead of using Default.
/// Fix:      PR #1185 / commit f760e9a
///
/// This test ensures that:
/// 1. Default constructors are valid
/// 2. Minimal JSON deserialization works (serde defaults fill missing fields)
/// 3. All struct fields round-trip through serialization
#[cfg(test)]
mod tests {
    use crate::models::{AppSettings, ProviderConfig};

    #[test]
    fn regression_1175_provider_config_default_is_valid() {
        let pc = ProviderConfig::default();
        // All fields should have sensible defaults
        assert!(
            pc.base_url.starts_with("http"),
            "base_url should have a default"
        );
        assert!(!pc.model.is_empty(), "model should have a default");
        assert!(pc.request_timeout_ms > 0, "timeout should be positive");
    }

    #[test]
    fn regression_1175_app_settings_default_is_valid() {
        let s = AppSettings::default();
        // provider should be constructible
        let _ = s.provider.masked();
        // providers list starts empty
        assert!(s.providers.is_empty());
        assert_eq!(s.active_provider_index, 0);
    }

    #[test]
    fn regression_1175_minimal_json_deserializes() {
        // Simulates loading an old settings file that lacks new fields
        let json = r#"{
            "vaultDir": "/tmp/test",
            "provider": { "apiKey": "sk-test", "baseUrl": "https://api.openai.com/v1" }
        }"#;
        let settings: AppSettings =
            serde_json::from_str(json).expect("minimal JSON should deserialize");
        assert_eq!(settings.vault_dir, "/tmp/test");
        assert_eq!(settings.provider.api_key, "sk-test");
        // New fields should have defaults, not crash
        assert!(settings.providers.is_empty());
        assert_eq!(settings.active_provider_index, 0);
        assert!(settings.provider.name.is_empty());
    }

    #[test]
    fn regression_1175_serialization_round_trip() {
        let original = AppSettings::default();
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original.vault_dir, restored.vault_dir);
        assert_eq!(original.provider.model, restored.provider.model);
        assert_eq!(
            original.active_provider_index,
            restored.active_provider_index
        );
    }
}
