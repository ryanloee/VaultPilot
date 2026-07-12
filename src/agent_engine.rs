//! Multi-Agent Engine Adapter Layer — external agent integration (#1996).
//!
//! VaultPilot's self-built agent lives in [`crate::agent`] and exposes a
//! `run_agent()` loop that drives the configured LLM through VaultPilot's own
//! tool proxies. This module is a **separate, coexisting abstraction** that lets
//! *external* CLI agents — such as Anthropic's `claude` / `claude-code` and
//! OpenAI's `codex` — also operate inside a vault, behind a single uniform
//! interface. The builtin `run_agent()` is untouched.
//!
//! # Design
//!
//! The layer is built around three small pieces:
//!
//! 1. [`AgentEngine`] — a trait with the unified lifecycle
//!    `available → send_prompt → response`. Every engine (builtin or external)
//!    implements it.
//! 2. [`EngineContext`] — the per-invocation context: the vault directory (used
//!    as the agent's working directory / sandbox root), the enabled
//!    capabilities, an optional system preamble, and resource limits reused from
//!    [`crate::agent::AgentResourceLimits`].
//! 3. [`AgentEngineRegistry`] — lists/selects engines by name.
//!
//! External engines are subprocess-based: the CLI binary is located on `PATH`,
//! spawned with `cwd = vault_dir` (vault-scoped), receives the composed prompt
//! (preamble + capabilities + user task) and the vault context, and its stdout
//! is captured into an [`EngineResponse`]. If the backing binary is not
//! installed, [`AgentEngine::available`] returns `false` and
//! [`AgentEngine::send_prompt`] returns a clear error — the crate still
//! compiles and the unit tests never spawn real agent CLIs.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::agent::AgentResourceLimits;

/// Close a pipe handle on Windows via FFI.
/// Equivalent to `CloseHandle` from `kernel32.dll`.
#[cfg(windows)]
unsafe fn close_windows_handle(handle: *mut std::ffi::c_void) {
    extern "system" {
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }
    CloseHandle(handle);
}

/// Default system preamble injected into the composed prompt unless the caller
/// overrides it. It orientates the external agent to the vault and to
/// VaultPilot's capability model.
pub const DEFAULT_SYSTEM_PREAMBLE: &str = "\
You are operating inside a VaultPilot vault. Stay within the current working \
directory (the vault root). Only use the capabilities listed below; do not \
modify files outside the vault. Be concise and ground every claim in vault \
contents.";

// ── Context ───────────────────────────────────────────────────────────────

/// Per-invocation context handed to every agent engine.
///
/// `vault_dir` is the most important field: it becomes the agent subprocess's
/// working directory, confining file operations to the vault. `capabilities`
/// and `system_preamble` are injected into the prompt so an external agent
/// behaves consistently with VaultPilot's skill/MCP model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineContext {
    /// Vault root — used as the agent's `cwd` (sandbox root).
    pub vault_dir: PathBuf,
    /// Enabled capabilities/skills shared across agents (e.g. `search_notes`,
    /// `write_note`, `mcp:*`).
    pub capabilities: Vec<String>,
    /// Optional system preamble prepended to every composed prompt.
    #[serde(default)]
    pub system_preamble: Option<String>,
    /// Resource limits reused from the builtin agent configuration.
    #[serde(default)]
    pub limits: AgentResourceLimits,
}

impl EngineContext {
    /// Create a context rooted at `vault_dir` with default limits.
    pub fn new(vault_dir: impl Into<PathBuf>) -> Self {
        Self {
            vault_dir: vault_dir.into(),
            capabilities: Vec::new(),
            system_preamble: None,
            limits: AgentResourceLimits::default(),
        }
    }

    /// Builder: set the enabled capabilities.
    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Builder: set the system preamble.
    pub fn with_preamble(mut self, preamble: impl Into<String>) -> Self {
        self.system_preamble = Some(preamble.into());
        self
    }

    /// Builder: override the resource limits.
    pub fn with_limits(mut self, limits: AgentResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Validate that `vault_dir` exists and is a directory.
    ///
    /// Engines call this before spawning so a missing vault produces a clear
    /// error rather than a subprocess failure.
    pub fn validate(&self) -> Result<()> {
        if !self.vault_dir.is_dir() {
            bail!(
                "vault_dir does not exist or is not a directory: {}",
                self.vault_dir.display()
            );
        }
        Ok(())
    }

    /// Compose the full text fed to the agent: the (optional) system preamble,
    /// the enabled-capability list, and finally the user's task prompt.
    ///
    /// This is what gets written to the agent's stdin (or passed as a prompt
    /// argument).
    pub fn compose_prompt(&self, prompt: &str) -> String {
        let mut out = String::new();
        if let Some(preamble) = &self.system_preamble {
            out.push_str(preamble);
            out.push_str("\n\n");
        }
        if !self.capabilities.is_empty() {
            out.push_str("# Enabled capabilities\n");
            for cap in &self.capabilities {
                out.push_str("- ");
                out.push_str(cap);
                out.push('\n');
            }
            out.push('\n');
        }
        out.push_str("# Task\n");
        out.push_str(prompt);
        out
    }
}

// ── Response / events ─────────────────────────────────────────────────────

/// Kind of event emitted while an engine runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EngineEventKind {
    /// The agent subprocess was spawned.
    Started,
    /// A chunk captured from the agent's stdout.
    Stdout,
    /// A chunk captured from the agent's stderr.
    Stderr,
    /// The agent finished successfully.
    Completed,
    /// The agent failed (non-zero exit / spawn error).
    Failed,
}

/// A single captured event from an engine run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineEvent {
    pub kind: EngineEventKind,
    pub message: String,
}

impl EngineEvent {
    fn started() -> Self {
        Self {
            kind: EngineEventKind::Started,
            message: "agent subprocess started".to_string(),
        }
    }
    fn stdout(msg: impl Into<String>) -> Self {
        Self {
            kind: EngineEventKind::Stdout,
            message: msg.into(),
        }
    }
    fn stderr(msg: impl Into<String>) -> Self {
        Self {
            kind: EngineEventKind::Stderr,
            message: msg.into(),
        }
    }
    fn completed() -> Self {
        Self {
            kind: EngineEventKind::Completed,
            message: "agent subprocess exited successfully".to_string(),
        }
    }
    fn failed() -> Self {
        Self {
            kind: EngineEventKind::Failed,
            message: "agent subprocess exited with non-zero status".to_string(),
        }
    }
}

