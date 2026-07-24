//! Custom tool plugin system (#3384).
//!
//! Allows users to register custom shell commands as Agent tools. Each custom
//! tool declares a name, description, optional JSON Schema parameters, and an
//! execution command. The agent can invoke these tools just like built-in ones
//! (`search_notes`, `read_file`, etc.), and the proxy sandbox executes the
//! configured command with the tool-call arguments as JSON on stdin.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

// ── Tool definition ───────────────────────────────────────────────────────

/// A user-defined tool that the AI agent can invoke.
///
/// Custom tools are declared in `AppSettings.custom_tools` (or discovered from
/// `.vaultpilot/tools/*.toml`). Each tool maps an agent-callable name to a
/// shell command; when the agent selects the tool, its arguments (as JSON) are
/// passed to the command on stdin and the command's stdout is returned to the
/// agent as the tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomTool {
    /// Tool name as referenced by the AI agent. Must be unique and not clash
    /// with built-in tool names (`search_notes`, `read_file`, etc.).
    /// Only lowercase alphanumeric + underscores are allowed.
    pub name: String,
    /// Human-readable description shown to the model in the tool prompt.
    pub description: String,
    /// Shell command to execute. Arguments from the agent are passed as a JSON
    /// object on stdin. The command's stdout becomes the tool result.
    ///
    /// Example: `"python3 -c 'import json,sys; print(json.load(sys.stdin))'"`
    /// Example: `"curl -s -X POST -d @- https://api.example.com/webhook"`
    pub command: String,
    /// Maximum execution time in seconds. Default 30s.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    /// Optional working directory for the command (relative to vault dir if
    /// not absolute). Defaults to the vault root.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Whether this tool is enabled. Disabled tools are hidden from the agent.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_timeout_seconds() -> u64 {
    30
}

fn default_enabled() -> bool {
    true
}

/// Built-in tool names that custom tools must not shadow.
pub const BUILTIN_TOOLS: &[&str] = &[
    "search_notes",
    "read_file",
    "list_directory",
    "list_notes",
    "save_note",
];

impl CustomTool {
    /// Validate a custom tool definition.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("custom tool name is empty");
        }
        // Only allow lowercase alphanumeric + underscores for tool names.
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            anyhow::bail!(
                "custom tool name '{}' contains invalid characters (only a-z, 0-9, _ allowed)",
                self.name
            );
        }
        if BUILTIN_TOOLS.contains(&self.name.as_str()) {
            anyhow::bail!(
                "custom tool name '{}' conflicts with a built-in tool",
                self.name
            );
        }
        if self.command.trim().is_empty() {
            anyhow::bail!("custom tool '{}' has empty command", self.name);
        }
        if self.timeout_seconds == 0 {
            anyhow::bail!("custom tool '{}' has zero timeout", self.name);
        }
        Ok(())
    }

    /// Resolve the working directory relative to the vault root.
    pub fn resolve_workdir(&self, vault_dir: &Path) -> PathBuf {
        match &self.working_dir {
            Some(wd) if !wd.trim().is_empty() => {
                let p = PathBuf::from(wd);
                if p.is_absolute() {
                    p
                } else {
                    vault_dir.join(p)
                }
            }
            _ => vault_dir.to_path_buf(),
        }
    }

    /// Split the command string into program + args using shell-like parsing.
    /// Handles single and double quotes, and backslash escapes.
    fn split_command(&self) -> Result<(String, Vec<String>)> {
        let parts = split_shell_words(&self.command);
        if parts.is_empty() {
            anyhow::bail!(
                "custom tool '{}' has empty command after parsing",
                self.name
            );
        }
        let program = parts[0].clone();
        let args = parts[1..].to_vec();
        Ok((program, args))
    }

    /// Execute the custom tool with the given arguments (JSON string from the
    /// agent) and return the stdout output.
    pub async fn execute(&self, args_json: &str, vault_dir: &Path) -> Result<String> {
        let (program, args) = self.split_command()?;
        let workdir = self.resolve_workdir(vault_dir);

        let mut cmd = Command::new(&program);
        cmd.kill_on_drop(true); // Ensure subprocess is killed on timeout/drop (#3413)
        cmd.args(&args);
        cmd.current_dir(&workdir);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Security: run the subprocess in the vault directory context, but
        // don't leak the API key or other secrets into child env.
        cmd.env_clear();
        // Provide essential environment variables for scripts.
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("HOME", std::env::var("HOME").unwrap_or_default());
        cmd.env("VAULTPILOT_VAULT_DIR", vault_dir);
        cmd.env("VAULTPILOT_TOOL_NAME", &self.name);

        let mut child = cmd.spawn().with_context(|| {
            format!(
                "failed to spawn command for custom tool '{}': {}",
                self.name, program
            )
        })?;

        // Write arguments to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(args_json.as_bytes()).await.ok();
            stdin.shutdown().await.ok();
        }

        // Wait with timeout
        let timeout = Duration::from_secs(self.timeout_seconds);
        let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                anyhow::bail!("custom tool '{}' execution failed: {}", self.name, e);
            }
            Err(_) => {
                anyhow::bail!(
                    "custom tool '{}' timed out after {}s",
                    self.name,
                    self.timeout_seconds
                );
            }
        };

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if stdout.trim().is_empty() {
                Ok(format!("Tool '{}' completed with no output.", self.name))
            } else {
                Ok(stdout)
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            anyhow::bail!(
                "custom tool '{}' exited with code {:?}: {}",
                self.name,
                output.status.code(),
                stderr.trim()
            );
        }
    }
}

