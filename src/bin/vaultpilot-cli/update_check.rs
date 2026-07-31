//! CLI startup version self-check (#3648).
//!
//! On each interactive CLI invocation (skipped for `serve`, `mcp`, `mcp-http`,
//! and when `--no-update-check` / `VAULTPILOT_NO_UPDATE_CHECK` is set), we
//! asynchronously query the GitHub Releases API for the latest published tag
//! and print a warning to stderr when a newer version exists.
//!
//! Design goals:
//! - Non-blocking: a 3-second timeout; failure is silent.
//! - Cached: results are cached for 24 h in `<config_dir>/.vaultpilot/update_check.json`
//!   so frequent CLI invocations don't hammer the API.
//! - Opt-out: `--no-update-check` flag, `VAULTPILOT_NO_UPDATE_CHECK=1` env, or
//!   `auto_check_updates: false` in settings.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const GITHUB_API_URL: &str = "https://api.github.com/repos/ryanloee/VaultPilot/releases/latest";
const CACHE_TTL_SECS: u64 = 86_400; // 24 hours
const REQUEST_TIMEOUT_SECS: u64 = 3;

/// On-disk cache record.
#[derive(Serialize, Deserialize)]
struct CacheRecord {
    /// Unix timestamp (seconds) of the last successful check.
    checked_at: u64,
    /// Latest tag name from GitHub (e.g. "v0.6.54").
    latest_tag: String,
}

/// Async entry point — call this from the runtime in `main()`.
///
/// All parameters are caller-provided so the logic is easy to unit-test.
pub async fn run_update_check(config_dir: PathBuf, auto_check_updates: bool) {
    // Quick gates — bail before any I/O.
    if !auto_check_updates {
        return;
    }
    if std::env::var_os("VAULTPILOT_NO_UPDATE_CHECK").is_some() {
        return;
    }

    let cache_path = config_dir.join(".vaultpilot").join("update_check.json");

    // Try cache first — if fresh enough, use it without hitting the network.
    if let Some(latest) = read_cache(&cache_path) {
        print_warning_if_outdated(&latest);
        return;
    }

    // Network fetch with short timeout.
    let latest = match fetch_latest_tag().await {
        Ok(tag) => tag,
        Err(_) => return, // silent on failure
    };

    // Persist cache (best-effort).
    let _ = write_cache(&cache_path, &latest);

    print_warning_if_outdated(&latest);
}

fn print_warning_if_outdated(latest_tag: &str) {
    let current = env!("CARGO_PKG_VERSION"); // e.g. "0.6.54"
    if is_newer(latest_tag, current) {
        eprintln!(
            "⚠ vaultpilot-cli {current} — new version {latest_tag} available\n  \
             → https://github.com/ryanloee/VaultPilot/releases\n  \
             (set VAULTPILOT_NO_UPDATE_CHECK=1 to suppress)"
        );
    }
}

// ─── Cache helpers ───────────────────────────────────────────────

fn read_cache(path: &PathBuf) -> Option<String> {
    let data = std::fs::read_to_string(path).ok()?;
    let record: CacheRecord = serde_json::from_str(&data).ok()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now >= record.checked_at && now - record.checked_at < CACHE_TTL_SECS {
        Some(record.latest_tag)
    } else {
        None
    }
}

fn write_cache(path: &PathBuf, latest_tag: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let record = CacheRecord {
        checked_at: now,
        latest_tag: latest_tag.to_string(),
    };
    let json = serde_json::to_string(&record)?;
    std::fs::write(path, json)
}

// ─── Network ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

async fn fetch_latest_tag() -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .user_agent(format!("vaultpilot-cli/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    let release: GithubRelease = client.get(GITHUB_API_URL).send().await?.json().await?;
    Ok(release.tag_name)
}

// ─── Version comparison ──────────────────────────────────────────
//
// We don't pull in the `semver` crate (Cargo.toml is off-limits for new deps).
// Instead we implement a minimal comparator that handles "vX.Y.Z" / "X.Y.Z"
// and optional pre-release suffixes.

/// Returns `true` if `latest` (e.g. "v0.6.55") is strictly newer than
/// `current` (e.g. "0.6.54").
fn is_newer(latest: &str, current: &str) -> bool {
    let (lv, lpre) = strip_v(latest);
    let (cv, cpre) = strip_v(current);
    match cmp_release(lv, cv) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            // Equal release → pre-release is *older* than release.
            match (lpre, cpre) {
                (None, None) => false,
                (None, Some(_)) => true, // latest is release, current is pre → newer
                (Some(_), None) => false, // latest is pre, current is release → not newer
                (Some(a), Some(b)) => a > b, // both pre-release: lexicographic
            }
        }
    }
}