/// The outcome of a single [`AgentEngine::send_prompt`] call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineResponse {
    /// Name of the engine that produced this response.
    pub engine: String,
    /// Captured stdout (the agent's primary answer channel).
    pub stdout: String,
    /// Process exit code, if available.
    pub exit_status: Option<i32>,
    /// Ordered events captured during the run.
    pub events: Vec<EngineEvent>,
}

// ── Engine trait ──────────────────────────────────────────────────────────

/// Uniform interface for an agent engine — builtin or external CLI.
///
/// Lifecycle: check [`AgentEngine::available`] (e.g. is the CLI installed?),
/// then call [`AgentEngine::send_prompt`] with a prompt and an
/// [`EngineContext`].
pub trait AgentEngine: Send {
    /// Stable, lowercase engine identifier (e.g. `claude-code`, `codex`,
    /// `builtin`).
    fn name(&self) -> &str;

    /// Whether this engine is ready to run *right now* — for subprocess engines
    /// this is true only when the backing CLI binary is found on `PATH`.
    fn available(&self) -> bool;

    /// Short human-readable description.
    fn description(&self) -> &str;

    /// Run `prompt` against this engine within `ctx`. Returns the captured
    /// response or a clear error (e.g. binary missing, vault invalid).
    ///
    /// # Failure semantics (subprocess engines)
    ///
    /// For subprocess-backed engines, `send_prompt` returns `Err` when the
    /// external agent either **times out** (exceeds
    /// [`EngineContext::limits`].`max_duration`) or **exits with a non-zero
    /// status** — rather than masking those as success. The captured
    /// stdout/stderr are preserved in the error message, so callers (including
    /// the `agent-engine run` CLI) observe a genuine failure and a non-zero
    /// process exit code (#2284 / #2285).
    fn send_prompt(&mut self, prompt: &str, ctx: &EngineContext) -> Result<EngineResponse>;
}

// ── Subprocess engine ─────────────────────────────────────────────────────

/// A subprocess-backed agent engine.
///
/// This is the reusable implementation behind the Claude Code and Codex
/// adapters (and is also used directly by the hermetic tests with a safe shim
/// such as `sh`). The engine resolves one of `binary_names` on `PATH`, spawns it
/// with `cwd = vault_dir`, optionally pipes the composed prompt to stdin, and
/// captures stdout/stderr.
///
/// `ClaudeCodeEngine` and `CodexEngine` are type aliases for this struct,
/// pre-configured by the registry factories.
pub struct SubprocessEngine {
    engine_name: String,
    binary_names: Vec<String>,
    extra_args: Vec<String>,
    /// When true, the composed prompt is written to the child's stdin.
    pass_prompt_via_stdin: bool,
    description: String,
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Set the pipe file descriptor `O_NONBLOCK` so that a read loop can avoid
/// hanging indefinitely when grandchild processes inherit the write end of
/// the pipe (#2364).
///
/// Returns an error if `fcntl` fails, so callers can abort the subprocess
/// upfront rather than risk a permanent thread hang (#2541).
#[cfg(unix)]
fn make_nonblocking<T: std::os::unix::io::AsRawFd>(handle: &T) -> Result<()> {
    let fd = handle.as_raw_fd();
    // Safety: `fcntl` is safe as long as `fd` is valid and we pass valid
    // arguments. `fd` came from a valid OS pipe handle.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags >= 0 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            let err = std::io::Error::last_os_error();
            return Err(anyhow::anyhow!(
                "make_nonblocking: failed to set O_NONBLOCK on fd {fd}: {err}"
            ));
        }
    }
    Ok(())
}

impl SubprocessEngine {
    /// Create a new subprocess engine.
    pub fn new(
        engine_name: impl Into<String>,
        binary_names: Vec<String>,
        extra_args: Vec<String>,
        pass_prompt_via_stdin: bool,
        description: impl Into<String>,
    ) -> Self {
        Self {
            engine_name: engine_name.into(),
            binary_names,
            extra_args,
            pass_prompt_via_stdin,
            description: description.into(),
        }
    }

    /// Resolve the backing binary on `PATH`, trying `binary_names` in order.
    fn binary(&self) -> Option<PathBuf> {
        find_binary(&self.binary_names)
    }

