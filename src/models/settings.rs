use serde::{Deserialize, Serialize};

use super::provider::ProviderConfig;
use crate::models::ResponseStyle;

use sha2::{Digest, Sha256};

/// Number of PBKDF2-HMAC-SHA256 iterations applied to App Lock PINs (#3323).
///
/// OWASP (2023) recommends ≥ 600,000 iterations for PBKDF2-SHA256 to resist
/// offline brute-force attacks. This makes brute-forcing a 4-digit PIN take
/// hours-to-days instead of <1 ms (as was the case with unsalted SHA-256).
const PBKDF2_ITERATIONS: u32 = 600_000;
/// Salt length in bytes (128 bits of randomness from `Uuid::new_v4`).
const PBKDF2_SALT_LEN: usize = 16;
/// PBKDF2 output length (SHA-256 digest size).
const PBKDF2_HASH_LEN: usize = 32;
/// Minimum PIN length accepted by `enable_app_lock_pin` (#3324).
const MIN_PIN_LEN: usize = 4;
/// Maximum PIN length accepted by `enable_app_lock_pin` (#3324).
const MAX_PIN_LEN: usize = 32;

/// Compute HMAC-SHA256(key, message) and return the 32-byte digest.
///
/// This is a minimal RFC 2104 implementation built on `sha2::Sha256`,
/// avoiding the need for a separate `hmac` crate dependency.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; PBKDF2_HASH_LEN] {
    const BLOCK_SIZE: usize = 64;

    // If key is longer than the block size, hash it first.
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let kh = Sha256::digest(key);
        key_block[..PBKDF2_HASH_LEN].copy_from_slice(&kh);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    // Inner pad (key XOR 0x36) and outer pad (key XOR 0x5c).
    let mut ipad = [0u8; BLOCK_SIZE];
    let mut opad = [0u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] = key_block[i] ^ 0x36;
        opad[i] = key_block[i] ^ 0x5c;
    }

    // Inner hash: H(ipad || message)
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    // Outer hash: H(opad || inner_hash)
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    outer.finalize().into()
}

/// Compute PBKDF2-HMAC-SHA256(password, salt, iterations) producing 32 bytes.
///
/// Because the requested output length (32) equals the hash size, only one
/// block needs to be computed (RFC 8018 §5.2).
fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; PBKDF2_HASH_LEN] {
    let block_index = 1u32;

    // U_1 = HMAC(password, salt || INT_32_BE(1))
    let mut msg = Vec::with_capacity(salt.len() + 4);
    msg.extend_from_slice(salt);
    msg.extend_from_slice(&block_index.to_be_bytes());
    let mut u = hmac_sha256(password, &msg);
    let mut result = u;

    for _ in 1..iterations {
        u = hmac_sha256(password, &u);
        for i in 0..PBKDF2_HASH_LEN {
            result[i] ^= u[i];
        }
    }

    result
}

/// Generate a 16-byte cryptographically random salt using `Uuid::new_v4`,
/// which draws from the OS CSPRNG.
fn generate_salt() -> [u8; PBKDF2_SALT_LEN] {
    let id = uuid::Uuid::new_v4();
    let mut salt = [0u8; PBKDF2_SALT_LEN];
    salt.copy_from_slice(id.as_bytes());
    salt
}

/// Encode a byte slice as lowercase hexadecimal.
fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Decode a hexadecimal string into bytes. Returns `None` on malformed input.
fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

/// Convert a single hex ASCII character to its nibble value.
fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Configuration for intelligent model routing (#1842).
///
/// When enabled, VaultPilot inspects each request and routes it to a
/// different model depending on its task type (simple / complex / code),
/// so cheap models handle trivial work and stronger models handle hard work.
/// Disabled by default; users opt in via Settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelRoutingConfig {
    /// Master switch. Disabled by default — the active provider's model is
    /// used for every request when this is `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Model used for simple tasks (short Q&A, translation, summaries).
    /// When `None`, falls back to the active provider's configured model.
    #[serde(default)]
    pub simple_task_model: Option<String>,
    /// Model used for complex tasks (long-form analysis, reasoning, multi-step).
    /// When `None`, falls back to the active provider's configured model.
    #[serde(default)]
    pub complex_task_model: Option<String>,
    /// Model used for code tasks (contains code or programming keywords).
    /// When `None`, falls back to the active provider's configured model.
    #[serde(default)]
    pub code_task_model: Option<String>,
}

