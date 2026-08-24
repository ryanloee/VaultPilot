//! User script extension system (#3562).
//!
//! Allows users to place executable scripts in `.vaultpilot/scripts/` and run
//! them via `vaultpilot-cli script run <name>`. Scripts can declare metadata
//! (description, timeout, arguments) via a simple frontmatter comment block or
//! a companion `.yaml` manifest.
//!
//! Inspired by Notion Workers (2026) — a CLI-first way for users to extend
//! VaultPilot without modifying core code. Scripts receive vault context via
//! environment variables and optional JSON arguments via stdin.
//!
//! ## Script discovery
//!
//! The scripts directory (default: `.vaultpilot/scripts/`) is scanned for:
//! - Executable files (any extension: `.sh`, `.py`, `.js`, `.ts`, `.rs`, etc.)
//! - Companion `.yaml` manifests (e.g. `my-script.sh` → `my-script.yaml`)
//!
//! ## Metadata format (YAML manifest)
//!
//! ```yaml
//! # my-script.yaml — companion manifest for my-script.sh
//! description: Fetch weather data and save as a note
//! timeout_seconds: 60
//! interpreter: bash  # optional: override shebang detection
//! tags: [weather, automation]
//! ```
//!
//! Alternatively, scripts can embed metadata as a leading comment block:
//!
//! ```bash
//! #!/usr/bin/env bash
//! # @vp-description Fetch weather data and save as a note
//! # @vp-timeout 60
//! # @vp-tags weather,automation
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

/// Default scripts directory relative to the vault root.
pub const SCRIPTS_DIR: &str = ".vaultpilot/scripts";

/// Default timeout for script execution (seconds).
const DEFAULT_TIMEOUT: u64 = 60;

/// Maximum timeout allowed (seconds) — prevents runaway scripts.
const MAX_TIMEOUT: u64 = 3600;

// ── Script metadata ───────────────────────────────────────────────────────

/// Metadata for a user script, parsed from a YAML manifest or inline comments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptMeta {
    /// Human-readable description shown in `script list`.
    #[serde(default)]
    pub description: String,
    /// Maximum execution time in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Optional interpreter override (e.g., "python3", "node", "bash").
    /// If not set, the script's shebang line or file extension is used.
    #[serde(default)]
    pub interpreter: Option<String>,
    /// Optional tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Whether this script is enabled. Disabled scripts are hidden from listing.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT
}

fn default_enabled() -> bool {
    true
}

impl Default for ScriptMeta {
    fn default() -> Self {
        Self {
            description: String::new(),
            timeout_seconds: DEFAULT_TIMEOUT,
            interpreter: None,
            tags: Vec::new(),
            enabled: true,
        }
    }
}

// ── Discovered script ─────────────────────────────────────────────────────

/// A user script discovered in the scripts directory.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserScript {
    /// Script name (filename without extension), used as the invocation key.
    pub name: String,
    /// Full path to the script file.
    pub path: PathBuf,
    /// File extension (e.g., "sh", "py", "js"), or empty if none.
    pub extension: String,
    /// Parsed metadata (from companion YAML manifest or inline comments).
    pub meta: ScriptMeta,
    /// Whether the script file is executable.
    pub is_executable: bool,
}

impl UserScript {
    /// Get the command parts needed to execute this script.
    ///
    /// Resolution order:
    /// 1. Manifest `interpreter` field (if set)
    /// 2. Shebang line in the script file (`#!...`)
    /// 3. File extension mapping (`.py` → python3, `.js` → node, `.sh` → bash)
    /// 4. Direct execution (if the file is executable)
    fn resolve_command(&self) -> Result<(String, Vec<String>)> {
        // 1. Manifest interpreter override
        if let Some(interp) = &self.meta.interpreter {
            if !interp.trim().is_empty() {
                let parts: Vec<&str> = interp.split_whitespace().collect();
                if parts.is_empty() {
                    anyhow::bail!("interpreter is empty for script '{}'", self.name);
                }
                let program = parts[0].to_string();
                let mut args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
                args.push(self.path.to_string_lossy().to_string());
                return Ok((program, args));
            }
        }

        // 2. Shebang line
        if let Some(shebang) = self.read_shebang()? {
            let (prog, mut args) = parse_shebang(&shebang);
            // When using the shebang interpreter explicitly (rather than direct
            // execution), we need to append the script path as an argument.
            args.push(self.path.to_string_lossy().to_string());
            return Ok((prog, args));
        }

        // 3. Extension mapping
        let interpreter = match self.extension.as_str() {
            "py" => Some("python3"),
            "js" => Some("node"),
            "ts" => Some("npx"),
            "sh" => Some("bash"),
            "rb" => Some("ruby"),
            "pl" => Some("perl"),
            "lua" => Some("lua"),
            "php" => Some("php"),
            _ => None,
        };

        if let Some(interp) = interpreter {
            let script_path = self.path.to_string_lossy().to_string();
            if interp == "npx" {
                // For TypeScript: npx tsx <script>
                return Ok(("npx".to_string(), vec!["tsx".to_string(), script_path]));
            }
            return Ok((interp.to_string(), vec![script_path]));
        }

        // 4. Direct execution
        if !self.is_executable {
            anyhow::bail!(
                "script '{}' is not executable and has no recognizable extension or interpreter",
                self.name
            );
        }
        Ok((self.path.to_string_lossy().to_string(), Vec::new()))
    }