    /// Build the [`Command`] to spawn (vault-scoped) for the given resolved
    /// binary and context. Separated from [`Self::run`] so the construction is
    /// independently testable.
    fn build_command(binary: &Path, engine: &SubprocessEngine, ctx: &EngineContext) -> Command {
        let mut cmd = Command::new(binary);
        // Vault-scoped execution: the agent's cwd is the vault root.
        cmd.current_dir(&ctx.vault_dir);
        cmd.args(&engine.extra_args);
        // Inject vault context so the agent can read it programmatically.
        cmd.env("VAULTPILOT_VAULT_DIR", &ctx.vault_dir);
        if !ctx.capabilities.is_empty() {
            cmd.env("VAULTPILOT_CAPABILITIES", ctx.capabilities.join(","));
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd
    }

    /// Spawn the resolved binary, feed it the prompt, and capture output.
    ///
    /// # Resource-limit & failure contract (#2284 / #2285)
    ///
    /// - **Timeout (`ctx.limits.max_duration`)**: the spawned child is killed
    ///   and reaped once the deadline elapses, and this method returns `Err`.
    ///   A hung external agent therefore can never block the caller
    ///   indefinitely — mirroring the builtin agent's pattern in
    ///   [`crate::agent::Agent::run_command`] (`tokio::select!` + kill).
    /// - **Non-zero exit**: a failing agent (e.g. `exit 7`) is surfaced as
    ///   `Err`, with the captured stdout/stderr preserved in the error
    ///   message, rather than masked as `Ok`. Callers — including the
    ///   `agent-engine run` CLI — consequently observe a real failure and a
    ///   non-zero process exit code.
    fn run(&self, binary: &Path, prompt: &str, ctx: &EngineContext) -> Result<EngineResponse> {
        use std::io::Read;
        use std::sync::mpsc;
        use wait_timeout::ChildExt;

        // Upper bound for draining the child's pipes once it has ended. A
        // grandchild that inherited the pipes could otherwise keep a drain open
        // indefinitely; this mirrors the builtin agent's `io_timeout`.
        const IO_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

        let composed = ctx.compose_prompt(prompt);
        let mut cmd = Self::build_command(binary, self, ctx);

        let mut child = cmd.spawn().with_context(|| {
            format!(
                "failed to spawn '{}' ({})",
                self.engine_name,
                binary.display()
            )
        })?;

        // Concurrently drain stdout/stderr so a full OS pipe buffer cannot
        // deadlock the child while we enforce the deadline below. Results come
        // back over channels so collection can be time-bounded.
        //
        // #2364 — use NON-BLOCKING I/O + cancellation flag. When the agent
        // subprocess creates grandchildren that inherit the pipe write ends,
        // the write end stays open after the child is killed. Blocking
        // `read_to_end` would hang forever, leaking OS threads. Instead, we
        // set the pipe FDs to non-blocking mode and loop with short sleeps;
        // the `drain_done` flag (set after the child is reaped) terminates
        // the drain threads promptly.
        //
        // #2428 — the drain threads are spawned BEFORE feeding the prompt to
        // stdin. Writing first could block forever on `write_all` if the
        // prompt exceeds the OS pipe buffer (~64 KB on Linux) while the child
        // blocks on a full stdout pipe that nobody is draining — a classic
        // subprocess deadlock. With the drains running first, stdout/stderr
        // can flow while we feed stdin below.
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        // #2541 — set O_NONBLOCK on pipe read ends BEFORE spawning drain
        // threads, so a failure here kills the child and reports an error
        // rather than risking a permanent thread hang later.
        #[cfg(unix)]
        {
            if let Some(ref handle) = stdout_handle {
                if let Err(e) = make_nonblocking(handle)
                    .with_context(|| "failed to set O_NONBLOCK on stdout pipe")
                {
                    // Kill and reap the child before propagating, otherwise
                    // the child becomes an orphan/zombie process.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e);
                }
            }
            if let Some(ref handle) = stderr_handle {
                if let Err(e) = make_nonblocking(handle)
                    .with_context(|| "failed to set O_NONBLOCK on stderr pipe")
                {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e);
                }
            }
        }

        let (out_tx, out_rx) = mpsc::channel::<String>();
        let (err_tx, err_rx) = mpsc::channel::<String>();

        let drain_done = Arc::new(AtomicBool::new(false));

        let done = drain_done.clone();
        let stdout_thread = match std::thread::Builder::new()
            .name("agent-stdout-drain".into())
            .spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut s) = stdout_handle {
                    let mut tmp = [0u8; 4096];
                    loop {
                        if done.load(Ordering::Acquire) {
                            // #2746 — drain ALL residual data in the pipe
                            // buffer. A single 4 KB read cannot capture bursts
                            // larger than 4 KB (e.g. normal-exit burst or
                            // grandchild residual #2440). Loop until WouldBlock
                            // or EOF — safe because the FD is non-blocking
                            // (#2541) so this never hangs.
                            loop {
                                match s.read(&mut tmp) {
                                    Ok(0) => break,
                                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                                    Err(ref e)
                                        if e.kind() == std::io::ErrorKind::WouldBlock
                                            || e.kind() == std::io::ErrorKind::Interrupted =>
                                    {
                                        break;
                                    }
                                    Err(ref e) => {
                                        tracing::warn!(
                                            "[agent_engine] stdout final drain \
                                             read failed: {e}"
                                        );
                                        break;
                                    }
                                }
                            }
                            break;
                        }
                        match s.read(&mut tmp) {
                            Ok(0) => break,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                            Err(ref e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    || e.kind() == std::io::ErrorKind::Interrupted =>
                            {
                                std::thread::sleep(Duration::from_millis(10));
                                continue;
                            }
                            // #2414 — log unexpected read errors (e.g. a broken
                            // pipe from an abruptly-closed write end) so any
                            // truncation of the child's output is visible in
                            // diagnostics instead of silently discarded.
                            Err(ref e) => {
                                tracing::warn!(
                                    "[agent_engine] stdout drain stopped on \
                                     unexpected read error (output may be \
                                     truncated): {e}"
                                );
                                break;
                            }
                        }
                    }
                }
                let _ = out_tx.send(String::from_utf8_lossy(&buf).into_owned());
            }) {
            Ok(handle) => handle,
            Err(e) => {
                // #2427 — no drain thread is running yet, but the child must
                // not be orphaned. Kill and reap it before propagating.
                let _ = child.kill();
                let _ = child.wait();
                return Err(e).with_context(|| "failed to spawn stdout drain thread");
            }
        };
        let done = drain_done.clone();
        let stderr_thread = match std::thread::Builder::new()
            .name("agent-stderr-drain".into())
            .spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut s) = stderr_handle {
                    let mut tmp = [0u8; 4096];
                    loop {
                        if done.load(Ordering::Acquire) {
                            // #2746 — drain ALL residual data in the pipe
                            // buffer. A single 4 KB read cannot capture bursts
                            // larger than 4 KB (e.g. normal-exit burst or
                            // grandchild residual #2440). Loop until WouldBlock
                            // or EOF — safe because the FD is non-blocking
                            // (#2541) so this never hangs.
                            loop {
                                match s.read(&mut tmp) {
                                    Ok(0) => break,
                                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                                    Err(ref e)
                                        if e.kind() == std::io::ErrorKind::WouldBlock
                                            || e.kind() == std::io::ErrorKind::Interrupted =>
                                    {
                                        break;
                                    }
                                    Err(ref e) => {
                                        tracing::warn!(
                                            "[agent_engine] stderr final drain \
                                             read failed: {e}"
                                        );
                                        break;
                                    }
                                }
                            }
                            break;
                        }
                        match s.read(&mut tmp) {
                            Ok(0) => break,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                            Err(ref e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    || e.kind() == std::io::ErrorKind::Interrupted =>
                            {
                                std::thread::sleep(Duration::from_millis(10));
                                continue;
                            }
                            // #2414 — log unexpected read errors (e.g. a broken
                            // pipe from an abruptly-closed write end) so any
                            // truncation of the child's output is visible in
                            // diagnostics instead of silently discarded.
                            Err(ref e) => {
                                tracing::warn!(
                                    "[agent_engine] stderr drain stopped on \
                                     unexpected read error (output may be \
                                     truncated): {e}"
                                );
                                break;
                            }
                        }
                    }
                }
                let _ = err_tx.send(String::from_utf8_lossy(&buf).into_owned());
            }) {
            Ok(handle) => handle,
            Err(e) => {
                // #2427 — the stdout drain is already running. Signal it to
                // stop via `drain_done`, kill/reap the child to avoid a zombie,
                // join the stdout thread first (so it finishes sending its
                // data), then drain the channel. The previous code called
                // `recv_timeout` BEFORE `join`, which could waste the entire
                // 5-second timeout waiting on data the thread hadn't finished
                // producing yet — and the data was discarded anyway (#2473).
                drain_done.store(true, Ordering::Release);
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = out_rx.recv_timeout(IO_DRAIN_TIMEOUT);
                return Err(e).with_context(|| "failed to spawn stderr drain thread");
            }
        };

        // #2428 — feed the prompt to stdin AFTER the drain threads are running
        // so a large prompt cannot deadlock against an un-drained stdout pipe.
        // Best-effort write; a closed stdin is not fatal. Taking + dropping
        // stdin here sends EOF to the child promptly.
        //
        // #2484 — protect the write with a timeout. If the child does not read
        // stdin, `write_all` blocks indefinitely once the OS pipe buffer (~64 KB
        // on Linux) fills up. We spawn a dedicated thread and use a channel with
        // `recv_timeout` so the caller is never permanently blocked.
        if self.pass_prompt_via_stdin {
            if let Some(stdin) = child.stdin.take() {
                use std::io::Write;

                // Platform-specific raw handle capture.
                // On Unix we need the raw fd so we can close the pipe on timeout
                // (forcing BrokenPipe in the blocked thread). On Windows we use
                // CloseHandle via FFI for the same purpose.
                #[cfg(unix)]
                use std::os::unix::io::AsRawFd;
                #[cfg(windows)]
                use std::os::windows::io::AsRawHandle;

                #[cfg(unix)]
                let stdin_raw = stdin.as_raw_fd();
                #[cfg(windows)]
                let stdin_raw = stdin.as_raw_handle();

                let (tx, rx) = mpsc::channel::<std::io::Result<()>>();
                let std_in_thread = match std::thread::Builder::new()
                    .name("agent-stdin-write".into())
                    .spawn(move || {
                        // #2509 — wrap stdin in ManuallyDrop to prevent a
                        // double-close race. When the parent closes the raw fd
                        // on timeout (via libc::close), the thread's write_all
                        // receives BrokenPipe, and the closure ends. Without
                        // ManuallyDrop, ChildStdin::Drop would then call
                        // close() on the same fd — which may already have been
                        // reused by another thread.
                        use std::mem::ManuallyDrop;
                        let stdin = ManuallyDrop::new(stdin);
                        let result = (&*stdin).write_all(composed.as_bytes());
                        // If write succeeded, take stdin back and drop it
                        // (closes fd, sends EOF to child). If write failed
                        // (e.g. BrokenPipe from timeout close), forget stdin
                        // — the parent already closed (or will close) the fd.
                        if result.is_ok() {
                            drop(ManuallyDrop::into_inner(stdin));
                        }
                        let _ = tx.send(result);
                    }) {
                    Ok(handle) => handle,
                    Err(e) => {
                        // #2493 — the stdin-write thread could not be spawned
                        // (e.g. resource exhaustion on a heavily loaded system).
                        // Signal drain threads to stop, kill/reap the child to
                        // avoid a zombie, join both drain threads so they finish
                        // sending their data, then drain the channels.
                        drain_done.store(true, Ordering::Release);
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = stdout_thread.join();
                        let _ = stderr_thread.join();
                        let _ = out_rx.recv_timeout(IO_DRAIN_TIMEOUT);
                        let _ = err_rx.recv_timeout(IO_DRAIN_TIMEOUT);
                        return Err(e).with_context(|| "failed to spawn stdin write thread");
                    }
                };
                match rx.recv_timeout(IO_DRAIN_TIMEOUT) {
                    Ok(Ok(())) => {
                        /* written successfully — thread already closed stdin */
                        if let Err(e) = std_in_thread.join() {
                            tracing::warn!(
                                "[agent_engine] stdin write thread panicked (after success): {e:?}",
                            );
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            "[agent_engine] stdin write failed for '{}': {e}",
                            self.engine_name,
                        );
                        if let Err(e) = std_in_thread.join() {
                            tracing::warn!(
                                "[agent_engine] stdin write thread panicked (after error): {e:?}",
                            );
                        }
                        // Write failed before timeout; ManuallyDrop prevented
                        // the thread from closing stdin. Close it here.
                        #[cfg(unix)]
                        unsafe {
                            libc::close(stdin_raw);
                        }
                        #[cfg(windows)]
                        unsafe {
                            close_windows_handle(stdin_raw);
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        tracing::warn!(
                            "[agent_engine] stdin write timed out for '{}' after {}.{:03}s \
                             — child may not be reading stdin",
                            self.engine_name,
                            IO_DRAIN_TIMEOUT.as_secs(),
                            IO_DRAIN_TIMEOUT.subsec_millis(),
                        );
                        // #2497 — close the pipe from this side to force BrokenPipe
                        // in the blocked thread, then join it to reclaim the OS
                        // resources (preventing unbounded thread leaks).
                        #[cfg(unix)]
                        unsafe {
                            libc::close(stdin_raw);
                        }
                        #[cfg(windows)]
                        unsafe {
                            close_windows_handle(stdin_raw);
                        }
                        if let Err(e) = std_in_thread.join() {
                            tracing::warn!(
                                "[agent_engine] stdin write thread panicked (after timeout): {e:?}",
                            );
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        tracing::warn!(
                            "[agent_engine] stdin write thread disconnected for '{}'",
                            self.engine_name,
                        );
                        if let Err(e) = std_in_thread.join() {
                            tracing::warn!(
                                "[agent_engine] stdin write thread panicked (after disconnect): {e:?}",
                            );
                        }
                        // Thread may not have closed stdin; close it here.
                        #[cfg(unix)]
                        unsafe {
                            libc::close(stdin_raw);
                        }
                        #[cfg(windows)]
                        unsafe {
                            close_windows_handle(stdin_raw);
                        }
                    }
                }
            }
        } else {
            // #2510 — close stdin immediately when not piping the prompt.
            // Without this, the pipe write end stays open until `child` is
            // dropped at the end of `run()`, which could cause the child to
            // hang if it reads stdin waiting for EOF.
            drop(child.stdin.take());
        }

        // Enforce the wall-clock deadline (#2284). On timeout the child is
        // killed and reaped (no zombies) and the call returns a clear error.
        let max_duration = ctx.limits.max_duration;
        match child.wait_timeout(max_duration) {
            Ok(Some(status)) => {
                // Normal exit — child exited on its own.
                // Signal drain threads to stop, join them (so they finish
                // sending their data), then drain the channels. This order
                // ensures no data is lost even when grandchildren hold pipe
                // write ends (#2440, #2442).
                drain_done.store(true, Ordering::Release);
                if let Err(e) = stdout_thread.join() {
                    tracing::warn!("[agent_engine] stdout drain thread panicked: {e:?}");
                }
                if let Err(e) = stderr_thread.join() {
                    tracing::warn!("[agent_engine] stderr drain thread panicked: {e:?}");
                }
                let stdout = match out_rx.recv_timeout(IO_DRAIN_TIMEOUT) {
                    Ok(s) => s,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        tracing::warn!("[agent_engine] stdout drain timed out after normal exit");
                        String::new()
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        tracing::warn!("[agent_engine] stdout channel disconnected — drain thread may have panicked");
                        String::new()
                    }
                };
                let stderr = match err_rx.recv_timeout(IO_DRAIN_TIMEOUT) {
                    Ok(s) => s,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        tracing::warn!("[agent_engine] stderr drain timed out after normal exit");
                        String::new()
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        tracing::warn!("[agent_engine] stderr channel disconnected — drain thread may have panicked");
                        String::new()
                    }
                };

                let code = status.code();
                let success = status.success();

                let mut events = vec![EngineEvent::started()];
                if !stdout.is_empty() {
                    events.push(EngineEvent::stdout(&stdout));
                }
                if !stderr.is_empty() {
                    events.push(EngineEvent::stderr(&stderr));
                }
                events.push(if success {
                    EngineEvent::completed()
                } else {
                    EngineEvent::failed()
                });

                // A non-zero exit is a real failure: propagate it (preserving
                // the captured stdout/stderr) instead of masking it as success.
                if !success {
                    bail!(
                        "agent engine '{}' exited with non-zero status {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                        self.engine_name,
                        code,
                        stdout,
                        stderr
                    );
                }

                Ok(EngineResponse {
                    engine: self.engine_name.clone(),
                    stdout,
                    exit_status: code,
                    events,
                })
            }
            Ok(None) => {
                // Timed out — child still running.
                let _ = child.kill();
                let _ = child.wait();
                // Signal drain threads to stop and join them so the OS
                // threads do not accumulate across calls (#2442).
                drain_done.store(true, Ordering::Release);
                if let Err(e) = stdout_thread.join() {
                    tracing::warn!("[agent_engine] stdout drain thread panicked on timeout: {e:?}");
                }
                if let Err(e) = stderr_thread.join() {
                    tracing::warn!("[agent_engine] stderr drain thread panicked on timeout: {e:?}");
                }
                // Drain for diagnostics; the timeout error is what
                // propagates.
                if let Err(e) = out_rx.recv_timeout(IO_DRAIN_TIMEOUT) {
                    tracing::warn!(
                        "[agent_engine] stdout drain timed out after kill — \
                         thread may be blocked on a non-blocking FD: {e:?}"
                    );
                }
                if let Err(e) = err_rx.recv_timeout(IO_DRAIN_TIMEOUT) {
                    tracing::warn!(
                        "[agent_engine] stderr drain timed out after kill — \
                         thread may be blocked on a non-blocking FD: {e:?}"
                    );
                }
                bail!(
                    "agent engine '{}' timed out after {:?}",
                    self.engine_name,
                    max_duration
                );
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                drain_done.store(true, Ordering::Release);
                if let Err(e) = stdout_thread.join() {
                    tracing::warn!("[agent_engine] stdout drain thread panicked on error: {e:?}");
                }
                if let Err(e) = stderr_thread.join() {
                    tracing::warn!("[agent_engine] stderr drain thread panicked on error: {e:?}");
                }
                if let Err(e) = out_rx.recv_timeout(IO_DRAIN_TIMEOUT) {
                    tracing::warn!(
                        "[agent_engine] stdout drain timed out after error — \
                         thread may be blocked on a non-blocking FD: {e:?}"
                    );
                }
                if let Err(e) = err_rx.recv_timeout(IO_DRAIN_TIMEOUT) {
                    tracing::warn!(
                        "[agent_engine] stderr drain timed out after error — \
                         thread may be blocked on a non-blocking FD: {e:?}"
                    );
                }
                bail!(
                    "failed to wait for agent engine '{}': {}",
                    self.engine_name,
                    e
                );
            }
        }
    }
}