// ── Registry ──────────────────────────────────────────────────────────────

/// Registry of all custom tools loaded from settings + vault `.vaultpilot/tools/`.
#[derive(Debug, Clone, Default)]
pub struct CustomToolRegistry {
    tools: HashMap<String, CustomTool>,
}

impl CustomToolRegistry {
    /// Build a registry from a list of tool definitions. Invalid or disabled
    /// tools are silently skipped (with validation errors ignored for robustness).
    pub fn from_tools(tools: &[CustomTool]) -> Self {
        let mut map = HashMap::new();
        for tool in tools {
            if !tool.enabled {
                continue;
            }
            if tool.validate().is_err() {
                continue;
            }
            // First definition wins (dedup by name)
            map.entry(tool.name.clone()).or_insert_with(|| tool.clone());
        }
        Self { tools: map }
    }

    /// Returns true if a custom tool with the given name is registered.
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<&CustomTool> {
        self.tools.get(name)
    }

    /// List all registered tool names.
    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Generate a human-readable description block for the AI prompt, listing
    /// available custom tools. Returns empty string if no tools are registered.
    pub fn prompt_description(&self) -> String {
        if self.tools.is_empty() {
            return String::new();
        }
        let mut lines =
            vec!["\nAdditionally, the following custom tools are available:".to_string()];
        for tool in self.tools.values() {
            lines.push(format!("- {}: {}", tool.name, tool.description));
        }
        lines.push(
            "- To use a custom tool, set the \"tool\" field to its name and pass any required arguments as JSON fields in the tool call object.".to_string(),
        );
        lines.join("\n")
    }
}

// ── Shell word splitting ──────────────────────────────────────────────────