    /// Read the shebang line from the script file, if present.
    fn read_shebang(&self) -> Result<Option<String>> {
        use std::io::{BufRead, BufReader};
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };
        let mut reader = BufReader::new(file);
        let mut first_line = String::new();
        if reader.read_line(&mut first_line).is_err() {
            return Ok(None);
        }
        if first_line.starts_with("#!") {
            Ok(Some(first_line.trim().to_string()))
        } else {
            Ok(None)
        }
    }

    /// Execute the script with optional JSON arguments on stdin.
    ///
    /// Reuses the sandbox pattern from `custom_tools.rs`: environment is
    /// cleared, only essential vars are provided, and a timeout is enforced.
    pub async fn execute(&self, args_json: &str, vault_dir: &Path) -> Result<String> {
        let (program, args) = self.resolve_command()?;
        let script_dir = self.path.parent().unwrap_or(Path::new(".")).to_path_buf();

        let mut cmd = Command::new(&program);
        cmd.kill_on_drop(true);
        cmd.args(&args);
        cmd.current_dir(&script_dir);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Security: clear environment, provide only essential vars
        cmd.env_clear();
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("HOME", std::env::var("HOME").unwrap_or_default());
        cmd.env("VAULTPILOT_VAULT_DIR", vault_dir);
        cmd.env("VAULTPILOT_SCRIPT_NAME", &self.name);
        cmd.env("VAULTPILOT_SCRIPT_DIR", &script_dir);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to execute script '{}': {}", self.name, program))?;

        // Write arguments to stdin with timeout protection
        if !args_json.is_empty() {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let stdin_timeout = Duration::from_secs(self.meta.timeout_seconds.clamp(1, 10));
                let write_result =
                    tokio::time::timeout(stdin_timeout, stdin.write_all(args_json.as_bytes()))
                        .await;
                if write_result.is_err() {
                    anyhow::bail!(
                        "script '{}' stdin write timed out after {}s",
                        self.name,
                        stdin_timeout.as_secs()
                    );
                }
                stdin.shutdown().await.ok();
            }
        } else if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.shutdown().await.ok();
        }

        // Wait with timeout
        let timeout = Duration::from_secs(self.meta.timeout_seconds.min(MAX_TIMEOUT));
        let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                anyhow::bail!("script '{}' execution failed: {}", self.name, e);
            }
            Err(_) => {
                anyhow::bail!(
                    "script '{}' timed out after {}s",
                    self.name,
                    timeout.as_secs()
                );
            }
        };

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout.trim().is_empty() {
                Ok(format!("Script '{}' completed with no output.", self.name))
            } else {
                Ok(stdout)
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            anyhow::bail!(
                "script '{}' exited with code {:?}: {}",
                self.name,
                output.status.code(),
                stderr.trim()
            );
        }
    }
}

/// Parse a shebang line into (program, args).
///
/// Handles `/usr/bin/env python3` style shebangs, including the `-S` flag
/// (split args) used by some scripts: `#!/usr/bin/env -S python3 -u`.
fn parse_shebang(shebang: &str) -> (String, Vec<String>) {
    // Strip the `#!` prefix
    let line = shebang.strip_prefix("#!").unwrap_or(shebang).trim();
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return ("/bin/sh".to_string(), Vec::new());
    }

    // Handle /usr/bin/env wrapper — may include -S flag
    if parts[0].ends_with("env") {
        // Skip flags like -S to find the actual interpreter
        let mut idx = 1;
        while idx < parts.len() && parts[idx].starts_with('-') {
            idx += 1;
        }
        if idx < parts.len() {
            let program = parts[idx].to_string();
            let args: Vec<String> = parts[idx + 1..].iter().map(|s| s.to_string()).collect();
            return (program, args);
        }
    }

    let program = parts[0].to_string();
    let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
    (program, args)
}