impl AgentEngine for SubprocessEngine {
    fn name(&self) -> &str {
        &self.engine_name
    }

    fn available(&self) -> bool {
        self.binary().is_some()
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn send_prompt(&mut self, prompt: &str, ctx: &EngineContext) -> Result<EngineResponse> {
        ctx.validate()?;
        let binary = self.binary().ok_or_else(|| {
            anyhow::anyhow!(
                "agent engine '{}' is not available: none of its CLI binaries were found on PATH ({})",
                self.engine_name,
                self.binary_names.join(", ")
            )
        })?;
        self.run(&binary, prompt, ctx)
    }
}

/// Claude Code engine — spawns `claude` / `claude-code` in the vault dir.
pub type ClaudeCodeEngine = SubprocessEngine;

/// Codex engine — spawns `codex` in the vault dir.
pub type CodexEngine = SubprocessEngine;

/// Build a [`ClaudeCodeEngine`] configured for the `claude` / `claude-code` CLIs.
///
/// The prompt is delivered over stdin and the agent runs in print mode
/// (`--print`) so it emits its answer to stdout and exits.
pub fn claude_code_engine() -> ClaudeCodeEngine {
    SubprocessEngine::new(
        "claude-code",
        vec!["claude".to_string(), "claude-code".to_string()],
        vec!["--print".to_string()],
        true,
        "Anthropic Claude Code CLI — runs inside the vault sandbox",
    )
}

/// Build a [`CodexEngine`] configured for the `codex` CLI.
///
/// `codex exec` runs a single prompt non-interactively and prints the result.
pub fn codex_engine() -> CodexEngine {
    SubprocessEngine::new(
        "codex",
        vec!["codex".to_string()],
        vec!["exec".to_string()],
        true,
        "OpenAI Codex CLI — runs inside the vault sandbox",
    )
}

// ── Builtin engine ────────────────────────────────────────────────────────

/// The builtin engine — a coherent placeholder that points back at the existing
/// self-built [`crate::agent::run_agent`] loop.
///
/// The builtin agent is driven by its own command (`agent`) and async runtime
/// dependencies, so it is intentionally **not** re-spawned through this
/// subprocess adapter. `available()` returns `false` (the engine does not
/// implement `send_prompt` directly) and `send_prompt` returns a clear,
/// actionable error directing the caller to the `agent` command. This keeps
/// the builtin and external engines selectable through one registry without
/// duplicating the builtin execution path.
///
/// # Contract note
///
/// Unlike subprocess engines where `available() == true` guarantees
/// `send_prompt()` will succeed, `BuiltinEngine` always returns `false` from
/// `available()` because it cannot service `send_prompt` calls — it redirects
/// callers to the dedicated `agent` command instead.
pub struct BuiltinEngine;

impl AgentEngine for BuiltinEngine {
    fn name(&self) -> &str {
        "builtin"
    }