/// Split a command string into words, respecting single quotes, double quotes,
/// and backslash escapes. This is a minimal shell-like parser that avoids
/// adding external dependencies.
///
/// Examples:
/// - `"echo hello"` → `["echo", "hello"]`
/// - `"python3 -c 'print(1)'"` → `["python3", "-c", "print(1)"]`
/// - `"curl -d \"a b\""` → `["curl", "-d", "a b"]`
fn split_shell_words(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();
    let mut in_word = false;
    let mut in_single = false;
    let mut in_double = false;

    while let Some(ch) = chars.next() {
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                current.push(ch);
            }
            continue;
        }
        if in_double {
            match ch {
                '"' => in_double = false,
                '\\' => {
                    if let Some(&next) = chars.peek() {
                        if next == '"' || next == '\\' || next == '$' {
                            chars.next();
                            current.push(next);
                        } else {
                            current.push('\\');
                        }
                    } else {
                        current.push('\\');
                    }
                }
                _ => current.push(ch),
            }
            continue;
        }
        // Not in any quote context
        match ch {
            ' ' | '\t' | '\n' => {
                if in_word {
                    result.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                in_single = true;
            }
            '"' => {
                in_word = true;
                in_double = true;
            }
            '\\' => {
                in_word = true;
                if let Some(&next) = chars.peek() {
                    chars.next();
                    current.push(next);
                }
            }
            _ => {
                in_word = true;
                current.push(ch);
            }
        }
    }

    if in_word {
        result.push(current);
    }

    result
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(name: &str, cmd: &str) -> CustomTool {
        CustomTool {
            name: name.to_string(),
            description: format!("Test tool {}", name),
            command: cmd.to_string(),
            timeout_seconds: 5,
            working_dir: None,
            enabled: true,
        }
    }

    #[test]
    fn test_custom_tool_validation_valid() {
        let tool = make_tool("my_tool", "echo hello");
        assert!(tool.validate().is_ok());
    }

    #[test]
    fn test_custom_tool_validation_empty_name() {
        let mut tool = make_tool("test", "echo hi");
        tool.name = "".to_string();
        assert!(tool.validate().is_err());
    }

    #[test]
    fn test_custom_tool_validation_invalid_chars() {
        let mut tool = make_tool("test", "echo hi");
        tool.name = "My-Tool".to_string(); // uppercase + hyphen
        assert!(tool.validate().is_err());
    }

    #[test]
    fn test_custom_tool_validation_builtin_conflict() {
        let tool = make_tool("search_notes", "echo hi");
        assert!(tool.validate().is_err());
    }

    #[test]
    fn test_custom_tool_validation_empty_command() {
        let mut tool = make_tool("test", "echo hi");
        tool.command = "   ".to_string();
        assert!(tool.validate().is_err());
    }

    #[test]
    fn test_custom_tool_validation_zero_timeout() {
        let mut tool = make_tool("test", "echo hi");
        tool.timeout_seconds = 0;
        assert!(tool.validate().is_err());
    }

    #[test]
    fn test_registry_from_tools() {
        let tools = vec![make_tool("alpha", "echo a"), make_tool("beta", "echo b")];
        let reg = CustomToolRegistry::from_tools(&tools);
        assert!(reg.has("alpha"));
        assert!(reg.has("beta"));
        assert!(!reg.has("gamma"));
        assert_eq!(reg.names().len(), 2);
    }

    #[test]
    fn test_registry_skips_disabled() {
        let mut tool = make_tool("alpha", "echo a");
        tool.enabled = false;
        let reg = CustomToolRegistry::from_tools(&[tool]);
        assert!(!reg.has("alpha"));
        assert!(reg.names().is_empty());
    }

    #[test]
    fn test_registry_skips_invalid() {
        let tools = vec![
            make_tool("alpha", "echo a"),
            make_tool("search_notes", "echo conflict"), // conflicts with builtin
        ];
        let reg = CustomToolRegistry::from_tools(&tools);
        assert!(reg.has("alpha"));
        assert!(!reg.has("search_notes")); // filtered out by validation
    }

    #[test]
    fn test_registry_dedup_by_name() {
        let tools = vec![
            make_tool("alpha", "echo first"),
            make_tool("alpha", "echo second"),
        ];
        let reg = CustomToolRegistry::from_tools(&tools);
        assert_eq!(reg.names().len(), 1);
        assert_eq!(reg.get("alpha").unwrap().command, "echo first");
    }

    #[test]
    fn test_registry_prompt_description_empty() {
        let reg = CustomToolRegistry::default();
        assert!(reg.prompt_description().is_empty());
    }

    #[test]
    fn test_registry_prompt_description_has_tools() {
        let tools = vec![
            make_tool("webhook", "curl -s https://api.example.com"),
            make_tool("query_db", "sqlite3 vault.db"),
        ];
        let reg = CustomToolRegistry::from_tools(&tools);
        let desc = reg.prompt_description();
        assert!(desc.contains("webhook"));
        assert!(desc.contains("query_db"));
        assert!(desc.contains("custom tools are available"));
    }

    #[test]
    fn test_resolve_workdir_default() {
        let tool = make_tool("test", "echo hi");
        let dir = tool.resolve_workdir(Path::new("/vault"));
        assert_eq!(dir, PathBuf::from("/vault"));
    }

    #[test]
    fn test_resolve_workdir_relative() {
        let mut tool = make_tool("test", "echo hi");
        tool.working_dir = Some("scripts".to_string());
        let dir = tool.resolve_workdir(Path::new("/vault"));
        assert_eq!(dir, PathBuf::from("/vault/scripts"));
    }

    #[test]
    fn test_resolve_workdir_absolute() {
        let mut tool = make_tool("test", "echo hi");
        tool.working_dir = Some("/opt/scripts".to_string());
        let dir = tool.resolve_workdir(Path::new("/vault"));
        assert_eq!(dir, PathBuf::from("/opt/scripts"));
    }

    #[tokio::test]
    async fn test_execute_simple_echo() {
        // `echo` is a standalone binary on Unix but only a cmd.exe built-in on
        // Windows, so route through `cmd /C` there.
        #[cfg(unix)]
        let tool = make_tool("echo_tool", "echo hello-from-tool");
        #[cfg(windows)]
        let tool = make_tool("echo_tool", "cmd /C echo hello-from-tool");
        let result = tool.execute("{}", &std::env::temp_dir()).await;
        assert!(result.is_ok());
        assert!(
            result.unwrap().contains("hello-from-tool"),
            "expected the echoed argument in tool output"
        );
    }

    #[tokio::test]
    async fn test_execute_with_stdin_args() {
        // Echo stdin back: `cat` on Unix; `findstr .` on Windows prints any
        // stdin line containing at least one character.
        #[cfg(unix)]
        let tool = make_tool("passthrough", "cat");
        #[cfg(windows)]
        let tool = make_tool("passthrough", "findstr .");
        let result = tool
            .execute(r#"{"key":"value"}"#, &std::env::temp_dir())
            .await;
        assert!(result.is_ok());
        assert!(
            result.unwrap().contains("key"),
            "expected the stdin arguments echoed back on stdout"
        );
    }

    #[tokio::test]
    async fn test_execute_timeout() {
        // A long-running command that exceeds the 1s timeout: `sleep` on Unix,
        // a `ping` loop on Windows.
        #[cfg(unix)]
        let mut tool = make_tool("slow_tool", "sleep 100");
        #[cfg(windows)]
        let mut tool = make_tool("slow_tool", "ping -n 100 127.0.0.1");
        tool.timeout_seconds = 1;
        let result = tool.execute("{}", &std::env::temp_dir()).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("timed out"),
            "expected a timeout error from the long-running tool"
        );
    }

    /// Regression test for #3413: subprocess must be killed on timeout,
    /// not orphaned. Uses a marker file — the process writes it after a
    /// delay; if kill_on_drop works, the file should NOT exist after
    /// the timeout fires.
    // The sh/sleep/touch marker-file mechanism is Unix-only; kill_on_drop
    // itself is cross-platform (same Rust std impl) and is exercised here
    // on Unix runners. Skipped on Windows where sh is unavailable.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_execute_timeout_kills_subprocess() {
        let marker =
            std::env::temp_dir().join(format!("vp-regression-3413-{}.tmp", std::process::id()));
        // Clean up any leftover from a previous run
        let _ = std::fs::remove_file(&marker);

        // Script: sleep 5s, then create marker file. Timeout is 1s.
        // If the process is killed on timeout, the file will never be created.
        let script = format!("sleep 5 && touch {}", marker.to_string_lossy());
        let mut tool = make_tool("slow_marker", &format!("sh -c '{}'", script));
        tool.timeout_seconds = 1;

        let result = tool.execute("{}", &std::env::temp_dir()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));

        // Wait 7 seconds — enough time for the orphaned process (if still
        // running) to have created the marker file.
        tokio::time::sleep(Duration::from_secs(7)).await;

        // The marker file must NOT exist — proving the subprocess was killed.
        assert!(
            !marker.exists(),
            "subprocess was not killed on timeout — orphan process created marker file (regression #3413)"
        );
    }

    #[tokio::test]
    async fn test_execute_failure_exit_code() {
        // Exit non-zero with "error" on stderr: `sh -c` on Unix; PowerShell on
        // Windows (avoids cmd.exe `/C` quote-stripping quirks around `>`/`&`).
        #[cfg(unix)]
        let tool = make_tool("fail_tool", "sh -c 'echo error >&2; exit 1'");
        #[cfg(windows)]
        let tool = make_tool(
            "fail_tool",
            r#"powershell -NoProfile -Command "[Console]::Error.WriteLine('error'); exit 1""#,
        );
        let result = tool.execute("{}", &std::env::temp_dir()).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("error"),
            "expected the subprocess stderr to be surfaced in the error"
        );
    }

    #[test]
    fn test_split_command_simple() {
        let tool = make_tool("test", "echo hello");
        let (prog, args) = tool.split_command().unwrap();
        assert_eq!(prog, "echo");
        assert_eq!(args, vec!["hello"]);
    }

    #[test]
    fn test_split_command_quoted() {
        let tool = make_tool("test", r#"python3 -c "print('hi')""#);
        let (prog, args) = tool.split_command().unwrap();
        assert_eq!(prog, "python3");
        assert_eq!(args, vec!["-c", "print('hi')"]);
    }
}