// ── Script discovery ──────────────────────────────────────────────────────

/// Discover all user scripts in the given scripts directory.
///
/// Scans for executable files and companion `.yaml` manifests. Non-executable
/// files without a recognized extension are skipped.
pub fn discover_scripts(scripts_dir: &Path) -> Result<Vec<UserScript>> {
    let mut scripts = Vec::new();

    if !scripts_dir.exists() {
        return Ok(scripts);
    }
    if !scripts_dir.is_dir() {
        anyhow::bail!(
            "scripts path '{}' is not a directory",
            scripts_dir.display()
        );
    }

    // Build a map of YAML manifests for quick lookup
    let mut manifests: HashMap<String, ScriptMeta> = HashMap::new();
    let entries = std::fs::read_dir(scripts_dir)
        .with_context(|| format!("failed to read scripts dir '{}'", scripts_dir.display()))?;

    // First pass: collect manifests
    let mut script_entries: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        if ext == "yaml" || ext == "yml" {
            // Parse manifest
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(meta) = serde_yaml_ng::from_str::<ScriptMeta>(&content) {
                    manifests.insert(stem, meta);
                }
            }
        } else if is_script_file(&path, &ext) {
            script_entries.push(path);
        }
    }

    // Second pass: build UserScript entries
    for path in script_entries {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        let is_executable = is_executable(&path);

        // Get metadata: companion YAML manifest → inline comments → default
        let mut meta = manifests
            .get(&name)
            .cloned()
            .unwrap_or_else(|| parse_inline_meta(&path).unwrap_or_default());

        // If YAML manifest exists, merge inline overrides for unset fields
        if manifests.contains_key(&name) {
            if let Some(inline) = parse_inline_meta(&path) {
                if meta.description.is_empty() && !inline.description.is_empty() {
                    meta.description = inline.description;
                }
                if meta.interpreter.is_none() && inline.interpreter.is_some() {
                    meta.interpreter = inline.interpreter;
                }
                if meta.tags.is_empty() && !inline.tags.is_empty() {
                    meta.tags = inline.tags;
                }
            }
        }

        scripts.push(UserScript {
            name,
            path,
            extension,
            meta,
            is_executable,
        });
    }

    // Sort by name for deterministic output
    scripts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(scripts)
}

/// Check if a file is a plausible script (by extension or executability).
fn is_script_file(path: &Path, ext: &str) -> bool {
    // Known script extensions
    const KNOWN_EXTS: &[&str] = &["sh", "py", "js", "ts", "rb", "pl", "lua", "php"];

    if KNOWN_EXTS.contains(&ext) {
        return true;
    }

    // No extension but executable → likely a script with shebang
    if ext.is_empty() && is_executable(path) {
        return true;
    }

    // Extensionless executable files are handled above; skip dotfiles
    if path
        .file_name()
        .map(|n| n.to_string_lossy().starts_with('.'))
        .unwrap_or(true)
    {
        return false;
    }

    false
}

/// Check if a file is executable on the current platform.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            return meta.permissions().mode() & 0o111 != 0;
        }
        false
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, check for .exe/.bat/.cmd extensions
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        matches!(ext.as_str(), "exe" | "bat" | "cmd" | "ps1")
    }
}

/// Parse inline metadata from script comment lines.
///
/// Supports `@vp-` prefixed tags in comments:
/// ```bash
/// # @vp-description Fetch weather data
/// # @vp-timeout 60
/// # @vp-tags weather,automation
/// # @vp-interpreter python3
/// ```
fn parse_inline_meta(path: &Path) -> Option<ScriptMeta> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut meta = ScriptMeta::default();
    let mut found_any = false;

    // Only scan the first 20 lines for metadata
    for line in reader.lines().take(20) {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();

        // Skip empty lines and shebang
        if trimmed.is_empty() || trimmed.starts_with("#!") {
            continue;
        }

        // Only process comment lines (starting with # or //)
        let comment = if trimmed.starts_with('#') {
            trimmed.strip_prefix('#').unwrap_or(trimmed).trim()
        } else if trimmed.starts_with("//") {
            trimmed.strip_prefix("//").unwrap_or(trimmed).trim()
        } else {
            // Non-comment, non-empty line → end of metadata block
            break;
        };

        if let Some(desc) = comment.strip_prefix("@vp-description ") {
            meta.description = desc.trim().to_string();
            found_any = true;
        } else if let Some(timeout) = comment.strip_prefix("@vp-timeout ") {
            if let Ok(t) = timeout.trim().parse::<u64>() {
                meta.timeout_seconds = t;
                found_any = true;
            }
        } else if let Some(tags) = comment.strip_prefix("@vp-tags ") {
            meta.tags = tags
                .trim()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            found_any = true;
        } else if let Some(interp) = comment.strip_prefix("@vp-interpreter ") {
            let interp = interp.trim().to_string();
            if !interp.is_empty() {
                meta.interpreter = Some(interp);
                found_any = true;
            }
        }
    }

    if found_any {
        Some(meta)
    } else {
        None
    }
}