    fn available(&self) -> bool {
        false
    }

    fn description(&self) -> &str {
        "VaultPilot's self-built agent loop (run_agent) — invoke via the `agent` command"
    }

    fn send_prompt(&mut self, _prompt: &str, _ctx: &EngineContext) -> Result<EngineResponse> {
        bail!(
            "the builtin engine runs through the existing `agent` command; \
             select 'claude-code' or 'codex' to run an external engine via this adapter"
        );
    }
}

// ── Registry ──────────────────────────────────────────────────────────────

/// Lightweight, serializable summary of a registered engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInfo {
    pub name: String,
    pub available: bool,
    pub description: String,
}

/// Registry of all known agent engines. Lets the caller enumerate engines and
/// pick one by name.
#[derive(Default)]
pub struct AgentEngineRegistry;

impl AgentEngineRegistry {
    /// Create a registry. Engines are constructed on demand (see
    /// [`Self::list_engines`] / [`Self::select`]) so registration is cheap.
    pub fn new() -> Self {
        Self
    }

    /// Instantiate every known engine. Order is stable: builtin, claude-code,
    /// codex.
    pub fn list_engines(&self) -> Vec<Box<dyn AgentEngine>> {
        vec![
            Box::new(BuiltinEngine),
            Box::new(claude_code_engine()),
            Box::new(codex_engine()),
        ]
    }