impl ModelRoutingConfig {
    /// Returns `true` when routing is both enabled and has at least one
    /// per-task model configured. A config that is `enabled` but has no
    /// model overrides is a no-op (everything routes to the default model).
    pub fn is_active(&self) -> bool {
        self.enabled
            && (self.simple_task_model.is_some()
                || self.complex_task_model.is_some()
                || self.code_task_model.is_some())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub vault_dir: String,
    /// Legacy single-provider config (kept for backward compatibility).
    #[serde(default)]
    pub provider: ProviderConfig,
    /// Multi-provider list. When non-empty, overrides `provider`.
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Index into `providers` for the currently active provider.
    #[serde(default)]
    pub active_provider_index: usize,
    #[serde(default = "default_auto_check_updates")]
    pub auto_check_updates: bool,
    #[serde(default = "default_auto_wake_enabled")]
    pub auto_wake_enabled: bool,
    #[serde(default = "default_auto_wake_interval_minutes")]
    pub auto_wake_interval_minutes: u64,
    #[serde(default = "default_auto_wake_model")]
    pub auto_wake_model: String,
    #[serde(default = "default_auto_wake_start_time")]
    pub auto_wake_start_time: String,
    #[serde(default = "default_auto_wake_end_time")]
    pub auto_wake_end_time: String,
    /// Prompt sent to the AI when auto-wake fires (#861).
    #[serde(default = "default_auto_wake_prompt")]
    pub auto_wake_prompt: String,
    /// Response style for controlling AI answer length/depth (#1965).
    #[serde(default)]
    pub response_style: ResponseStyle,
    /// Enable automatic context compression (#1928). When enabled, earlier
    /// conversation history is compressed into a summary once token usage
    /// exceeds `compression_threshold` of the model's context window, so that
    /// long conversations are not silently truncated. Disabled by default;
    /// users opt in via Settings.
    #[serde(default)]
    pub context_compression: bool,
    /// Fraction (0.0–1.0) of the model context window at which automatic
    /// compression triggers (#1928). Default 0.8 (compress at 80% capacity).
    /// Values outside [0.1, 1.0] are clamped at runtime.
    #[serde(default = "default_compression_threshold")]
    pub compression_threshold: f32,
    /// Intelligent model routing config (#1842). Defaults to disabled.
    #[serde(default)]
    pub model_routing: ModelRoutingConfig,
    /// Name of the active vault prompt (from `.vaultpilot/prompts/`).
    /// When set, this prompt's content is prepended to AI system prompts (#1929).
    #[serde(default)]
    pub active_prompt_name: Option<String>,
    /// When true, session history is automatically exported as markdown files
    /// into the vault after each save (#1944).
    #[serde(default = "default_session_export_enabled")]
    pub session_export_enabled: bool,
    /// Custom path for session export files (relative to vault dir).
    /// Default: `.vaultpilot/sessions/`
    #[serde(default)]
    pub session_export_path: Option<String>,
    /// Global system directive that is prepended to all chat system prompts (#1766).
    /// Users can set this to define persistent AI behavior preferences (e.g. tone,
    /// response format, domain focus) that apply across all chat sessions.
    #[serde(default)]
    pub system_directive: String,
    /// Privacy mode (#2992): when enabled, only local/offline providers
    /// (e.g. Ollama on localhost) are permitted. Any cloud provider that would
    /// send data off-device is rejected at validation time and at request time,
    /// guaranteeing "nothing ever leaves your machine".
    #[serde(default)]
    pub privacy_mode: bool,
    /// Which semantic embedding provider to use for similarity search (#3129).
    /// Defaults to the built-in keyword n-gram hash embedder.
    #[serde(default)]
    pub embedding_provider: crate::semantic::EmbeddingProvider,
    /// Allow list for vaultpilot:// URI actions that skip confirmation dialog
    /// (#3074). Each entry is a URI prefix pattern; an incoming URI is allowed
    /// without confirmation if it starts with any pattern in this list.
    /// Example: ["vaultpilot://note/", "vaultpilot://chat/new"]
    #[serde(default)]
    pub allowed_uris: Vec<String>,
    /// Global proxy URL for all AI API requests (e.g. "http://127.0.0.1:7890").
    /// When set to a non-empty URL, all AI API calls go through this proxy.
    /// When empty or None, the system proxy is disabled (no auto-detection).
    /// Per-provider proxy is NOT supported — this is a global setting.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// App Lock (#3304): when enabled, the client (WinUI/mobile) must
    /// authenticate the user on launch before granting access to vault content.
    /// The actual authentication (PIN entry, biometric prompt) is handled by
    /// the platform UI layer; this flag and `app_lock_pin_hash` provide the
    /// shared configuration that all clients read from the settings file.
    #[serde(default)]
    pub app_lock_enabled: bool,
    /// Hashed PIN string when App Lock method is "pin". Stored in PHC-style
    /// format: `pbkdf2-sha256$<iterations>$<salt_hex>$<hash_hex>` (#3323).
    /// Legacy settings files may still contain a bare 64-char SHA-256 hex
    /// digest; `verify_pin` handles both formats for backward compatibility.
    /// `None` when App Lock is disabled or using biometric-only auth.
    /// The PIN itself is never stored in plaintext.
    #[serde(default)]
    pub app_lock_pin_hash: Option<String>,
    /// Custom agent tools (#3384). Each entry defines a user-registered
    /// shell command that the AI agent can invoke as a tool. Empty by
    /// default; users add tools via settings or `.vaultpilot/tools/*.toml`.
    #[serde(default)]
    pub custom_tools: Vec<crate::custom_tools::CustomTool>,
    /// WinUI: Always on Top (#3473). When enabled, the application window
    /// stays above all other windows. The actual pinning is applied by the
    /// WinUI client via AppWindow.IsAlwaysOnTop; this flag provides the
    /// persisted toggle state.
    #[serde(default)]
    pub is_always_on_top: bool,
    /// Smart Paste (#3547): When enabled and the user has text selected in
    /// the editor, pasting a URL (http:// or https://) auto-wraps the
    /// selected text as a Markdown link: `[selected text](url)`. Enabled by
    /// default to match Obsidian/Notion/Slack/Figma behaviour.
    #[serde(default = "default_smart_paste_enabled")]
    pub smart_paste_enabled: bool,
    /// Behavior when deleting a note that references attachments (images,
    /// audio, PDFs, …) that are exclusive to it (#3718, parity with Obsidian
    /// 1.12.0). Defaults to `Ask` — the platform UI prompts the user; the
    /// CLI treats `Ask` like `Never` unless `--purge-attachments` is given.
    #[serde(default)]
    pub attachment_cleanup_on_note_delete: crate::models::AttachmentCleanupMode,
    /// Persisted open-tab state for desktop clients (#3700 — WinUI multi-tab).
    /// The WinUI client saves which notes are open at session end and restores
    /// them on relaunch (parity with Anytype 0.54 "tabs restore"). Mobile and
    /// CLI ignore this field. Empty by default.
    #[serde(default)]
    pub session_tabs: Vec<TabInfo>,
    /// Index into `session_tabs` for the currently active tab.
    /// `None` when no tabs are open or after a fresh launch.
    #[serde(default)]
    pub active_tab_index: Option<usize>,
}

/// A single persisted tab entry for WinUI multi-tab support (#3700).
///
/// Desktop clients (WinUI) populate this at session end and restore on launch.
/// The `note_id` is the vault-relative path or identifier of the note; `title`
/// is a cached display title (the client refreshes it after the note loads).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    /// Vault-relative note identifier (path or slug).
    pub note_id: String,
    /// Whether the tab is pinned (immune to "close other tabs").
    #[serde(default)]
    pub is_pinned: bool,
    /// Cached display title for the tab header. `None` until first render.
    #[serde(default)]
    pub title: Option<String>,
}

fn default_privacy_mode() -> bool {
    false
}

fn default_smart_paste_enabled() -> bool {
    true
}

/// Compare two byte slices in constant time to mitigate timing attacks.
///
/// Does NOT early-return on length mismatch — iterates over the full length of
/// both buffers (zero-padding the shorter one). This prevents leaking the
/// expected hash length via timing (#3326).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max = a.len().max(b.len());
    let mut diff = 0u8;
    diff |= (a.len() != b.len()) as u8;
    for i in 0..max {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0
}

fn default_session_export_enabled() -> bool {
    false
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            vault_dir: String::new(),
            provider: ProviderConfig::default(),
            providers: Vec::new(),
            active_provider_index: 0,
            auto_check_updates: default_auto_check_updates(),
            auto_wake_enabled: default_auto_wake_enabled(),
            auto_wake_interval_minutes: default_auto_wake_interval_minutes(),
            auto_wake_model: default_auto_wake_model(),
            auto_wake_start_time: default_auto_wake_start_time(),
            auto_wake_end_time: default_auto_wake_end_time(),
            auto_wake_prompt: default_auto_wake_prompt(),
            response_style: ResponseStyle::default(),
            context_compression: false,
            compression_threshold: default_compression_threshold(),
            model_routing: ModelRoutingConfig::default(),
            active_prompt_name: None,
            session_export_enabled: false,
            session_export_path: None,
            system_directive: String::new(),
            privacy_mode: default_privacy_mode(),
            embedding_provider: crate::semantic::EmbeddingProvider::default(),
            allowed_uris: Vec::new(),
            proxy_url: None,
            app_lock_enabled: false,
            app_lock_pin_hash: None,
            custom_tools: Vec::new(),
            is_always_on_top: false,
            smart_paste_enabled: default_smart_paste_enabled(),
            attachment_cleanup_on_note_delete: crate::models::AttachmentCleanupMode::default(),
            session_tabs: Vec::new(),
            active_tab_index: None,
        }
    }
}

impl AppSettings {
    /// Return the currently active provider config.
    /// If `providers` list is non-empty, returns `providers[active_provider_index]`.
    /// Otherwise falls back to the legacy single `provider` field.
    pub fn effective_provider(&self) -> &ProviderConfig {
        if !self.providers.is_empty() {
            let idx = self.active_provider_index.min(self.providers.len() - 1);
            &self.providers[idx]
        } else {
            &self.provider
        }
    }

    /// Mutable version of effective_provider for runtime overrides.
    pub fn effective_provider_mut(&mut self) -> &mut ProviderConfig {
        if !self.providers.is_empty() {
            let idx = self.active_provider_index.min(self.providers.len() - 1);
            &mut self.providers[idx]
        } else {
            &mut self.provider
        }
    }

    /// Migrate legacy single `provider` into `providers` list if empty.
    /// Called after loading settings.
    pub fn migrate_providers(&mut self) {
        if self.providers.is_empty() && !self.provider.base_url.is_empty() {
            self.provider.name = if self.provider.name.is_empty() {
                "Default".to_string()
            } else {
                self.provider.name.clone()
            };
            self.providers.push(self.provider.clone());
        }
    }