// ── Script initialization ─────────────────────────────────────────────────

/// Create the scripts directory and optionally a starter script.
///
/// Returns the path to the created directory.
pub fn init_scripts_dir(vault_dir: &Path) -> Result<PathBuf> {
    let scripts_dir = vault_dir.join(SCRIPTS_DIR);
    if !scripts_dir.exists() {
        std::fs::create_dir_all(&scripts_dir).with_context(|| {
            format!(
                "failed to create scripts directory '{}'",
                scripts_dir.display()
            )
        })?;
    }

    // Create a README with usage instructions
    let readme = scripts_dir.join("README.md");
    if !readme.exists() {
        std::fs::write(&readme, SCRIPTS_README).ok();
    }

    // Create an example script
    let example = scripts_dir.join("hello.sh");
    if !example.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(
                &example,
                "#!/usr/bin/env bash\n\
                 # @vp-description Example script — prints a greeting\n\
                 # @vp-tags example,demo\n\
                 \n\
                 echo \"Hello from VaultPilot Scripts!\"\n\
                 echo \"Vault directory: $VAULTPILOT_VAULT_DIR\"\n\
                 echo \"Script name: $VAULTPILOT_SCRIPT_NAME\"\n",
            )?;
            let mut perms = std::fs::metadata(&example)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&example, perms)?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(
                &example,
                "@echo off\n\
                 rem @vp-description Example script\n\
                 echo Hello from VaultPilot Scripts!\n",
            )?;
        }
    }

    Ok(scripts_dir)
}

/// Find a script by name (case-insensitive) in the discovered list.
pub fn find_script<'a>(scripts: &'a [UserScript], name: &str) -> Option<&'a UserScript> {
    // Exact match first
    if let Some(s) = scripts.iter().find(|s| s.name == name) {
        return Some(s);
    }
    // Case-insensitive fallback
    scripts.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

// ── Constants ─────────────────────────────────────────────────────────────

const SCRIPTS_README: &str = r#"# VaultPilot User Scripts

This directory contains your custom scripts. Each script can be run via:

    vaultpilot-cli script run <name>

## Adding a script

1. Create a file here (e.g., `weather.sh`, `sync.py`, `summarize.js`).
2. Make it executable: `chmod +x weather.sh`
3. Optionally add metadata comments at the top:

```bash
#!/usr/bin/env bash
# @vp-description Fetch weather data and save as a note
# @vp-timeout 60
# @vp-tags weather,automation
# @vp-interpreter bash
```

4. Run it: `vaultpilot-cli script run weather`

## Metadata

You can also create a companion `.yaml` file (e.g., `weather.yaml`):

```yaml
description: Fetch weather data and save as a note
timeoutSeconds: 60
interpreter: bash
tags: [weather, automation]
```

## Environment variables

Scripts receive these environment variables:

| Variable | Description |
|----------|-------------|
| `VAULTPILOT_VAULT_DIR` | Path to the vault root |
| `VAULTPILOT_SCRIPT_NAME` | Name of the script being run |
| `VAULTPILOT_SCRIPT_DIR` | Path to the scripts directory |
| `PATH` | System PATH |
| `HOME` | User home directory |

## Passing arguments

Arguments are passed as JSON on stdin. Read them in your script:

```python
import json, sys
args = json.load(sys.stdin) if not sys.stdin.isatty() else {}
print(args.get("city", "unknown"))
```