/// Strip leading `v` and split into (release, pre-release).
/// "v0.6.54-rc1" → ("0.6.54", Some("rc1"))
fn strip_v(tag: &str) -> (&str, Option<&str>) {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    match tag.find('-') {
        Some(idx) => (&tag[..idx], Some(&tag[idx + 1..])),
        None => (tag, None),
    }
}

/// Compare two "X.Y.Z" release strings component by component.
fn cmp_release(a: &str, b: &str) -> std::cmp::Ordering {
    let pa: Vec<u64> = a.split('.').filter_map(|s| s.parse().ok()).collect();
    let pb: Vec<u64> = b.split('.').filter_map(|s| s.parse().ok()).collect();
    for i in 0..pa.len().max(pb.len()) {
        let va = pa.get(i).copied().unwrap_or(0);
        let vb = pb.get(i).copied().unwrap_or(0);
        match va.cmp(&vb) {
            std::cmp::Ordering::Equal => continue,
            ord => return ord,
        }
    }
    std::cmp::Ordering::Equal
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_basic() {
        assert!(is_newer("v0.6.55", "0.6.54"));
        assert!(is_newer("0.7.0", "0.6.54"));
        assert!(is_newer("v1.0.0", "0.6.54"));
    }

    #[test]
    fn test_not_newer() {
        assert!(!is_newer("v0.6.54", "0.6.54"));
        assert!(!is_newer("v0.6.53", "0.6.54"));
        assert!(!is_newer("v0.5.99", "0.6.0"));
    }

    #[test]
    fn test_is_newer_prerelease() {
        // Pre-release is older than release of same version
        assert!(!is_newer("v0.6.54-rc1", "0.6.54"));
        // Higher version pre-release is still newer
        assert!(is_newer("v0.6.55-rc0", "0.6.54"));
    }

    #[test]
    fn test_strip_v() {
        assert_eq!(strip_v("v0.6.54"), ("0.6.54", None));
        assert_eq!(strip_v("0.6.54"), ("0.6.54", None));
        assert_eq!(strip_v("v1.2.3-rc1"), ("1.2.3", Some("rc1")));
        assert_eq!(strip_v("v1.2.3-beta.2"), ("1.2.3", Some("beta.2")));
    }

    #[test]
    fn test_cmp_release() {
        assert_eq!(cmp_release("0.6.54", "0.6.54"), std::cmp::Ordering::Equal);
        assert_eq!(cmp_release("0.6.55", "0.6.54"), std::cmp::Ordering::Greater);
        assert_eq!(cmp_release("0.6.53", "0.6.54"), std::cmp::Ordering::Less);
        assert_eq!(cmp_release("1.0.0", "0.9.99"), std::cmp::Ordering::Greater);
        assert_eq!(cmp_release("0.6.1", "0.6"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn test_cache_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "vaultpilot-update-test-{}-{}.json",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        // Clean up after test
        let _ = std::fs::remove_file(&path);

        // No cache → None
        assert!(read_cache(&path).is_none());

        // Write cache
        write_cache(&path, "v9.9.9").unwrap();

        // Read back
        let result = read_cache(&path).unwrap();
        assert_eq!(result, "v9.9.9");

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_auto_check_disabled() {
        // When auto_check_updates is false, the function should return immediately
        // without any side effects (no cache file created, no network call).
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-update-disabled-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        run_update_check(dir.clone(), false).await;
        assert!(!dir.join(".vaultpilot").join("update_check.json").exists());
    }

    #[tokio::test]
    async fn test_env_var_disables() {
        // When VAULTPILOT_NO_UPDATE_CHECK is set, skip entirely.
        std::env::set_var("VAULTPILOT_NO_UPDATE_CHECK", "1");
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-update-env-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        run_update_check(dir.clone(), true).await;
        assert!(!dir.join(".vaultpilot").join("update_check.json").exists());
        std::env::remove_var("VAULTPILOT_NO_UPDATE_CHECK");
    }
}