    /// Hash a PIN string using PBKDF2-HMAC-SHA256 with a random salt (#3304, #3323).
    ///
    /// The returned string follows the PHC-style format:
    /// `pbkdf2-sha256$<iterations>$<salt_hex>$<hash_hex>`
    ///
    /// Because a fresh random salt is generated each call, hashing the same
    /// PIN twice produces **different** strings — preventing rainbow-table and
    /// pre-computation attacks. The 600k iteration count makes brute-forcing a
    /// 4-digit PIN take hours rather than <1 ms.
    ///
    /// Used by the UI layer when the user sets or changes their App Lock PIN.
    /// The plaintext PIN is never persisted — only this hash is stored.
    pub fn hash_pin(pin: &str) -> String {
        Self::hash_pin_with_iterations(pin, PBKDF2_ITERATIONS)
    }

    /// Internal helper that allows overriding the iteration count (used by
    /// tests to keep the suite fast while production uses the full count).
    fn hash_pin_with_iterations(pin: &str, iterations: u32) -> String {
        let salt = generate_salt();
        let hash = pbkdf2_sha256(pin.as_bytes(), &salt, iterations);
        format!(
            "pbkdf2-sha256${}${}${}",
            iterations,
            bytes_to_hex(&salt),
            bytes_to_hex(&hash),
        )
    }

    /// Verify a user-entered PIN against the stored hash (#3304, #3323, #3324).
    ///
    /// Returns `false` when:
    /// - App Lock is disabled (`app_lock_enabled == false`), even if a stale
    ///   hash remains from a prior configuration (#3324).
    /// - No PIN hash is configured.
    /// - The PIN does not match the stored hash.
    ///
    /// Both the modern PBKDF2 format and legacy bare-SHA-256 hashes are
    /// accepted so that existing settings files continue to work after
    /// upgrade. Comparison is constant-time to mitigate timing attacks.
    ///
    /// This function is read-only. Use [`verify_pin_and_upgrade`] instead when
    /// you also need to migrate legacy SHA-256 hashes to PBKDF2 on success
    /// (#3330).
    pub fn verify_pin(&self, pin: &str) -> bool {
        // #3324: Must honour the enabled flag — a stale hash from a previous
        // configuration must NOT allow verification when App Lock is off.
        if !self.app_lock_enabled {
            return false;
        }
        match &self.app_lock_pin_hash {
            Some(stored) => {
                if let Some(rest) = stored.strip_prefix("pbkdf2-sha256$") {
                    Self::verify_pbkdf2_hash(pin, rest)
                } else {
                    // Legacy: bare SHA-256 hex digest (pre-#3323).
                    let input = Self::legacy_sha256_hex(pin);
                    constant_time_eq(stored.as_bytes(), input.as_bytes())
                }
            }
            None => false,
        }
    }

    /// Verify a PIN and seamlessly upgrade legacy SHA-256 hashes to PBKDF2
    /// on successful verification (#3330).
    ///
    /// Behaves identically to [`verify_pin`] for PBKDF2 hashes. For legacy
    /// bare-SHA-256 hashes (pre-#3323), a successful verification triggers an
    /// automatic in-place upgrade: the stored hash is replaced with a
    /// PBKDF2-HMAC-SHA256 hash (600k iterations, random salt) of the same PIN.
    ///
    /// Callers that hold a `&mut AppSettings` should prefer this function over
    /// [`verify_pin`] so that users who set their PIN before the PBKDF2
    /// migration (#3323) are automatically upgraded the next time they unlock.
    ///
    /// Still returns `false` when App Lock is disabled or no PIN hash is
    /// configured, same as [`verify_pin`].
    pub fn verify_pin_and_upgrade(&mut self, pin: &str) -> bool {
        if !self.app_lock_enabled {
            return false;
        }
        match &self.app_lock_pin_hash {
            Some(stored) => {
                if let Some(rest) = stored.strip_prefix("pbkdf2-sha256$") {
                    // Already PBKDF2 — just verify, no upgrade needed.
                    Self::verify_pbkdf2_hash(pin, rest)
                } else {
                    // Legacy: bare SHA-256 hex digest (pre-#3323).
                    // If the PIN matches, upgrade the hash to PBKDF2.
                    let input = Self::legacy_sha256_hex(pin);
                    let matches = constant_time_eq(stored.as_bytes(), input.as_bytes());
                    if matches {
                        self.app_lock_pin_hash = Some(Self::hash_pin(pin));
                    }
                    matches
                }
            }
            None => false,
        }
    }

    /// Verify a PIN against a PBKDF2 hash body (everything after the
    /// `pbkdf2-sha256$` prefix). Returns `false` on any parse failure.
    fn verify_pbkdf2_hash(pin: &str, body: &str) -> bool {
        let parts: Vec<&str> = body.split('$').collect();
        if parts.len() != 3 {
            return false;
        }
        let iterations = match parts[0].parse::<u32>() {
            Ok(n) if n > 0 => n,
            _ => return false,
        };
        let salt = match hex_to_bytes(parts[1]) {
            Some(s) if s.len() == PBKDF2_SALT_LEN => s,
            _ => return false,
        };
        let expected = match hex_to_bytes(parts[2]) {
            Some(h) if h.len() == PBKDF2_HASH_LEN => h,
            _ => return false,
        };
        let computed = pbkdf2_sha256(pin.as_bytes(), &salt, iterations);
        constant_time_eq(&computed, &expected[..])
    }

    /// Compute a bare SHA-256 hex digest (legacy format, pre-#3323).
    fn legacy_sha256_hex(pin: &str) -> String {
        let hash = Sha256::digest(pin.as_bytes());
        format!("{:x}", hash)
    }

    /// Enable App Lock with a PIN (#3304, #3324).
    ///
    /// Validates that the PIN is ≥ 4 ASCII digits and ≤ 32 characters, then
    /// stores the PBKDF2 hash and sets the enabled flag. Returns an error
    /// message describing why the PIN was rejected, if applicable.
    pub fn enable_app_lock_pin(&mut self, pin: &str) -> Result<(), String> {
        if pin.len() < MIN_PIN_LEN {
            return Err(format!("PIN must be at least {MIN_PIN_LEN} digits"));
        }
        if pin.len() > MAX_PIN_LEN {
            return Err(format!("PIN must be at most {MAX_PIN_LEN} digits"));
        }
        if !pin.chars().all(|c| c.is_ascii_digit()) {
            return Err("PIN must contain only digits (0-9)".to_string());
        }
        self.app_lock_pin_hash = Some(Self::hash_pin(pin));
        self.app_lock_enabled = true;
        Ok(())
    }

    /// Disable App Lock entirely (#3304). Clears the PIN hash and flag.
    pub fn disable_app_lock(&mut self) {
        self.app_lock_pin_hash = None;
        self.app_lock_enabled = false;
    }

    /// Validate settings after deserialization, returning all error messages at once.
    /// An empty list means the settings are valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Validate vault_dir exists and is a directory (if non-empty).
        let vault = self.vault_dir.trim();
        if !vault.is_empty() {
            let path = std::path::Path::new(vault);
            if !path.exists() {
                errors.push(format!("vault_dir does not exist: {}", self.vault_dir));
            } else if !path.is_dir() {
                errors.push(format!("vault_dir is not a directory: {}", self.vault_dir));
            }
        }