Then invoke: `echo '{"city":"Tokyo"}' | vaultpilot-cli script run weather`
"#;

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// RAII guard that wipes a temp directory on drop. Matches the pattern used
    /// elsewhere in the crate (e.g. `canvas::tests`) — no `tempfile` dev-dependency.
    struct TempDirGuard(std::path::PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_temp_dir() -> (std::path::PathBuf, TempDirGuard) {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "vp-scripts-test-{}-{}-{}",
            std::process::id(),
            counter,
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let guard = TempDirGuard(dir.clone());
        (dir, guard)
    }

    fn write_script(dir: &Path, name: &str, content: &str, executable: bool) -> PathBuf {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).expect("failed to create script");
        file.write_all(content.as_bytes()).expect("failed to write");
        drop(file);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if executable {
                let mut perms = std::fs::metadata(&path).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&path, perms).unwrap();
            }
        }
        #[cfg(not(unix))]
        {
            let _ = executable; // suppress warning
        }

        path
    }

    #[test]
    fn test_parse_shebang_simple() {
        let (prog, args) = parse_shebang("#!/bin/bash");
        assert_eq!(prog, "/bin/bash");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_shebang_env() {
        let (prog, args) = parse_shebang("#!/usr/bin/env python3");
        assert_eq!(prog, "python3");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_shebang_env_with_args() {
        let (prog, args) = parse_shebang("#!/usr/bin/env -S python3 -u");
        assert_eq!(prog, "python3");
        assert_eq!(args, vec!["-u"]);
    }

    #[test]
    fn test_parse_shebang_empty() {
        let (prog, _) = parse_shebang("#!");
        assert_eq!(prog, "/bin/sh");
    }

    #[test]
    fn test_script_meta_default() {
        let meta = ScriptMeta::default();
        assert_eq!(meta.timeout_seconds, DEFAULT_TIMEOUT);
        assert!(meta.enabled);
        assert!(meta.description.is_empty());
        assert!(meta.interpreter.is_none());
        assert!(meta.tags.is_empty());
    }

    #[test]
    fn test_script_meta_yaml_parse() {
        let yaml_str = r#"
description: Test script
timeoutSeconds: 120
interpreter: python3
tags:
  - test
  - demo
"#;
        let meta: ScriptMeta = serde_yaml_ng::from_str(yaml_str).expect("failed to parse");
        assert_eq!(meta.description, "Test script");
        assert_eq!(meta.timeout_seconds, 120);
        assert_eq!(meta.interpreter.as_deref(), Some("python3"));
        assert_eq!(meta.tags, vec!["test", "demo"]);
        assert!(meta.enabled);
    }

    #[test]
    fn test_parse_inline_meta_basic() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(
            &dir,
            "test.sh",
            "#!/usr/bin/env bash\n\
             # @vp-description A test script\n\
             # @vp-timeout 45\n\
             # @vp-tags alpha,beta\n\
             \n\
             echo hello\n",
            true,
        );
        let meta = parse_inline_meta(&path).expect("should parse metadata");
        assert_eq!(meta.description, "A test script");
        assert_eq!(meta.timeout_seconds, 45);
        assert_eq!(meta.tags, vec!["alpha", "beta"]);
    }

    #[test]
    fn test_parse_inline_meta_interpreter() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(
            &dir,
            "test.py",
            "#!/usr/bin/env python3\n\
             # @vp-description Python test\n\
             # @vp-interpreter python3.11\n\
             \n\
             print('hello')\n",
            false,
        );
        let meta = parse_inline_meta(&path).expect("should parse");
        assert_eq!(meta.interpreter.as_deref(), Some("python3.11"));
    }

    #[test]
    fn test_parse_inline_meta_none() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(
            &dir,
            "plain.sh",
            "#!/usr/bin/env bash\n\
             echo hello\n",
            true,
        );
        assert!(parse_inline_meta(&path).is_none());
    }

    #[test]
    fn test_parse_inline_meta_double_slash_comments() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(
            &dir,
            "test.js",
            "// @vp-description JS test\n\
             // @vp-timeout 30\n\
             \n\
             console.log('hello');\n",
            false,
        );
        let meta = parse_inline_meta(&path).expect("should parse JS comments");
        assert_eq!(meta.description, "JS test");
        assert_eq!(meta.timeout_seconds, 30);
    }

    #[test]
    fn test_discover_scripts_empty_dir() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        let scripts = discover_scripts(&scripts_dir).expect("should succeed");
        assert!(scripts.is_empty());
    }

    #[test]
    fn test_discover_scripts_nonexistent_dir() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("nonexistent");
        let scripts = discover_scripts(&scripts_dir).expect("should return empty");
        assert!(scripts.is_empty());
    }

    #[test]
    fn test_discover_scripts_finds_sh() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        write_script(
            &scripts_dir,
            "backup.sh",
            "#!/usr/bin/env bash\n# @vp-description Backup script\necho backup\n",
            true,
        );
        let scripts = discover_scripts(&scripts_dir).expect("should succeed");
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "backup");
        assert_eq!(scripts[0].extension, "sh");
        assert_eq!(scripts[0].meta.description, "Backup script");
        #[cfg(unix)]
        assert!(scripts[0].is_executable);
    }

    #[test]
    fn test_discover_scripts_finds_py() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        write_script(
            &scripts_dir,
            "sync.py",
            "#!/usr/bin/env python3\nprint('sync')\n",
            false,
        );
        let scripts = discover_scripts(&scripts_dir).expect("should succeed");
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "sync");
        assert_eq!(scripts[0].extension, "py");
        #[cfg(unix)]
        assert!(!scripts[0].is_executable);
    }

    #[test]
    fn test_discover_scripts_with_yaml_manifest() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();

        write_script(
            &scripts_dir,
            "weather.sh",
            "#!/usr/bin/env bash\necho weather\n",
            true,
        );

        // Companion manifest
        std::fs::write(
            scripts_dir.join("weather.yaml"),
            "description: Get weather forecast\n\
             timeoutSeconds: 120\n\
             tags:\n  - weather\n  - api\n",
        )
        .unwrap();

        let scripts = discover_scripts(&scripts_dir).expect("should succeed");
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "weather");
        assert_eq!(scripts[0].meta.description, "Get weather forecast");
        assert_eq!(scripts[0].meta.timeout_seconds, 120);
        assert_eq!(scripts[0].meta.tags, vec!["weather", "api"]);
    }

    #[test]
    fn test_discover_scripts_sorted_by_name() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        write_script(&scripts_dir, "zebra.sh", "#!/bin/bash\necho z\n", true);
        write_script(&scripts_dir, "alpha.sh", "#!/bin/bash\necho a\n", true);
        write_script(&scripts_dir, "mid.sh", "#!/bin/bash\necho m\n", true);

        let scripts = discover_scripts(&scripts_dir).expect("should succeed");
        assert_eq!(scripts.len(), 3);
        assert_eq!(scripts[0].name, "alpha");
        assert_eq!(scripts[1].name, "mid");
        assert_eq!(scripts[2].name, "zebra");
    }

    #[test]
    fn test_discover_scripts_skips_non_script_files() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        // .md and .txt files should be skipped
        std::fs::write(scripts_dir.join("README.md"), "# Readme").unwrap();
        std::fs::write(scripts_dir.join("notes.txt"), "some notes").unwrap();
        write_script(&scripts_dir, "real.sh", "#!/bin/bash\necho hi\n", true);

        let scripts = discover_scripts(&scripts_dir).expect("should succeed");
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "real");
    }

    #[test]
    fn test_discover_scripts_skips_go_and_rs_without_interpreter() {
        // Regression for #3646: `.go` and `.rs` have no interpreter mapping in
        // resolve_command(), so they must NOT be listed as runnable scripts.
        // Only extensions with a known interpreter (or shebang) are included.
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        // .go and .rs are NOT in KNOWN_EXTS → skipped even when executable
        write_script(&scripts_dir, "tool.go", "package main\n", true);
        write_script(&scripts_dir, "tool.rs", "fn main() {}\n", true);
        // Known-interpreter script still discovered
        write_script(&scripts_dir, "real.sh", "#!/bin/bash\necho hi\n", true);

        let scripts = discover_scripts(&scripts_dir).expect("should succeed");
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "real");
        assert_eq!(scripts[0].extension, "sh");
    }

    #[test]
    fn test_known_exts_matches_interpreter_mapping() {
        // Regression for #3646 + #3662: every extension in KNOWN_EXTS must have a
        // corresponding interpreter mapping in resolve_command(), otherwise
        // `script list` shows entries that `script run` cannot execute.
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();

        // Write scripts WITHOUT shebangs — this exercises the extension mapping
        // path in resolve_command() (not the shebang path). PHP files in the wild
        // typically start with `<?php`, not a shebang (#3662).
        for (ext, content) in [
            ("sh", "echo hi\n"),
            ("py", "print('hi')\n"),
            ("js", "console.log('hi')\n"),
            ("ts", "console.log('hi')\n"),
            ("rb", "puts 'hi'\n"),
            ("pl", "print 'hi\\n';\n"),
            ("lua", "print('hi')\n"),
            ("php", "<?php echo 'hi';\n"),
        ] {
            write_script(&scripts_dir, &format!("script.{ext}"), content, false);
        }
        // .go / .rs have no interpreter mapping → must not be discovered
        write_script(&scripts_dir, "compiled.go", "package main\n", true);
        write_script(&scripts_dir, "compiled.rs", "fn main() {}\n", true);

        let scripts = discover_scripts(&scripts_dir).expect("should succeed");
        let mut exts: Vec<&str> = scripts.iter().map(|s| s.extension.as_str()).collect();
        exts.sort_unstable();
        assert_eq!(
            exts,
            vec!["js", "lua", "php", "pl", "py", "rb", "sh", "ts"],
            "KNOWN_EXTS must stay in sync with resolve_command() interpreter mapping"
        );

        // Verify resolve_command() succeeds for every discovered extension.
        // This catches the #3662 regression where an ext was in KNOWN_EXTS
        // but had no interpreter mapping (the original test gave false confidence).
        for script in &scripts {
            let result = script.resolve_command();
            assert!(
                result.is_ok(),
                "resolve_command() failed for .{} script '{}': {:?}",
                script.extension,
                script.name,
                result.err()
            );
        }
    }

    #[test]
    fn test_php_without_shebang_resolves() {
        // Regression for #3662: PHP files without a shebang (the common case,
        // since PHP files start with `<?php` not `#!/usr/bin/env php`) must
        // resolve to the `php` interpreter via extension mapping.
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();

        write_script(&scripts_dir, "hello.php", "<?php echo 'hi';\n", false);

        let scripts = discover_scripts(&scripts_dir).expect("should succeed");
        assert_eq!(scripts.len(), 1, "PHP script should be discovered");
        assert_eq!(scripts[0].extension, "php");

        let (prog, args) = scripts[0]
            .resolve_command()
            .expect("PHP script should resolve to php interpreter");
        assert_eq!(prog, "php", "PHP script should resolve to php interpreter");
        assert!(
            args.iter().any(|a| a.ends_with("hello.php")),
            "args should contain the script path"
        );
    }

    #[test]
    fn test_find_script_exact() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        write_script(&scripts_dir, "alpha.sh", "#!/bin/bash\necho a\n", true);
        write_script(&scripts_dir, "beta.sh", "#!/bin/bash\necho b\n", true);

        let scripts = discover_scripts(&scripts_dir).unwrap();
        assert!(find_script(&scripts, "alpha").is_some());
        assert!(find_script(&scripts, "beta").is_some());
        assert!(find_script(&scripts, "gamma").is_none());
    }

    #[test]
    fn test_find_script_case_insensitive() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        write_script(&scripts_dir, "Weather.sh", "#!/bin/bash\necho w\n", true);

        let scripts = discover_scripts(&scripts_dir).unwrap();
        assert!(find_script(&scripts, "weather").is_some());
        assert!(find_script(&scripts, "WEATHER").is_some());
    }

    #[test]
    fn test_resolve_command_shebang() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(
            &dir,
            "test.sh",
            "#!/usr/bin/env python3\nprint('hi')\n",
            true,
        );
        let script = UserScript {
            name: "test".to_string(),
            path,
            extension: "sh".to_string(),
            meta: ScriptMeta::default(),
            is_executable: true,
        };
        let (prog, _) = script.resolve_command().expect("should resolve");
        assert_eq!(prog, "python3");
    }

    #[test]
    fn test_resolve_command_extension_py() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(&dir, "test.py", "print('hi')\n", false);
        let script = UserScript {
            name: "test".to_string(),
            path,
            extension: "py".to_string(),
            meta: ScriptMeta::default(),
            is_executable: false,
        };
        let (prog, args) = script.resolve_command().expect("should resolve");
        assert_eq!(prog, "python3");
        assert_eq!(args.len(), 1); // script path
    }

    #[test]
    fn test_resolve_command_interpreter_override() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(&dir, "test.py", "print('hi')\n", false);
        let script = UserScript {
            name: "test".to_string(),
            path,
            extension: "py".to_string(),
            meta: ScriptMeta {
                interpreter: Some("python3.12".to_string()),
                ..Default::default()
            },
            is_executable: false,
        };
        let (prog, _) = script.resolve_command().expect("should resolve");
        assert_eq!(prog, "python3.12");
    }

    #[test]
    fn test_resolve_command_extension_js() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(&dir, "test.js", "console.log('hi')\n", false);
        let script = UserScript {
            name: "test".to_string(),
            path,
            extension: "js".to_string(),
            meta: ScriptMeta::default(),
            is_executable: false,
        };
        let (prog, args) = script.resolve_command().expect("should resolve");
        assert_eq!(prog, "node");
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn test_resolve_command_executable_no_extension() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(&dir, "mycmd", "#!/bin/bash\necho hi\n", true);
        let script = UserScript {
            name: "mycmd".to_string(),
            path,
            extension: String::new(),
            meta: ScriptMeta::default(),
            is_executable: true,
        };
        let (prog, args) = script.resolve_command().expect("should resolve");
        // Shebang is detected: program is /bin/bash, script path is the arg
        assert!(prog.contains("bash"));
        assert_eq!(args.len(), 1); // the script path
    }

    #[test]
    fn test_resolve_command_not_executable_no_ext() {
        let (dir, _guard) = make_temp_dir();
        let path = write_script(&dir, "mycmd", "random content\n", false);
        let script = UserScript {
            name: "mycmd".to_string(),
            path,
            extension: String::new(),
            meta: ScriptMeta::default(),
            is_executable: false,
        };
        assert!(script.resolve_command().is_err());
    }

    #[test]
    fn test_init_scripts_dir_creates_directory() {
        let (dir, _guard) = make_temp_dir();
        let vault_dir = &dir;
        let scripts_dir = init_scripts_dir(vault_dir).expect("should init");
        assert!(scripts_dir.exists());
        assert!(scripts_dir.is_dir());
        assert!(scripts_dir.join("README.md").exists());
        assert!(scripts_dir.join("hello.sh").exists());
    }

    #[test]
    fn test_init_scripts_dir_idempotent() {
        let (dir, _guard) = make_temp_dir();
        let vault_dir = &dir;
        init_scripts_dir(vault_dir).expect("first init");
        init_scripts_dir(vault_dir).expect("second init should not fail");
        let scripts_dir = vault_dir.join(SCRIPTS_DIR);
        assert!(scripts_dir.exists());
    }

    #[tokio::test]
    async fn test_execute_simple_echo() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script_path = scripts_dir.join("echo_test.sh");
            std::fs::write(&script_path, "#!/bin/sh\necho hello_world\n").unwrap();
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();

            let script = UserScript {
                name: "echo_test".to_string(),
                path: script_path,
                extension: "sh".to_string(),
                meta: ScriptMeta {
                    timeout_seconds: 5,
                    ..Default::default()
                },
                is_executable: true,
            };

            let result = script.execute("", &dir).await.expect("should execute");
            assert!(result.contains("hello_world"));
        }
    }

    #[tokio::test]
    async fn test_execute_timeout() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script_path = scripts_dir.join("slow.sh");
            std::fs::write(&script_path, "#!/bin/sh\nsleep 10\n").unwrap();
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();

            let script = UserScript {
                name: "slow".to_string(),
                path: script_path,
                extension: "sh".to_string(),
                meta: ScriptMeta {
                    timeout_seconds: 1,
                    ..Default::default()
                },
                is_executable: true,
            };

            let result = script.execute("", &dir).await;
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("timed out"));
        }
    }

    #[tokio::test]
    async fn test_execute_failure_exit_code() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script_path = scripts_dir.join("fail.sh");
            std::fs::write(&script_path, "#!/bin/sh\necho 'error msg' >&2\nexit 1\n").unwrap();
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();

            let script = UserScript {
                name: "fail".to_string(),
                path: script_path,
                extension: "sh".to_string(),
                meta: ScriptMeta {
                    timeout_seconds: 5,
                    ..Default::default()
                },
                is_executable: true,
            };

            let result = script.execute("", &dir).await;
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("exited with code"));
            assert!(err.contains("error msg"));
        }
    }

    #[tokio::test]
    async fn test_execute_env_vars_set() {
        let (dir, _guard) = make_temp_dir();
        let scripts_dir = dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script_path = scripts_dir.join("envtest.sh");
            std::fs::write(
                &script_path,
                "#!/bin/sh\necho \"NAME=$VAULTPILOT_SCRIPT_NAME\"\necho \"DIR=$VAULTPILOT_VAULT_DIR\"\n",
            )
            .unwrap();
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();

            let script = UserScript {
                name: "envtest".to_string(),
                path: script_path,
                extension: "sh".to_string(),
                meta: ScriptMeta {
                    timeout_seconds: 5,
                    ..Default::default()
                },
                is_executable: true,
            };

            let result = script.execute("", &dir).await.expect("should execute");
            assert!(result.contains("NAME=envtest"));
            assert!(result.contains("DIR="));
        }
    }
}