    /// Summary of every engine (name + availability + description).
    pub fn engine_infos(&self) -> Vec<EngineInfo> {
        self.list_engines()
            .into_iter()
            .map(|e| EngineInfo {
                name: e.name().to_string(),
                available: e.available(),
                description: e.description().to_string(),
            })
            .collect()
    }

    /// Select a single engine by name (case-insensitive), or `None` if unknown.
    pub fn select(&self, name: &str) -> Option<Box<dyn AgentEngine>> {
        let target = name.trim().to_ascii_lowercase();
        self.list_engines()
            .into_iter()
            .find(|e| e.name().eq_ignore_ascii_case(&target))
    }
}

// ── Binary resolution ─────────────────────────────────────────────────────

/// Resolve the first of `names` found on `PATH`. Returns `None` if none exist.
///
/// This is a minimal, dependency-free `which(1)` that also accepts an explicit
/// relative/absolute path in `names`.
pub fn find_binary(names: &[String]) -> Option<PathBuf> {
    for name in names {
        if let Some(found) = which(name) {
            return Some(found);
        }
    }
    None
}

/// Look up a single command on `PATH` (or use it directly if it is a path).
fn which(name: &str) -> Option<PathBuf> {
    // If the caller gave a path (contains a separator), check it directly.
    let looks_like_path = name.contains(std::path::MAIN_SEPARATOR);
    #[cfg(windows)]
    let looks_like_path = looks_like_path || name.contains('/');
    if looks_like_path {
        let candidate = PathBuf::from(name);
        return if is_executable(&candidate) {
            Some(candidate)
        } else {
            None
        };
    }

    let path_os = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_os) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        // On Windows, also probe common executable extensions.
        #[cfg(windows)]
        {
            for ext in ["exe", "bat", "cmd", "com"] {
                let with_ext = dir.join(format!("{name}.{ext}"));
                if is_executable(&with_ext) {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

/// Whether `path` is an executable file.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111 != 0),
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        std::fs::metadata(path)
            .map(|meta| meta.is_file())
            .unwrap_or(false)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    /// Build a unique temp vault directory (no `tempfile` dependency — matches
    /// the pattern used elsewhere in the crate).
    fn temp_vault() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot_agent_engine_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp vault");
        dir
    }

    /// RAII guard that removes the temp vault on drop.
    struct TempGuard(PathBuf);
    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn registry_lists_known_engines() {
        let registry = AgentEngineRegistry::new();
        let infos = registry.engine_infos();
        let names: Vec<&str> = infos.iter().map(|i| i.name.as_str()).collect();
        assert!(
            names.contains(&"builtin"),
            "builtin engine must be registered"
        );
        assert!(
            names.contains(&"claude-code"),
            "claude-code engine must be registered"
        );
        assert!(names.contains(&"codex"), "codex engine must be registered");

        // Builtin is not available via send_prompt (it redirects to the `agent` command).
        let builtin = infos.iter().find(|i| i.name == "builtin").expect("builtin");
        assert!(!builtin.available);

        // Descriptions are non-empty for every engine.
        assert!(infos.iter().all(|i| !i.description.is_empty()));
    }

    #[test]
    fn registry_select_case_insensitive() {
        let registry = AgentEngineRegistry::new();
        let selected = registry.select("Claude-Code").expect("select claude-code");
        assert_eq!(selected.name(), "claude-code");
        assert!(registry.select("does-not-exist").is_none());
    }

    #[test]
    fn missing_binary_is_unavailable_and_errors_clearly() {
        // An engine backed by a binary name that provably does not exist.
        let mut engine = SubprocessEngine::new(
            "fake-engine",
            vec!["vaultpilot_definitely_missing_binary_zzz".to_string()],
            Vec::new(),
            true,
            "deterministic missing-binary test engine",
        );

        assert!(
            !engine.available(),
            "engine must be unavailable when binary missing"
        );

        let vault = temp_vault();
        let _guard = TempGuard(vault.clone());
        let ctx = EngineContext::new(&vault);
        let err = engine.send_prompt("hello", &ctx).unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("not available") || msg.contains("not found"),
            "error should clearly explain the missing binary: {msg}"
        );
        assert!(
            msg.contains("vaultpilot_definitely_missing_binary_zzz"),
            "error should name the missing binary: {msg}"
        );
    }

    #[test]
    fn engine_context_serialization_roundtrip() {
        let ctx = EngineContext::new("/tmp/example-vault")
            .with_capabilities(vec!["search_notes".to_string(), "write_note".to_string()])
            .with_preamble("You are a vault agent.")
            .with_limits(AgentResourceLimits {
                max_duration: Duration::from_secs(42),
                max_tool_calls: 7,
                max_tokens: 1024,
            });

        let json = serde_json::to_string(&ctx).expect("serialize");
        let back: EngineContext = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.vault_dir, PathBuf::from("/tmp/example-vault"));
        assert_eq!(back.capabilities, ctx.capabilities);
        assert_eq!(back.system_preamble, ctx.system_preamble);
        assert_eq!(back.limits.max_tool_calls, 7);
        assert_eq!(back.limits.max_tokens, 1024);
        assert_eq!(back.limits.max_duration, Duration::from_secs(42));
    }

    #[test]
    fn compose_prompt_includes_preamble_and_capabilities() {
        let ctx = EngineContext::new("/tmp/v")
            .with_capabilities(vec!["search_notes".to_string()])
            .with_preamble("PREAMBLE-TEXT");
        let composed = ctx.compose_prompt("DO-THE-THING");
        assert!(composed.contains("PREAMBLE-TEXT"));
        assert!(composed.contains("search_notes"));
        assert!(composed.contains("DO-THE-THING"));
    }

    #[test]
    fn validate_rejects_missing_vault() {
        let ctx = EngineContext::new("/this/path/should/not/exist/xyz_1996");
        assert!(ctx.validate().is_err());
    }

    #[test]
    fn builtin_engine_send_prompt_returns_actionable_error() {
        let mut engine = BuiltinEngine;
        // The builtin engine is not available via send_prompt (it redirects to the `agent` command).
        assert!(!engine.available());
        let vault = temp_vault();
        let _guard = TempGuard(vault.clone());
        let ctx = EngineContext::new(&vault);
        let err = engine.send_prompt("hi", &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("builtin") && msg.contains("agent"),
            "error should redirect to the agent command: {msg}"
        );
    }

    #[test]
    #[cfg_attr(target_os = "windows", ignore)]
    fn subprocess_spawn_is_vault_scoped() {
        // Use `sh` as a safe, ubiquitous shim — no real agent CLI is spawned.
        if find_binary(&["sh".to_string()]).is_none() {
            eprintln!("skipping subprocess_spawn_is_vault_scoped: 'sh' not on PATH");
            return;
        }
        let vault = temp_vault();
        let _guard = TempGuard(vault.clone());

        let mut engine = SubprocessEngine::new(
            "sh-shim",
            vec!["sh".to_string()],
            // Print the resolved cwd so we can assert confinement.
            vec!["-c".to_string(), "pwd".to_string()],
            false, // prompt is irrelevant for the shim
            "sh shim used to verify cwd confinement",
        );
        assert!(engine.available());

        let ctx = EngineContext::new(&vault);
        let resp = engine
            .send_prompt("ignored", &ctx)
            .expect("shim should run");

        let expected = std::fs::canonicalize(&vault).expect("canonicalize vault");
        let actual = std::fs::canonicalize(resp.stdout.trim()).expect("canonicalize pwd output");
        assert_eq!(
            actual, expected,
            "agent subprocess cwd must equal the vault dir"
        );
        assert_eq!(resp.engine, "sh-shim");
        assert!(
            resp.exit_status.unwrap_or(-1) == 0,
            "shim should exit cleanly"
        );
        assert!(
            resp.events
                .iter()
                .any(|e| e.kind == EngineEventKind::Completed),
            "a completed event should be recorded"
        );
    }

    #[test]
    fn build_command_sets_vault_cwd_and_env() {
        // Verify command construction without spawning: invoke the shim with an
        // `echo` of the injected env var to prove context propagation.
        if find_binary(&["sh".to_string()]).is_none() {
            eprintln!("skipping build_command_sets_vault_cwd_and_env: 'sh' not on PATH");
            return;
        }
        let vault = temp_vault();
        let _guard = TempGuard(vault.clone());

        let engine = SubprocessEngine::new(
            "env-probe",
            vec!["sh".to_string()],
            vec![
                "-c".to_string(),
                "echo \"$VAULTPILOT_VAULT_DIR|$VAULTPILOT_CAPABILITIES\"".to_string(),
            ],
            false,
            "sh shim that prints injected env",
        );
        let caps = vec!["search_notes".to_string(), "mcp:github".to_string()];
        let ctx = EngineContext::new(&vault).with_capabilities(caps.clone());
        let binary = engine.binary().expect("sh resolved");
        let mut cmd = SubprocessEngine::build_command(&binary, &engine, &ctx);

        let output = cmd.output().expect("run probe");
        let out = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = out.trim().split('|').collect();
        assert_eq!(parts.len(), 2, "probe output shape: {out}");
        assert_eq!(
            std::fs::canonicalize(parts[0]).expect("canonicalize env vault"),
            std::fs::canonicalize(&vault).expect("canonicalize vault"),
            "VAULTPILOT_VAULT_DIR must equal the vault dir"
        );
        assert_eq!(
            parts[1],
            caps.join(","),
            "VAULTPILOT_CAPABILITIES must be the comma-joined capability list"
        );
    }

    /// #2284 — a subprocess that outlives `ctx.limits.max_duration` must be
    /// killed and surfaced as `Err`, not block the caller forever. Mirrors the
    /// builtin agent's `run_command` timeout+kill regression.
    #[test]
    #[cfg_attr(target_os = "windows", ignore)]
    fn subprocess_times_out_and_kills_child() {
        if find_binary(&["sh".to_string()]).is_none() {
            eprintln!("skipping subprocess_times_out_and_kills_child: 'sh' not on PATH");
            return;
        }
        let vault = temp_vault();
        let _guard = TempGuard(vault.clone());

        // Sleeps far longer than the deadline we will impose. `exec` makes the
        // shim replace itself with `sleep`, so killing the tracked PID reaps it
        // outright (no orphaned grandchild holding the pipe).
        let mut engine = SubprocessEngine::new(
            "sleep-shim",
            vec!["sh".to_string()],
            vec!["-c".to_string(), "exec sleep 30".to_string()],
            false,
            "sh shim that sleeps to exercise the run() deadline",
        );
        assert!(engine.available());

        let ctx = EngineContext::new(&vault).with_limits(AgentResourceLimits {
            max_duration: Duration::from_millis(150),
            max_tool_calls: 100,
            max_tokens: 0,
        });

        let start = std::time::Instant::now();
        let err = engine.send_prompt("ignored", &ctx).unwrap_err();
        let elapsed = start.elapsed();
        let msg = err.to_string();

        assert!(
            msg.contains("timed out"),
            "error must report the timeout: {msg}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "run() must return well before the shim's 30s sleep (child must be killed): {elapsed:?}"
        );
    }

    /// #2285 — a subprocess that exits non-zero must be surfaced as `Err`
    /// (with stdout/stderr preserved), never masked as `Ok`.
    #[test]
    fn subprocess_nonzero_exit_propagates_as_error() {
        if find_binary(&["sh".to_string()]).is_none() {
            eprintln!("skipping subprocess_nonzero_exit_propagates_as_error: 'sh' not on PATH");
            return;
        }
        let vault = temp_vault();
        let _guard = TempGuard(vault.clone());

        let mut engine = SubprocessEngine::new(
            "fail-shim",
            vec!["sh".to_string()],
            vec![
                "-c".to_string(),
                "echo OUT-MARKER; echo ERR-MARKER >&2; exit 7".to_string(),
            ],
            false,
            "sh shim that exits non-zero with stdout+stderr",
        );
        assert!(engine.available());

        let ctx = EngineContext::new(&vault);
        let err = engine.send_prompt("ignored", &ctx).unwrap_err();
        let msg = err.to_string();

        assert!(
            msg.contains("non-zero"),
            "error must report the non-zero exit: {msg}"
        );
        assert!(msg.contains('7'), "error must include exit code 7: {msg}");
        assert!(
            msg.contains("OUT-MARKER"),
            "error must preserve captured stdout: {msg}"
        );
        assert!(
            msg.contains("ERR-MARKER"),
            "error must preserve captured stderr: {msg}"
        );
    }

    /// #2746 — the drain threads must capture ALL residual data in the pipe
    /// buffer, not just the first 4 KB. Before the fix, the `done` branch did
    /// a single non-blocking read with a 4 KB buffer and then broke, silently
    /// truncating any output larger than 4 KB. This test writes 20 KB of
    /// unique marker data to both stdout and stderr and asserts every byte is
    /// present in the response.
    #[test]
    #[cfg_attr(target_os = "windows", ignore)]
    fn subprocess_drains_large_output_beyond_4kb() {
        if find_binary(&["sh".to_string()]).is_none() {
            eprintln!("skipping subprocess_drains_large_output_beyond_4kb: 'sh' not on PATH");
            return;
        }
        let vault = temp_vault();
        let _guard = TempGuard(vault.clone());

        // Write 20 KB (5 × 4096) of line-numbered output to stdout and stderr.
        // Each line is unique so we can verify completeness.
        let mut engine = SubprocessEngine::new(
            "large-out-shim",
            vec!["sh".to_string()],
            vec![
                "-c".to_string(),
                // 5000 lines × ~4 bytes/line ≈ 20 KB per stream.
                "i=0; while [ $i -lt 5000 ]; do echo \"OUT-$i-MARKER\"; echo \"ERR-$i-MARKER\" >&2; i=$((i + 1)); done"
                    .to_string(),
            ],
            false,
            "sh shim that writes >4 KB to stdout and stderr",
        );
        assert!(engine.available());

        let ctx = EngineContext::new(&vault);
        let resp = engine
            .send_prompt("ignored", &ctx)
            .expect("large-output shim should run");

        // Verify the FIRST and LAST markers are present — if truncation
        // occurred at 4 KB, the last markers would be missing.
        assert!(
            resp.stdout.contains("OUT-0-MARKER"),
            "stdout must contain first marker"
        );
        assert!(
            resp.stdout.contains("OUT-4999-MARKER"),
            "stdout must contain last marker (no 4KB truncation): got {} bytes",
            resp.stdout.len()
        );

        // stderr is surfaced as an EngineEvent of kind Stderr.
        let stderr: String = resp
            .events
            .iter()
            .filter(|e| e.kind == EngineEventKind::Stderr)
            .map(|e| e.message.as_str())
            .collect::<Vec<&str>>()
            .join("");
        assert!(
            stderr.contains("ERR-0-MARKER"),
            "stderr must contain first marker"
        );
        assert!(
            stderr.contains("ERR-4999-MARKER"),
            "stderr must contain last marker (no 4KB truncation): got {} bytes",
            stderr.len()
        );

        // Sanity: total captured output is well above the 4 KB single-read limit.
        assert!(
            resp.stdout.len() > 8192,
            "stdout should be >8 KB, got {} bytes",
            resp.stdout.len()
        );
        assert!(
            stderr.len() > 8192,
            "stderr should be >8 KB, got {} bytes",
            stderr.len()
        );
    }
}