        // Validate api_key is non-empty for providers that require authentication.
        // Ollama runs locally and does not need an API key (#2798).
        let ep = self.effective_provider();
        let provider_type = ep.effective_provider_type();
        if provider_type.requires_api_key() && ep.api_key.trim().is_empty() {
            errors.push("provider.api_key is empty; an API key is required".to_string());
        }

        // Delegate provider-specific validation.
        errors.extend(ep.validate());

        // Privacy mode (#2992): if enabled, only local/offline providers are
        // permitted. Cloud providers (anything requiring an API key or not
        // explicitly local) would send data off-device and must be rejected.
        if self.privacy_mode && ep.effective_provider_type().requires_api_key() {
            errors.push(
                "privacy_mode is enabled but the active provider requires an API key \
                 (cloud provider). Use a local provider such as Ollama (#2992)."
                    .to_string(),
            );
        }

        errors
    }

    /// Check whether `uri` is allowed by the `allowed_uris` allow list (#3074).
    /// A URI is allowed if it starts with any pattern in the list.
    /// An empty allow list means NO uri is auto-allowed (always prompt).
    pub fn is_uri_allowed(&self, uri: &str) -> bool {
        self.allowed_uris
            .iter()
            .any(|pattern| uri.starts_with(pattern))
    }

    /// Add a URI pattern to the allow list (#3074). Returns false if already present.
    pub fn add_allowed_uri(&mut self, pattern: String) -> bool {
        if self.allowed_uris.contains(&pattern) {
            false
        } else {
            self.allowed_uris.push(pattern);
            true
        }
    }

    /// Remove a URI pattern from the allow list (#3074). Returns false if not present.
    pub fn remove_allowed_uri(&mut self, pattern: &str) -> bool {
        let idx = self.allowed_uris.iter().position(|p| p == pattern);
        match idx {
            Some(i) => {
                self.allowed_uris.remove(i);
                true
            }
            None => false,
        }
    }
}

pub fn default_auto_check_updates() -> bool {
    true
}

pub fn default_auto_wake_enabled() -> bool {
    false
}

pub fn default_auto_wake_interval_minutes() -> u64 {
    30
}

pub fn default_auto_wake_model() -> String {
    String::new()
}

pub fn default_auto_wake_start_time() -> String {
    String::new()
}

pub fn default_auto_wake_end_time() -> String {
    String::new()
}

pub fn default_auto_wake_prompt() -> String {
    String::new()
}

/// Default compression threshold: compress once token usage reaches 80% of the
/// model's context window (#1928).
pub fn default_compression_threshold() -> f32 {
    0.8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::provider::{default_base_url, default_model, default_timeout_ms};

    #[test]
    fn app_settings_round_trips_with_camel_case() {
        let settings = AppSettings {
            vault_dir: "D:\\Vault".to_string(),
            provider: ProviderConfig {
                name: "test".to_string(),
                api_key: "test-key".to_string(),
                base_url: "https://api.example.com".to_string(),
                model: "test-model".to_string(),
                request_timeout_ms: 30_000,
                context_window_tokens: Some(128_000),
                max_output_tokens: Some(16384),
                provider_type: None,
            },
            providers: Vec::new(),
            active_provider_index: 0,
            auto_check_updates: false,
            auto_wake_enabled: true,
            auto_wake_interval_minutes: 60,
            auto_wake_model: "claude-3-5-haiku-latest".to_string(),
            auto_wake_start_time: "05:00".to_string(),
            auto_wake_end_time: "23:00".to_string(),
            auto_wake_prompt: String::new(),
            response_style: ResponseStyle::Standard,
            context_compression: true,
            compression_threshold: 0.75,
            model_routing: ModelRoutingConfig::default(),
            active_prompt_name: None,
            session_export_enabled: false,
            session_export_path: None,
            system_directive: String::new(),
            privacy_mode: false,
            embedding_provider: crate::semantic::EmbeddingProvider::default(),
            allowed_uris: Vec::new(),
            proxy_url: None,
            app_lock_enabled: false,
            app_lock_pin_hash: None,
            custom_tools: Vec::new(),
            is_always_on_top: false,
            smart_paste_enabled: true,
            attachment_cleanup_on_note_delete: crate::models::AttachmentCleanupMode::default(),
            session_tabs: Vec::new(),
            active_tab_index: None,
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        assert!(json.contains("\"vaultDir\""));
        assert!(json.contains("\"apiKey\""));
        assert!(json.contains("\"baseUrl\""));
        assert!(json.contains("\"requestTimeoutMs\""));
        assert!(json.contains("\"contextWindowTokens\""));
        assert!(json.contains("\"maxOutputTokens\""));
        assert!(json.contains("\"autoCheckUpdates\""));
        assert!(json.contains("\"autoWakeEnabled\""));
        assert!(json.contains("\"autoWakeIntervalMinutes\""));
        assert!(json.contains("\"autoWakeModel\""));
        assert!(json.contains("\"autoWakeStartTime\""));
        assert!(json.contains("\"autoWakeEndTime\""));
        // #1928: compression settings serialize as camelCase.
        assert!(json.contains("\"contextCompression\""));
        assert!(json.contains("\"compressionThreshold\""));
        // #1842: model routing config serializes as camelCase.
        assert!(json.contains("\"modelRouting\""));
        assert!(json.contains("\"simpleTaskModel\""));
        assert!(json.contains("\"complexTaskModel\""));
        assert!(json.contains("\"codeTaskModel\""));

        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.vault_dir, settings.vault_dir);
        assert_eq!(parsed.provider.api_key, settings.provider.api_key);
        assert_eq!(parsed.provider.context_window_tokens, Some(128_000));
        assert_eq!(parsed.provider.max_output_tokens, Some(16384));
        assert!(parsed.context_compression);
        assert_eq!(parsed.compression_threshold, 0.75);
    }

    #[test]
    fn default_values_are_correct() {
        let settings = AppSettings::default();
        assert!(settings.vault_dir.is_empty());
        assert_eq!(settings.provider.base_url, default_base_url());
        assert_eq!(settings.provider.model, default_model());
        assert_eq!(settings.provider.request_timeout_ms, default_timeout_ms());
        assert!(settings.provider.context_window_tokens.is_none());
        assert!(settings.auto_check_updates);
        assert!(!settings.auto_wake_enabled);
        assert_eq!(settings.auto_wake_interval_minutes, 30);
        assert!(settings.auto_wake_model.is_empty());
        assert!(settings.auto_wake_start_time.is_empty());
        assert!(settings.auto_wake_end_time.is_empty());
        assert!(settings.auto_wake_prompt.is_empty());
        assert_eq!(default_model(), "deepseek-v4-flash-free");
        assert_eq!(default_timeout_ms(), 60_000);
        assert!(default_auto_check_updates());
        // #1928: compression is off by default; threshold defaults to 0.8.
        assert!(!settings.context_compression);
        assert_eq!(settings.compression_threshold, 0.8);
        assert_eq!(default_compression_threshold(), 0.8);
    }

    #[test]
    fn compression_settings_round_trip_disabled() {
        // Default (disabled) settings must round-trip and preserve the toggle.
        let settings = AppSettings::default();
        let json = serde_json::to_string(&settings).expect("serialize");
        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert!(!parsed.context_compression);
        assert_eq!(parsed.compression_threshold, 0.8);
    }

    #[test]
    fn compression_settings_backwards_compatible_when_absent() {
        // A legacy settings JSON that omits the new fields entirely must
        // deserialize to safe defaults (compression disabled) (#1928).
        let legacy = serde_json::json!({
            "vaultDir": "/tmp/vault",
            "provider": {
                "apiKey": "k",
                "baseUrl": "https://api.example.com",
                "model": "m",
                "requestTimeoutMs": 60000,
            },
        });
        let parsed: AppSettings =
            serde_json::from_value(legacy).expect("legacy JSON must deserialize");
        assert!(!parsed.context_compression);
        assert_eq!(parsed.compression_threshold, 0.8);
    }

    // ── #1842: model routing ──

    #[test]
    fn model_routing_defaults_to_disabled() {
        let settings = AppSettings::default();
        assert!(!settings.model_routing.enabled);
        assert!(!settings.model_routing.is_active());
        assert!(settings.model_routing.simple_task_model.is_none());
        assert!(settings.model_routing.complex_task_model.is_none());
        assert!(settings.model_routing.code_task_model.is_none());
    }

    #[test]
    fn model_routing_backwards_compatible_when_absent() {
        // A legacy settings JSON that omits modelRouting entirely must
        // deserialize to a disabled, empty config (#1842).
        let legacy = serde_json::json!({
            "vaultDir": "/tmp/vault",
            "provider": {
                "apiKey": "k",
                "baseUrl": "https://api.example.com",
                "model": "m",
                "requestTimeoutMs": 60000,
            },
        });
        let parsed: AppSettings =
            serde_json::from_value(legacy).expect("legacy JSON must deserialize");
        assert!(!parsed.model_routing.enabled);
        assert!(!parsed.model_routing.is_active());
    }

    #[test]
    fn model_routing_round_trips_enabled_with_models() {
        let settings = AppSettings {
            model_routing: ModelRoutingConfig {
                enabled: true,
                simple_task_model: Some("haiku".into()),
                complex_task_model: Some("sonnet".into()),
                code_task_model: Some("coder".into()),
            },
            ..AppSettings::default()
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.model_routing.enabled);
        assert!(parsed.model_routing.is_active());
        assert_eq!(
            parsed.model_routing.simple_task_model.as_deref(),
            Some("haiku")
        );
        assert_eq!(
            parsed.model_routing.complex_task_model.as_deref(),
            Some("sonnet")
        );
        assert_eq!(
            parsed.model_routing.code_task_model.as_deref(),
            Some("coder")
        );
    }

    #[test]
    fn model_routing_accepts_camel_case_json() {
        // Verify the canonical camelCase JSON shape produced/consumed by the UI.
        let json = serde_json::json!({
            "modelRouting": {
                "enabled": true,
                "simpleTaskModel": "cheap",
                "complexTaskModel": null,
                "codeTaskModel": "coder",
            }
        });
        let parsed: AppSettings =
            serde_json::from_value(json).expect("camelCase JSON must deserialize");
        assert!(parsed.model_routing.enabled);
        assert_eq!(
            parsed.model_routing.simple_task_model.as_deref(),
            Some("cheap")
        );
        assert!(parsed.model_routing.complex_task_model.is_none());
        assert_eq!(
            parsed.model_routing.code_task_model.as_deref(),
            Some("coder")
        );
    }

    #[test]
    fn validate_accepts_valid_settings() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "sk-test-key".to_string(),
                base_url: "https://api.anthropic.com/v1/messages".to_string(),
                request_timeout_ms: 60_000,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        assert!(settings.validate().is_empty());
    }

    #[test]
    fn validate_privacy_mode_rejects_cloud_provider() {
        // Privacy mode (#2992): enabling it with a cloud (API-key-requiring)
        // provider must be rejected so data cannot leave the device.
        let cloud = AppSettings {
            privacy_mode: true,
            provider: ProviderConfig {
                api_key: "sk-test-key".to_string(),
                base_url: "https://api.anthropic.com/v1/messages".to_string(),
                request_timeout_ms: 60_000,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = cloud.validate();
        assert!(
            errors.iter().any(|e| e.contains("privacy_mode")),
            "expected privacy_mode rejection, got: {errors:?}"
        );

        // The same provider is fine when privacy mode is off.
        let mut off = cloud;
        off.privacy_mode = false;
        assert!(off.validate().is_empty());
    }

    #[test]
    fn validate_privacy_mode_allows_local_ollama() {
        // Ollama requires no API key and is a local endpoint, so it must be
        // permitted under privacy mode.
        let local = AppSettings {
            privacy_mode: true,
            provider: ProviderConfig {
                base_url: "http://localhost:11434/v1".to_string(),
                model: "llama3".to_string(),
                request_timeout_ms: 60_000,
                provider_type: Some(crate::models::provider::ProviderType::Ollama),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        assert!(
            local.validate().is_empty(),
            "local Ollama should be allowed under privacy mode"
        );
    }

    #[test]
    fn validate_catches_empty_api_key() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: String::new(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors.iter().any(|e| e.contains("api_key")));
    }

    #[test]
    fn validate_catches_whitespace_only_api_key() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "   ".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors.iter().any(|e| e.contains("api_key")));
    }

    #[test]
    fn validate_catches_invalid_base_url_scheme() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                base_url: "ftp://example.com".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors.iter().any(|e| e.contains("base_url")));
    }

    #[test]
    fn validate_accepts_http_base_url() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                base_url: "http://localhost:8080/v1".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(!errors.iter().any(|e| e.contains("base_url")));
    }

    #[test]
    fn validate_catches_timeout_too_low() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                request_timeout_ms: 500,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors
            .iter()
            .any(|e| e.contains("request_timeout_ms") && e.contains("too low")));
    }

    #[test]
    fn validate_catches_timeout_too_high() {
        let settings = AppSettings {
            provider: ProviderConfig {
                api_key: "key".to_string(),
                request_timeout_ms: 999_999,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors
            .iter()
            .any(|e| e.contains("request_timeout_ms") && e.contains("too high")));
    }

    #[test]
    fn validate_catches_nonexistent_vault_dir() {
        let settings = AppSettings {
            vault_dir: "/nonexistent/path/that/does/not/exist".to_string(),
            provider: ProviderConfig {
                api_key: "key".to_string(),
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(errors
            .iter()
            .any(|e| e.contains("vault_dir") && e.contains("not exist")));
    }

    #[test]
    fn validate_returns_all_errors_at_once() {
        let settings = AppSettings {
            vault_dir: "/nonexistent/path".to_string(),
            provider: ProviderConfig {
                api_key: String::new(),
                base_url: "ftp://bad".to_string(),
                request_timeout_ms: 0,
                ..ProviderConfig::default()
            },
            ..AppSettings::default()
        };
        let errors = settings.validate();
        assert!(
            errors.len() >= 4,
            "expected at least 4 errors, got: {}",
            errors.len()
        );
        assert!(errors.iter().any(|e| e.contains("vault_dir")));
        assert!(errors.iter().any(|e| e.contains("api_key")));
        assert!(errors.iter().any(|e| e.contains("base_url")));
        assert!(errors.iter().any(|e| e.contains("request_timeout_ms")));
    }

    // ── effective_provider() ──

    #[test]
    fn effective_provider_falls_back_to_legacy_when_empty() {
        let settings = AppSettings {
            provider: ProviderConfig {
                name: "legacy".into(),
                base_url: "https://legacy.api".into(),
                ..Default::default()
            },
            providers: Vec::new(),
            ..Default::default()
        };
        assert_eq!(settings.effective_provider().name, "legacy");
    }

    #[test]
    fn effective_provider_uses_active_from_list() {
        let settings = AppSettings {
            providers: vec![
                ProviderConfig {
                    name: "first".into(),
                    ..Default::default()
                },
                ProviderConfig {
                    name: "second".into(),
                    ..Default::default()
                },
            ],
            active_provider_index: 1,
            ..Default::default()
        };
        assert_eq!(settings.effective_provider().name, "second");
    }

    #[test]
    fn effective_provider_clamps_out_of_bounds_index() {
        let settings = AppSettings {
            providers: vec![ProviderConfig {
                name: "only".into(),
                ..Default::default()
            }],
            active_provider_index: 99,
            ..Default::default()
        };
        assert_eq!(settings.effective_provider().name, "only");
    }

    #[test]
    fn effective_provider_mut_modifies_correct_entry() {
        let mut settings = AppSettings {
            providers: vec![
                ProviderConfig {
                    name: "first".into(),
                    model: "m1".into(),
                    ..Default::default()
                },
                ProviderConfig {
                    name: "second".into(),
                    model: "m2".into(),
                    ..Default::default()
                },
            ],
            active_provider_index: 0,
            ..Default::default()
        };
        settings.effective_provider_mut().model = "updated".into();
        assert_eq!(settings.providers[0].model, "updated");
        assert_eq!(settings.providers[1].model, "m2");
    }

    // ── migrate_providers() ──

    #[test]
    fn migrate_providers_moves_legacy_to_list() {
        let mut settings = AppSettings {
            provider: ProviderConfig {
                name: String::new(),
                base_url: "https://api.example.com".into(),
                model: "test-model".into(),
                ..Default::default()
            },
            providers: Vec::new(),
            ..Default::default()
        };
        settings.migrate_providers();
        assert_eq!(settings.providers.len(), 1);
        assert_eq!(settings.providers[0].name, "Default");
        assert_eq!(settings.providers[0].base_url, "https://api.example.com");
    }

    #[test]
    fn migrate_providers_preserves_existing_name() {
        let mut settings = AppSettings {
            provider: ProviderConfig {
                name: "MyProvider".into(),
                base_url: "https://api.example.com".into(),
                ..Default::default()
            },
            providers: Vec::new(),
            ..Default::default()
        };
        settings.migrate_providers();
        assert_eq!(settings.providers[0].name, "MyProvider");
    }

    #[test]
    fn migrate_providers_skips_when_list_non_empty() {
        let mut settings = AppSettings {
            provider: ProviderConfig {
                base_url: "https://legacy.api".into(),
                ..Default::default()
            },
            providers: vec![ProviderConfig {
                name: "existing".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        settings.migrate_providers();
        assert_eq!(settings.providers.len(), 1);
        assert_eq!(settings.providers[0].name, "existing");
    }

    #[test]
    fn migrate_providers_skips_when_base_url_empty() {
        let mut settings = AppSettings {
            provider: ProviderConfig {
                base_url: String::new(),
                ..Default::default()
            },
            providers: Vec::new(),
            ..Default::default()
        };
        settings.migrate_providers();
        assert!(settings.providers.is_empty());
    }

    // ── #3074: URI allow list ──

    #[test]
    fn allowed_uris_default_empty() {
        let settings = AppSettings::default();
        assert!(settings.allowed_uris.is_empty());
        assert!(!settings.is_uri_allowed("vaultpilot://note/new"));
    }

    #[test]
    fn allowed_uris_exact_match() {
        let mut settings = AppSettings::default();
        settings.add_allowed_uri("vaultpilot://note/new".into());
        assert!(settings.is_uri_allowed("vaultpilot://note/new"));
        assert!(!settings.is_uri_allowed("vaultpilot://chat/new"));
    }

    #[test]
    fn allowed_uris_prefix_match() {
        let mut settings = AppSettings::default();
        settings.add_allowed_uri("vaultpilot://note/".into());
        assert!(settings.is_uri_allowed("vaultpilot://note/new"));
        assert!(settings.is_uri_allowed("vaultpilot://note/edit/abc"));
        assert!(!settings.is_uri_allowed("vaultpilot://chat/"));
    }

    #[test]
    fn allowed_uris_add_duplicate() {
        let mut settings = AppSettings::default();
        assert!(settings.add_allowed_uri("vaultpilot://note/".into()));
        assert!(!settings.add_allowed_uri("vaultpilot://note/".into()));
        assert_eq!(settings.allowed_uris.len(), 1);
    }

    #[test]
    fn allowed_uris_remove_existing() {
        let mut settings = AppSettings::default();
        settings.add_allowed_uri("vaultpilot://chat/new".into());
        assert!(settings.remove_allowed_uri("vaultpilot://chat/new"));
        assert!(!settings.is_uri_allowed("vaultpilot://chat/new"));
    }

    #[test]
    fn allowed_uris_remove_nonexistent() {
        let mut settings = AppSettings::default();
        assert!(!settings.remove_allowed_uri("vaultpilot://nonexistent"));
    }

    #[test]
    fn allowed_uris_serde_round_trip() {
        let mut settings = AppSettings::default();
        settings.add_allowed_uri("vaultpilot://note/".into());
        settings.add_allowed_uri("vaultpilot://chat/new".into());
        let json = serde_json::to_string(&settings).expect("serialize");
        assert!(json.contains("\"allowedUris\""));
        assert!(json.contains("vaultpilot://note/"));
        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.allowed_uris.len(), 2);
        assert!(parsed.is_uri_allowed("vaultpilot://note/abc"));
    }

    #[test]
    fn allowed_uris_backwards_compatible_when_absent() {
        // Legacy settings without allowedUris must deserialize to empty vec.
        let legacy = serde_json::json!({
            "vaultDir": "/tmp/vault",
            "provider": {
                "apiKey": "k",
                "baseUrl": "https://api.example.com",
                "model": "m",
                "requestTimeoutMs": 60000,
            },
        });
        let parsed: AppSettings =
            serde_json::from_value(legacy).expect("legacy JSON must deserialize");
        assert!(parsed.allowed_uris.is_empty());
        assert!(!parsed.is_uri_allowed("vaultpilot://note/new"));
    }

    // ── App Lock (#3304) ──────────────────────────────────────────────

    #[test]
    fn app_lock_disabled_by_default() {
        let settings = AppSettings::default();
        assert!(!settings.app_lock_enabled);
        assert!(settings.app_lock_pin_hash.is_none());
    }

    #[test]
    fn app_lock_enable_and_verify_pin() {
        let mut settings = AppSettings::default();
        settings.enable_app_lock_pin("1234").unwrap();
        assert!(settings.app_lock_enabled);
        assert!(settings.app_lock_pin_hash.is_some());
        // Correct PIN
        assert!(settings.verify_pin("1234"));
        // Wrong PIN
        assert!(!settings.verify_pin("9999"));
    }

    #[test]
    fn app_lock_disable_clears_state() {
        let mut settings = AppSettings::default();
        settings.enable_app_lock_pin("4321").unwrap();
        assert!(settings.app_lock_enabled);
        settings.disable_app_lock();
        assert!(!settings.app_lock_enabled);
        assert!(settings.app_lock_pin_hash.is_none());
    }

    #[test]
    fn app_lock_pin_hash_is_not_plaintext() {
        let mut settings = AppSettings::default();
        settings.enable_app_lock_pin("1234567890").unwrap();
        let hash = settings.app_lock_pin_hash.as_ref().unwrap();
        // Hash must NOT contain the plaintext PIN.
        assert!(!hash.contains("1234567890"));
        // #3323: Hash must be in PHC-style PBKDF2 format, not bare SHA-256.
        assert!(
            hash.starts_with("pbkdf2-sha256$"),
            "expected pbkdf2 prefix, got: {hash}"
        );
    }

    #[test]
    fn app_lock_verify_returns_false_without_hash() {
        let settings = AppSettings::default();
        assert!(!settings.verify_pin("1234"));
    }

    #[test]
    fn app_lock_serde_round_trip() {
        let mut settings = AppSettings::default();
        settings.enable_app_lock_pin("0000").unwrap();
        let json = serde_json::to_string(&settings).expect("serialize");
        assert!(json.contains("\"appLockEnabled\":true"));
        assert!(json.contains("\"appLockPinHash\""));
        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.app_lock_enabled);
        assert!(parsed.verify_pin("0000"));
    }

    #[test]
    fn app_lock_backwards_compatible_when_absent() {
        let legacy = serde_json::json!({
            "vaultDir": "/tmp/vault",
            "provider": {
                "apiKey": "k",
                "baseUrl": "https://api.example.com",
                "model": "m",
                "requestTimeoutMs": 60000,
            },
        });
        let parsed: AppSettings =
            serde_json::from_value(legacy).expect("legacy JSON must deserialize");
        assert!(!parsed.app_lock_enabled);
        assert!(parsed.app_lock_pin_hash.is_none());
    }

    // ── #3323: PBKDF2 + salt ──────────────────────────────────────────

    #[test]
    fn hash_pin_uses_pbkdf2_format() {
        // Format: pbkdf2-sha256$<iterations>$<salt_hex>$<hash_hex>
        let h = AppSettings::hash_pin_with_iterations("1234", 1000);
        let parts: Vec<&str> = h.split('$').collect();
        assert_eq!(parts[0], "pbkdf2-sha256");
        assert_eq!(parts[1], "1000", "iteration count must be encoded");
        // Salt is 16 bytes → 32 hex chars.
        assert_eq!(parts[2].len(), 32, "salt must be 16 bytes (32 hex chars)");
        assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()));
        // Hash is 32 bytes → 64 hex chars.
        assert_eq!(parts[3].len(), 64, "hash must be 32 bytes (64 hex chars)");
        assert!(parts[3].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_pin_same_pin_produces_different_hashes() {
        // #3323: Each call generates a fresh random salt, so two hashes of
        // the same PIN must differ. This is the core property that defeats
        // rainbow-table and pre-computation attacks.
        let a = AppSettings::hash_pin_with_iterations("1234", 1000);
        let b = AppSettings::hash_pin_with_iterations("1234", 1000);
        assert_ne!(a, b, "same PIN must produce different hashes (salt)");
        // But both must verify against the same correct PIN.
        let mut s = AppSettings {
            app_lock_enabled: true,
            app_lock_pin_hash: Some(a.clone()),
            ..Default::default()
        };
        assert!(s.verify_pin("1234"));
        s.app_lock_pin_hash = Some(b);
        assert!(s.verify_pin("1234"));
    }

    #[test]
    fn verify_pin_accepts_pbkdf2_hash() {
        let settings = AppSettings {
            app_lock_enabled: true,
            app_lock_pin_hash: Some(AppSettings::hash_pin_with_iterations("9999", 1000)),
            ..Default::default()
        };
        assert!(settings.verify_pin("9999"), "correct PIN must verify");
        assert!(!settings.verify_pin("0000"), "wrong PIN must not verify");
    }

    #[test]
    fn verify_pin_accepts_legacy_sha256_hash() {
        // #3323 backward compat: old settings files store a bare 64-char
        // SHA-256 hex digest. verify_pin must still accept the correct PIN.
        let legacy_hash = AppSettings::legacy_sha256_hex("4321");
        assert_eq!(legacy_hash.len(), 64);
        assert!(!legacy_hash.starts_with("pbkdf2"));
        let settings = AppSettings {
            app_lock_enabled: true,
            app_lock_pin_hash: Some(legacy_hash),
            ..Default::default()
        };
        assert!(settings.verify_pin("4321"), "legacy hash must verify");
        assert!(!settings.verify_pin("1234"), "wrong PIN must not verify");
    }

    // ── #3330: legacy hash → PBKDF2 migration on successful verify ──

    #[test]
    fn verify_pin_and_upgrade_migrates_legacy_hash_to_pbkdf2() {
        // Core #3330 property: when a user with a pre-#3323 legacy SHA-256
        // hash enters the correct PIN, the stored hash must be silently
        // upgraded to PBKDF2 so that the weak hash is no longer exploitable
        // for offline brute-force attacks.
        let legacy_hash = AppSettings::legacy_sha256_hex("7777");
        assert!(!legacy_hash.starts_with("pbkdf2"));
        let mut settings = AppSettings {
            app_lock_enabled: true,
            app_lock_pin_hash: Some(legacy_hash.clone()),
            ..Default::default()
        };
        // Verify with correct PIN — must succeed AND upgrade hash.
        assert!(
            settings.verify_pin_and_upgrade("7777"),
            "correct PIN must verify"
        );
        let upgraded = settings.app_lock_pin_hash.as_ref().unwrap();
        assert!(
            upgraded.starts_with("pbkdf2-sha256$"),
            "hash must be upgraded to PBKDF2 format, got: {upgraded}"
        );
        // The upgraded hash must still verify the correct PIN.
        assert!(settings.verify_pin("7777"), "upgraded hash must verify PIN");
        // Wrong PIN must still be rejected.
        assert!(
            !settings.verify_pin("0000"),
            "wrong PIN must not verify after upgrade"
        );
    }

    #[test]
    fn verify_pin_and_upgrade_keeps_pbkdf2_hash_unchanged() {
        // verify_pin_and_upgrade should be a no-op for modern PBKDF2 hashes:
        // the hash must not be rewritten when it is already strong.
        let mut settings = AppSettings {
            app_lock_enabled: true,
            app_lock_pin_hash: Some(AppSettings::hash_pin_with_iterations("1234", 1000)),
            ..Default::default()
        };
        let before = settings.app_lock_pin_hash.clone().unwrap();
        assert!(settings.verify_pin_and_upgrade("1234"));
        assert_eq!(
            settings.app_lock_pin_hash.as_ref().unwrap(),
            &before,
            "PBKDF2 hash must not be rewritten"
        );
    }

    #[test]
    fn verify_pin_and_upgrade_does_not_upgrade_on_wrong_pin() {
        // A wrong PIN must NOT trigger a hash upgrade; the weak hash must
        // stay untouched so the legitimate user can still unlock later.
        let legacy_hash = AppSettings::legacy_sha256_hex("8888");
        let mut settings = AppSettings {
            app_lock_enabled: true,
            app_lock_pin_hash: Some(legacy_hash.clone()),
            ..Default::default()
        };
        assert!(
            !settings.verify_pin_and_upgrade("0000"),
            "wrong PIN must fail"
        );
        assert_eq!(
            settings.app_lock_pin_hash.as_ref().unwrap(),
            &legacy_hash,
            "legacy hash must remain untouched on wrong PIN"
        );
    }

    #[test]
    fn verify_pin_and_upgrade_still_honours_enabled_flag() {
        // #3324 cross-check: when App Lock is disabled, verify_pin_and_upgrade
        // must return false even when a legacy hash is present, and must NOT
        // upgrade the hash.
        let legacy_hash = AppSettings::legacy_sha256_hex("5555");
        let mut settings = AppSettings {
            app_lock_enabled: false,
            app_lock_pin_hash: Some(legacy_hash.clone()),
            ..Default::default()
        };
        assert!(
            !settings.verify_pin_and_upgrade("5555"),
            "must reject when disabled"
        );
        assert_eq!(
            settings.app_lock_pin_hash.as_ref().unwrap(),
            &legacy_hash,
            "hash must not change when disabled"
        );
    }

    #[test]
    fn verify_pin_rejects_malformed_pbkdf2_hash() {
        // Garbage after the pbkdf2-sha256$ prefix must not panic or verify.
        let settings = AppSettings {
            app_lock_enabled: true,
            app_lock_pin_hash: Some("pbkdf2-sha256$garbage".to_string()),
            ..Default::default()
        };
        assert!(!settings.verify_pin("1234"));
        let settings = AppSettings {
            app_lock_enabled: true,
            app_lock_pin_hash: Some("pbkdf2-sha256$1000$bad$also_bad".to_string()),
            ..Default::default()
        };
        assert!(!settings.verify_pin("1234"));
    }

    // ── #3324: enabled-flag + PIN validation ─────────────────────────

    #[test]
    fn verify_pin_returns_false_when_disabled_but_hash_present() {
        // #3324 critical: when app_lock_enabled is false but a stale hash
        // remains (e.g. from partial migration), verify_pin MUST return false.
        let mut settings = AppSettings::default();
        settings.enable_app_lock_pin("1234").unwrap();
        assert!(settings.verify_pin("1234"));
        // Simulate the inconsistent state: enabled=false, hash still present.
        settings.app_lock_enabled = false;
        assert!(
            !settings.verify_pin("1234"),
            "verify_pin must respect app_lock_enabled flag"
        );
    }

    #[test]
    fn enable_app_lock_pin_rejects_empty_pin() {
        let mut settings = AppSettings::default();
        assert!(settings.enable_app_lock_pin("").is_err());
        assert!(!settings.app_lock_enabled, "must not enable on empty PIN");
        assert!(settings.app_lock_pin_hash.is_none());
    }

    #[test]
    fn enable_app_lock_pin_rejects_short_pin() {
        let mut settings = AppSettings::default();
        assert!(settings.enable_app_lock_pin("123").is_err());
        assert!(!settings.app_lock_enabled);
        // Exactly 4 digits is the minimum and must succeed.
        assert!(settings.enable_app_lock_pin("1234").is_ok());
        assert!(settings.app_lock_enabled);
    }

    #[test]
    fn enable_app_lock_pin_rejects_non_digit_pin() {
        let mut settings = AppSettings::default();
        // Letters mixed with digits.
        assert!(settings.enable_app_lock_pin("12a4").is_err());
        // Special characters.
        assert!(settings.enable_app_lock_pin("12-4").is_err());
        // Spaces.
        assert!(settings.enable_app_lock_pin("12 4").is_err());
        // Unicode digits (not ASCII).
        assert!(settings.enable_app_lock_pin("１２３４").is_err());
        assert!(!settings.app_lock_enabled);
    }

    #[test]
    fn enable_app_lock_pin_rejects_overlong_pin() {
        let mut settings = AppSettings::default();
        let long = "1".repeat(33);
        assert!(settings.enable_app_lock_pin(&long).is_err());
        // 32 digits is the maximum and must succeed.
        let max = "9".repeat(32);
        assert!(settings.enable_app_lock_pin(&max).is_ok());
    }

    #[test]
    fn enable_app_lock_pin_accepts_valid_pins() {
        let mut settings = AppSettings::default();
        assert!(settings.enable_app_lock_pin("0000").is_ok());
        assert!(settings.app_lock_enabled);
        assert!(settings.verify_pin("0000"));

        settings.disable_app_lock();
        assert!(settings.enable_app_lock_pin("1234567890").is_ok());
        assert!(settings.verify_pin("1234567890"));
    }

    // ── PBKDF2 + hex helpers unit tests ──────────────────────────────

    #[test]
    fn hmac_sha256_matches_known_test_vector() {
        // RFC 4231 Test Case 2:
        //   key = "Jefe", data = "what do ya want for nothing?"
        //   HMAC-SHA256 = 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843
        let result = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let expected_hex = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
        assert_eq!(bytes_to_hex(&result), expected_hex);
    }

    #[test]
    fn pbkdf2_sha256_matches_known_test_vector() {
        // RFC 7914 / RFC 6070-style vector for PBKDF2-HMAC-SHA256:
        //   password = "password", salt = "salt", c = 1
        //   output   = 120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b
        let result = pbkdf2_sha256(b"password", b"salt", 1);
        let expected_hex = "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b";
        assert_eq!(bytes_to_hex(&result), expected_hex);
    }

    #[test]
    fn hex_round_trip() {
        let original = vec![0x00u8, 0x01, 0xfe, 0xff, 0xab, 0xcd];
        let hex = bytes_to_hex(&original);
        assert_eq!(hex, "0001feffabcd");
        let decoded = hex_to_bytes(&hex).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn hex_to_bytes_rejects_malformed() {
        assert!(hex_to_bytes("abc").is_none(), "odd length");
        assert!(hex_to_bytes("xy").is_none(), "non-hex chars");
        assert!(hex_to_bytes("").is_some(), "empty is valid → empty vec");
    }

    // ── #3326: constant_time_eq length-leak fix ──────────────────────

    #[test]
    fn constant_time_eq_handles_length_mismatch() {
        // Same content prefix, different lengths → must return false.
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(!constant_time_eq(b"short", b"longer"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(!constant_time_eq(b"a", b""));
        // Same length, same content → must return true.
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(constant_time_eq(b"", b""));
        // Same length, different content → must return false.
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"abc", b"abd"));
    }

    // --- Regression tests for #3700: session tab persistence ---

    #[test]
    fn session_tabs_default_empty() {
        let settings = AppSettings::default();
        assert!(settings.session_tabs.is_empty());
        assert!(settings.active_tab_index.is_none());
    }

    #[test]
    fn session_tabs_round_trip() {
        let settings = AppSettings {
            session_tabs: vec![
                TabInfo {
                    note_id: "notes/project-alpha.md".into(),
                    is_pinned: true,
                    title: Some("Project Alpha".into()),
                },
                TabInfo {
                    note_id: "inbox.md".into(),
                    is_pinned: false,
                    title: None,
                },
            ],
            active_tab_index: Some(0),
            ..Default::default()
        };

        let json = serde_json::to_string(&settings).expect("serialize");
        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.session_tabs.len(), 2);
        assert_eq!(parsed.session_tabs[0].note_id, "notes/project-alpha.md");
        assert!(parsed.session_tabs[0].is_pinned);
        assert_eq!(
            parsed.session_tabs[0].title.as_deref(),
            Some("Project Alpha")
        );
        assert_eq!(parsed.session_tabs[1].note_id, "inbox.md");
        assert!(!parsed.session_tabs[1].is_pinned);
        assert!(parsed.session_tabs[1].title.is_none());
        assert_eq!(parsed.active_tab_index, Some(0));
    }

    #[test]
    fn session_tabs_backwards_compatible_when_absent() {
        // A legacy settings JSON that omits session_tabs and active_tab_index
        // entirely must still deserialize with empty defaults.
        let legacy = serde_json::json!({
            "vaultDir": "/tmp/vault",
            "provider": {
                "name": "test",
                "apiKey": "key",
                "baseUrl": "https://api.test.com",
                "model": "m",
                "requestTimeoutMs": 60000
            }
        });
        let parsed: AppSettings = serde_json::from_value(legacy).expect("deserialize");
        assert!(parsed.session_tabs.is_empty());
        assert!(parsed.active_tab_index.is_none());
    }

    #[test]
    fn session_tabs_serialize_camel_case() {
        let settings = AppSettings {
            session_tabs: vec![TabInfo {
                note_id: "x.md".into(),
                is_pinned: true,
                title: Some("X".into()),
            }],
            active_tab_index: Some(0),
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        assert!(json.contains("\"sessionTabs\""));
        assert!(json.contains("\"activeTabIndex\""));
        assert!(json.contains("\"noteId\""));
        assert!(json.contains("\"isPinned\""));
    }
}
