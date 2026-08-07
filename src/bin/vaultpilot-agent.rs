use std::fs;
use std::io::{self, Read, Write};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing_subscriber::EnvFilter;
use vaultpilot_lib::agent::{
    AgentConfig, AgentEvent as LibAgentEvent, AgentPermission, AgentResourceLimits, PlanDecision,
};
use vaultpilot_lib::ai::actions::{
    execute_ai_action, list_ai_actions, AiActionRequest, AiActionType,
};
use vaultpilot_lib::ai::transcription::{
    create_meeting_note, generate_meeting_summary, transcribe_audio, MeetingTranscriptionResult,
};
use vaultpilot_lib::ai::RequestUsage;
use vaultpilot_lib::diff::compute_diff;
use vaultpilot_lib::flashcards::{
    create_flashcard, get_stats, list_due_flashcards, list_flashcards, review_flashcard, Flashcard,
    FlashcardStats, ReviewRating, ReviewResult,
};
use vaultpilot_lib::models::{
    AppSettings, ChatState, ConversationSummary, ConversationTurn, NoteDocument,
};
use vaultpilot_lib::startup_stats::{PhaseTimer, StartupStats};
use vaultpilot_lib::storage::{
    create_subscription_async, delete_note_async, delete_subscription_async,
    find_related_notes_async, get_snapshot_async, get_subscription_async, import_markdown_async,
    initialize_storage_async, list_notes_async, list_snapshots_for_note_async,
    list_subscriptions_async, load_chat_state_async, load_note_async, load_settings_async,
    rebuild_index_async, restore_snapshot_async, save_chat_state_async, save_note_async,
    save_settings_async, set_subscription_enabled_with_context, update_subscription_async,
    StorageContext,
};
use vaultpilot_lib::{
    ask_with_ai_with_context, compress_chat_history_with_context, normalize_tool_path,
    run_all_due_subscriptions, run_single_subscription,
};

// ── Agent session state ─────────────────────────────────────────────────
// Allows runAgent (background task) and respondToWriteApproval (main loop)
// to coordinate write-approval decisions via a oneshot channel.

/// Shared state for the active agent session's write-approval channel.
static AGENT_APPROVAL: StdMutex<Option<std::sync::mpsc::Sender<bool>>> = StdMutex::new(None);

/// Handle of the active agent background thread. Used to:
/// - Join the agent thread on process exit so it isn't abruptly killed.
///
/// NOTE: must NOT be used as the concurrent-run guard — a fast-finishing
/// task clears it to None before the parent stores the handle, leaving a
/// stale finished handle that would reject every later runAgent call (#3788).
static ACTIVE_AGENT: StdMutex<Option<std::thread::JoinHandle<()>>> = StdMutex::new(None);

/// Concurrent-run guard, acquired atomically BEFORE the agent thread is
/// spawned and released by the thread itself when it exits (see #3788).
/// This replaces the previous check-and-set on ACTIVE_AGENT, which raced
/// when a task finished faster than the parent could store its handle.
static AGENT_RUNNING: AtomicBool = AtomicBool::new(false);

/// Set once the stdin loop hits EOF (client gone). The agent thread's
/// write-approval wait polls this and aborts (denying the write) so that
/// main()'s join() on exit cannot deadlock on an approval that will never
/// arrive (see #3788).
static AGENT_SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Shared state for the active agent's Plan Mode approval channel (#3791).
/// Allows `respondToPlanApproval` to deliver PlanDecision back to the agent
/// thread — mirroring the write-approval coordination model.
static PLAN_APPROVAL: StdMutex<Option<std::sync::mpsc::Sender<PlanDecision>>> = StdMutex::new(None);

/// Startup phase statistics for this agent process (#3910).
///
/// Populated once in `main()` right before the stdin request loop starts
/// (the loop only exits at EOF, so the value stays stable for the process
/// lifetime). Served to the WinUI client via the `startupStats` method.
static STARTUP_STATS: OnceLock<StartupStats> = OnceLock::new();

/// Shared stdout writer — ensures atomic line writes from both the main loop
/// and the background agent task.
struct SharedWriter {
    inner: StdMutex<Box<dyn Write + Send>>,
}

impl SharedWriter {
    fn write_line(&self, line: &str) {
        if let Ok(mut w) = self.inner.lock() {
            let _ = writeln!(w, "{line}");
            let _ = w.flush();
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentRequest {
    id: String,
    method: String,
    #[serde(default, rename = "params")]
    params: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentResponse {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<AgentError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentError {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentEvent<T> {
    event: String,
    payload: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentStatusPayload {
    stage: String,
    detail: String,
    timestamp: String,
}

fn main() {
    // Fine-grained startup phase timing (#3910). The timer is created at
    // the very start of main so every checkpoint is measured from process
    // entry; the finished stats are published to STARTUP_STATS right
    // before the stdin request loop starts.
    let mut startup_timer = PhaseTimer::new();

    install_panic_hook();

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("vaultpilot-agent starting");

    // Initialize configurable search rules from user's config directory
    let config_dir = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let rules_path = config_dir.join("search_rules.json");
    vaultpilot_lib::search_rules::SearchRules::init_from_file(&rules_path);
    startup_timer.checkpoint("search_rules_load");

    let stdin = io::stdin();
    let shared_writer: Arc<SharedWriter> = Arc::new(SharedWriter {
        inner: StdMutex::new(Box::new(io::stdout())),
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to initialize async runtime");
    startup_timer.checkpoint("runtime_build");

    // Create StorageContext once and reuse across all requests.
    // StorageContext already caches the SQLite connection and AppSettings
    // internally via Arc<Mutex<Option<...>>>, so reusing it avoids
    // redundant path resolution and connection setup on every request.
    let context = match StorageContext::for_sidecar() {
        Ok(ctx) => ctx,
        Err(error) => {
            eprintln!("fatal: failed to initialize storage context: {}", error);
            std::process::exit(1);
        }
    };
    startup_timer.checkpoint("storage_open");

    const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
    /// Maximum bytes allowed for a single JSON-RPC line on stdin.
    /// Prevents OOM from a malicious or buggy client sending an
    /// unbounded payload without a newline delimiter (#596).
    const MAX_LINE_BYTES: usize = 10 * 1024 * 1024; // 10 MB

    // Publish startup stats before entering the request loop. The loop only
    // exits at EOF, so this is the final checkpoint and the stats are stable
    // for the rest of the process lifetime (#3910).
    startup_timer.checkpoint("ipc_ready");
    let _ = STARTUP_STATS.set(startup_timer.finish());

    let mut stdin_buf = Vec::new();
    loop {
        stdin_buf.clear();
        // Read in 8 KB chunks until newline, enforcing the size limit during
        // reading rather than after.  `read_line()` buffers the entire line
        // before returning, making the post-read size check ineffective
        // against payloads that never contain a newline (#641).
        // Using chunked reads instead of byte-by-byte read_exact to reduce
        // syscall count from O(n) to O(n/8192) (#907).
        const CHUNK_SIZE: usize = 8 * 1024;
        let mut chunk_buf = [0u8; CHUNK_SIZE];
        let mut exceeded = false;
        let mut lock = stdin.lock();
        'read: loop {
            // Determine how many bytes we can still accept.
            let capacity_left = MAX_LINE_BYTES.saturating_sub(stdin_buf.len());
            let want = capacity_left.min(CHUNK_SIZE);
            if want == 0 {
                // Already at limit — drain byte-by-byte until newline.
                exceeded = true;
                match lock.read(&mut chunk_buf[..1]) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if chunk_buf[0] == b'\n' {
                    break;
                }
                continue;
            }
            let n = match lock.read(&mut chunk_buf[..want]) {
                Ok(0) => break 'read, // EOF
                Ok(n) => n,
                Err(_) => break 'read,
            };
            // Scan the chunk for newline.
            if let Some(nl_pos) = chunk_buf[..n].iter().position(|&b| b == b'\n') {
                let end = nl_pos + 1; // include the newline
                let room = MAX_LINE_BYTES - stdin_buf.len();
                let take = end.min(room);
                stdin_buf.extend_from_slice(&chunk_buf[..take]);
                break 'read;
            }
            // No newline found in this chunk — append the whole thing.
            stdin_buf.extend_from_slice(&chunk_buf[..n]);
        }
        if stdin_buf.is_empty() && !exceeded {
            break; // EOF
        }
        if exceeded {
            log_agent_event(
                "stdin_error",
                &format!(
                    "stdin line exceeds {}MB limit",
                    MAX_LINE_BYTES / (1024 * 1024)
                ),
            );
            let response = AgentResponse::error(
                String::new(),
                "input_too_large",
                format!(
                    "stdin line exceeds {}MB limit",
                    MAX_LINE_BYTES / (1024 * 1024)
                ),
            );
            if let Ok(serialized) = serde_json::to_string(&response) {
                shared_writer.write_line(&serialized);
            }
            continue;
        }
        let line = match std::str::from_utf8(&stdin_buf) {
            Ok(s) => s.to_string(),
            Err(e) => {
                log_agent_event(
                    "stdin_error",
                    &vaultpilot_lib::sanitize_error("invalid UTF-8 in request"),
                );
                let response = AgentResponse::error(
                    String::new(),
                    "invalid_encoding",
                    vaultpilot_lib::sanitize_error(&format!("invalid UTF-8 in request: {e}")),
                );
                if let Ok(serialized) = serde_json::to_string(&response) {
                    shared_writer.write_line(&serialized);
                }
                continue;
            }
        };
        let line = line.trim_end_matches('\n').trim_end_matches('\r');
        let line = line.to_string();
        let response = match runtime.block_on(async {
            tokio::time::timeout(
                REQUEST_TIMEOUT,
                handle_line(&context, &line, &shared_writer),
            )
            .await
        }) {
            Ok(response) => response,
            Err(_elapsed) => {
                log_agent_event(
                    "request_timeout",
                    &format!("request timed out after {}s", REQUEST_TIMEOUT.as_secs()),
                );
                // We don't have the request id here, so use an empty string.
                // The client matches on the sequential id from stdin, so this
                // is acceptable — the client knows which request it sent last.
                AgentResponse::error(
                    String::new(),
                    "timeout",
                    format!("request timed out after {}s", REQUEST_TIMEOUT.as_secs()),
                )
            }
        };

        if let Ok(serialized) = serde_json::to_string(&response) {
            shared_writer.write_line(&serialized);
        }
    }

    // Join the active agent thread on exit so it completes gracefully
    // instead of being abruptly killed when the process terminates.
    // First signal shutdown so a pending write-approval wait (which can no
    // longer be answered — the client is gone after stdin EOF) aborts and
    // the join below cannot deadlock (#3788).
    AGENT_SHUTDOWN.store(true, Ordering::SeqCst);
    if let Some(handle) = ACTIVE_AGENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        tracing::info!("vaultpilot-agent waiting for active agent thread to finish...");
        if let Err(e) = handle.join() {
            tracing::error!(
                "Agent thread panicked: {:?}",
                e.downcast_ref::<&str>().unwrap_or(&"<unknown>")
            );
        }
    }
}

fn install_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");

        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        };

        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let sanitized_payload = vaultpilot_lib::sanitize_error(&payload);
        let message = format!("panic on thread '{thread_name}': {sanitized_payload} at {location}");
        log_agent_event("panic", &message);

        // Write sanitized output to stderr instead of calling the default hook,
        // which would print the raw (unsanitized) payload and leak secrets. (#924)
        eprintln!("{message}");
    }));
}

fn log_agent_event(event: &str, detail: &str) {
    const MAX_LOG_SIZE: u64 = 512 * 1024; // 512 KB

    let log_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.local.vaultpilot");
    if fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let log_path = log_dir.join("agent-crash.log");

    // Rotate log if it exceeds the size limit — keep the last 256 KB
    if let Ok(metadata) = fs::metadata(&log_path) {
        if metadata.len() > MAX_LOG_SIZE {
            let keep = 256 * 1024usize;
            if let Ok(data) = fs::read(&log_path) {
                let start = data.len().saturating_sub(keep);
                // Find the next newline so we start at a complete line boundary
                let start = data[start..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|p| start + p + 1)
                    .unwrap_or(start);
                let _ = fs::write(&log_path, &data[start..]);
            }
        }
    }

    let timestamp = Utc::now().to_rfc3339();
    let entry = format!("[{timestamp}] {event}: {detail}\n");
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| f.write_all(entry.as_bytes()));
}

async fn handle_line(
    context: &StorageContext,
    line: &str,
    writer: &Arc<SharedWriter>,
) -> AgentResponse {
    let request = match serde_json::from_str::<AgentRequest>(line) {
        Ok(request) => request,
        Err(error) => {
            return AgentResponse::error(
                String::new(),
                "invalid_request",
                format!(
                    "failed to parse request JSON: {}",
                    vaultpilot_lib::sanitize_error(&error.to_string())
                ),
            );
        }
    };

    match handle_request(context, &request, writer).await {
        Ok(result) => AgentResponse::ok(request.id, result),
        Err(error) => AgentResponse::error(request.id, "request_failed", error),
    }
}

/// JSON-RPC result for the `startupStats` method (#3910): the recorded
/// startup phases, or an empty shape if the stats were never published
/// (shouldn't happen in practice — main() sets them before the loop).
fn startup_stats_response() -> Value {
    match STARTUP_STATS.get() {
        Some(stats) => startup_stats_to_json(stats),
        None => serde_json::json!({ "phases": [], "total_ms": 0 }),
    }
}

/// Serialize startup stats into the shape consumed by the WinUI
/// startup-stats window (#3910):
/// `{ "phases": [{ "name": "...", "elapsed_ms": 12.34 }, ...], "total_ms": 567.89 }`
/// where `elapsed_ms` is the per-phase **own** duration (the increment over
/// the previous phase, matching `StartupStats::report()`), not the
/// cumulative elapsed time, and `total_ms` is the overall startup time
/// (`StartupStats::total()`), which equals the sum of the phase increments.
fn startup_stats_to_json(stats: &StartupStats) -> Value {
    let mut previous = Duration::ZERO;
    let phases: Vec<Value> = stats
        .phases
        .iter()
        .map(|phase| {
            let own = phase.elapsed.saturating_sub(previous);
            previous = phase.elapsed;
            serde_json::json!({
                "name": phase.name,
                "elapsed_ms": own.as_secs_f64() * 1000.0,
            })
        })
        .collect();
    serde_json::json!({
        "phases": phases,
        "total_ms": stats.total().as_secs_f64() * 1000.0,
    })
}

async fn handle_request(
    context: &StorageContext,
    request: &AgentRequest,
    writer: &Arc<SharedWriter>,
) -> Result<Value, String> {
    match request.method.as_str() {
        "ping" => Ok(serde_json::json!({ "ok": true })),
        "startupStats" => Ok(startup_stats_response()),
        "getSettings" => {
            let mut settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            // Set provider to the active one so legacy consumers get the right config.
            settings.provider = settings.effective_provider().clone();
            settings.provider = settings.provider.masked();
            for p in &mut settings.providers {
                *p = p.masked();
            }
            serde_json::to_value(&settings)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "saveSettings" => {
            let params: SaveSettingsParams = parse_params(&request.params)?;
            let mut result = save_settings_async(context, params.settings)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            result.provider = result.provider.masked();
            for p in &mut result.providers {
                *p = p.masked();
            }
            serde_json::to_value(&result)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "loadChatState" => {
            let _guard = context.chat_state_lock.lock().await;
            serialize_result(load_chat_state_async(context).await)
        }
        "saveChatState" => {
            let _guard = context.chat_state_lock.lock().await;
            let params: SaveChatStateParams = parse_params(&request.params)?;
            serialize_result(save_chat_state_async(context, &params.state).await)
        }
        "listNotes" => serialize_result(list_notes_async(context).await),
        "loadNote" => {
            let params: IdParams = parse_params(&request.params)?;
            serialize_result(load_note_async(context, &params.id).await)
        }
        "saveNote" => {
            let params: SaveNoteParams = parse_params(&request.params)?;
            serialize_result(save_note_async(context, params.note).await)
        }
        "deleteNote" => {
            let params: IdParams = parse_params(&request.params)?;
            // Resolve cleanup mode from the persisted setting (#3732).
            let cleanup = load_settings_async(context)
                .await
                .unwrap_or_default()
                .attachment_cleanup_on_note_delete
                .resolve_delete_attachments();
            serialize_result(delete_note_async(context, &params.id, cleanup).await)
        }
        "findRelatedNotes" => {
            let params: IdWithLimitParams = parse_params(&request.params)?;
            serialize_result(
                find_related_notes_async(context, &params.id, params.limit.unwrap_or(5)).await,
            )
        }
        "importMarkdown" => {
            let params: ImportMarkdownParams = parse_params(&request.params)?;
            serialize_result(import_markdown_async(context, &params.paths).await)
        }
        "rebuildIndex" => serialize_result(rebuild_index_async(context).await),
        "listSnapshots" => {
            let params: ListSnapshotsParams = parse_params(&request.params)?;
            serialize_result(list_snapshots_for_note_async(context, &params.note_id).await)
        }
        "getSnapshot" => {
            let params: GetSnapshotParams = parse_params(&request.params)?;
            serialize_result(get_snapshot_async(context, &params.snapshot_id).await)
        }
        "restoreSnapshot" => {
            let params: RestoreSnapshotParams = parse_params(&request.params)?;
            serialize_result(
                restore_snapshot_async(context, &params.note_id, &params.snapshot_id).await,
            )
        }
        "diffSnapshot" => {
            let params: DiffSnapshotParams = parse_params(&request.params)?;
            // Load the snapshot to get its body, and load the current note to compare.
            let snapshot = get_snapshot_async(context, &params.snapshot_id)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?
                .ok_or_else(|| "snapshot not found".to_string())?;
            let current_note = load_note_async(context, &params.note_id)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let diff = compute_diff(&snapshot.body, &current_note.body, 3);
            serde_json::to_value(&diff).map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "readImagePreview" => {
            let params: PathParams = parse_params(&request.params)?;
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let vault_root = Path::new(&settings.vault_dir);
            let confined = normalize_tool_path(&params.path, vault_root)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            read_image_preview(&confined.to_string_lossy())
                .map(Value::String)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e))
        }
        // #3747: readFileAsDataUrl — reads any local image file (including
        // outside vault) and returns a base64 data-URI.  The vault path
        // containment check is intentionally skipped; this endpoint is
        // called by the trusted local WinUI Markdown image lightbox for
        // images referenced from notes.
        "readFileAsDataUrl" => {
            let params: PathParams = parse_params(&request.params)?;
            read_image_preview(&params.path)
                .map(Value::String)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e))
        }
        "openVaultDirectory" => {
            let params: PathParams = parse_params(&request.params)?;
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let vault_root = Path::new(&settings.vault_dir);
            let confined = normalize_tool_path(&params.path, vault_root)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            open_vault_directory(&confined.to_string_lossy())
                .map_err(|e| vaultpilot_lib::sanitize_error(&e))?;
            Ok(serde_json::json!({ "ok": true }))
        }
        "askWithAi" => {
            let params: AskWithAiParams = parse_params(&request.params)?;
            let result = ask_with_ai_with_context(
                context,
                params.question,
                params.history,
                params.image_paths,
                params.model_override,
                |stage, detail| {
                    writer.write_line(
                        &serde_json::to_string(&AgentEvent {
                            event: "agentStatus".to_string(),
                            payload: AgentStatusPayload {
                                stage: stage.to_string(),
                                detail,
                                timestamp: Utc::now().to_rfc3339(),
                            },
                        })
                        .unwrap_or_default(),
                    )
                },
            )
            .await;
            serialize_string_result(result)
        }
        "compressChatHistory" => {
            let params: CompressChatHistoryParams = parse_params(&request.params)?;
            let result = compress_chat_history_with_context(
                context,
                params.summary,
                params.history,
                |stage, detail| {
                    writer.write_line(
                        &serde_json::to_string(&AgentEvent {
                            event: "agentStatus".to_string(),
                            payload: AgentStatusPayload {
                                stage: stage.to_string(),
                                detail,
                                timestamp: Utc::now().to_rfc3339(),
                            },
                        })
                        .unwrap_or_default(),
                    )
                },
            )
            .await;
            serialize_string_result(result)
        }
        "runAgent" => {
            let params: RunAgentParams = parse_params(&request.params)?;

            // Reject concurrent agent runs — avoids the race where a second
            // runAgent overwrites AGENT_APPROVAL, leaving the first thread
            // blocked forever on rx.recv(). AGENT_RUNNING is acquired
            // atomically BEFORE spawning: a check-and-set on ACTIVE_AGENT
            // after spawn races with a fast-finishing task that clears it,
            // leaving a stale handle that permanently rejects later runs (#3788).
            if AGENT_RUNNING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                writer.write_line(
                    &serde_json::json!({
                        "event": "error",
                        "payload": {
                            "code": "AGENT_ALREADY_RUNNING",
                            "message": "Agent session is already in progress."
                        }
                    })
                    .to_string(),
                );
                return Ok(serde_json::json!({
                    "status": "rejected",
                    "reason": "agent already running"
                }));
            }

            let settings = match initialize_storage_async(context).await {
                Ok(settings) => settings,
                Err(e) => {
                    // No agent was spawned — release the run guard.
                    AGENT_RUNNING.store(false, Ordering::SeqCst);
                    return Err(vaultpilot_lib::sanitize_error(&e.to_string()));
                }
            };
            let ctx = context.clone();
            let writer: Arc<SharedWriter> = Arc::clone(writer);
            let prompt = params.prompt.clone();
            let images = params.images.clone();
            let history = params.history.clone();
            let max_steps = params.max_steps.unwrap_or(20);
            let auto_approve = params.auto_approve.unwrap_or(false);

            // Spawn agent in background so main loop can continue processing
            // respondToWriteApproval requests.
            let thread_writer = Arc::clone(&writer);
            let handle = std::thread::spawn(move || {
                // Guarantee the run guard and session state are cleared on
                // EVERY exit path (normal completion or panic). Without this,
                // a fast-finishing task would clear ACTIVE_AGENT before the
                // parent stores its handle, and a panicking task would leave
                // AGENT_RUNNING=true — both permanently rejecting later
                // runAgent calls until process restart (#3788).
                struct ClearSessionState;
                impl Drop for ClearSessionState {
                    fn drop(&mut self) {
                        AGENT_RUNNING.store(false, Ordering::SeqCst);
                        *AGENT_APPROVAL.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        *PLAN_APPROVAL.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        *ACTIVE_AGENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    }
                }
                let _session_guard = ClearSessionState;

                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create agent runtime");
                rt.block_on(async move {
                    run_agent_task(
                        &settings,
                        &ctx,
                        &prompt,
                        &images,
                        &history,
                        max_steps,
                        auto_approve,
                        thread_writer,
                    )
                    .await;
                });
            });

            // Store handle so main loop can detect concurrent runs and join on exit.
            *ACTIVE_AGENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);

            Ok(serde_json::json!({ "status": "started" }))
        }
        "respondToWriteApproval" => {
            let params: RespondToWriteApprovalParams = parse_params(&request.params)?;
            let tx = AGENT_APPROVAL
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            match tx {
                Some(tx) => {
                    let _ = tx.send(params.approved);
                    Ok(serde_json::json!({ "ok": true }))
                }
                None => Err("no active agent session waiting for approval".to_string()),
            }
        }
        "respondToPlanApproval" => {
            let params: RespondToPlanApprovalParams = parse_params(&request.params)?;
            let tx = PLAN_APPROVAL
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            match tx {
                Some(tx) => {
                    let decision = match params.action.as_str() {
                        "approve" => PlanDecision::Approve,
                        "reject" => PlanDecision::Reject,
                        "edit" => PlanDecision::Edit {
                            steps: params
                                .modified_steps
                                .unwrap_or_default()
                                .into_iter()
                                .map(|s| {
                                    vaultpilot_lib::agent::PlanStep::new(
                                        vaultpilot_lib::agent::PlanStepKind::Write,
                                        s,
                                        None,
                                    )
                                })
                                .collect(),
                        },
                        _ => PlanDecision::Reject,
                    };
                    let _ = tx.send(decision);
                    Ok(serde_json::json!({ "ok": true }))
                }
                None => Err("no active plan waiting for approval".to_string()),
            }
        }
        "executeAiAction" => {
            let params: ExecuteAiActionParams = parse_params(&request.params)?;
            let request = AiActionRequest {
                action: params.action,
                text: params.text.unwrap_or_default(),
                target_language: params.target_language,
                tone: params.tone,
                note_id: params.note_id,
                instruction: params.instruction,
                model: params.model_override,
                export_format: None,
            };
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let result = execute_ai_action(&settings, &request).await;
            serde_json::to_value(&result)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "listAiActions" => {
            let actions = list_ai_actions();
            serde_json::to_value(&actions)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        // ── Meeting audio transcription (#3629) ─────────────────
        "transcribeMeeting" => {
            let params: TranscribeMeetingParams = parse_params(&request.params)?;
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let provider = settings.effective_provider();

            // 1. Transcribe the audio file
            let transcript =
                transcribe_audio(&params.audio_path, provider, params.language.as_deref())
                    .await
                    .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;

            // 2. Generate structured meeting summary
            let mut summary = generate_meeting_summary(&transcript, &settings)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;

            // Override title if provided
            if let Some(ref t) = params.title {
                if !t.trim().is_empty() {
                    summary.title = t.clone();
                }
            }

            // 3. Build the result
            let mut result = MeetingTranscriptionResult {
                transcript: transcript.clone(),
                summary: summary.clone(),
                usage: RequestUsage::default(),
                note_path: None,
            };

            // 4. Save as a vault note
            let saved = tokio::task::block_in_place(|| create_meeting_note(context, &result))
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;

            result.note_path = Some(saved.meta.path.clone());

            serde_json::to_value(serde_json::json!({
                "transcript": transcript,
                "summary": summary,
                "note": saved,
            }))
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        // ── Subscription management (#2167) ──────────────────────
        "listSubscriptions" => {
            let subs = list_subscriptions_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let count = subs.len();
            serde_json::to_value(serde_json::json!({
                "subscriptions": subs,
                "count": count
            }))
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "createSubscription" => {
            let params: CreateSubscriptionParams = parse_params(&request.params)?;
            let sub = create_subscription_async(
                context,
                params.name,
                params.schedule,
                params.prompt,
                params.tools,
                params.target_collection,
            )
            .await
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            serde_json::to_value(serde_json::json!({
                "created": true,
                "subscription": sub
            }))
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "getSubscription" => {
            let params: IdParams = parse_params(&request.params)?;
            let sub = get_subscription_async(context, params.id)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?
                .ok_or_else(|| "subscription not found".to_string())?;
            serde_json::to_value(serde_json::json!({
                "subscription": sub
            }))
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "deleteSubscription" => {
            let params: IdParams = parse_params(&request.params)?;
            let deleted = delete_subscription_async(context, params.id)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            serde_json::to_value(serde_json::json!({
                "deleted": deleted
            }))
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "updateSubscription" => {
            let params: UpdateSubscriptionAgentParams = parse_params(&request.params)?;
            // Load existing subscription for partial merge
            let existing = get_subscription_async(context, params.id.clone())
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?
                .ok_or_else(|| "subscription not found".to_string())?;
            let new_schedule = params.schedule.unwrap_or(existing.schedule);
            let new_prompt = params.prompt.unwrap_or(existing.prompt);
            let new_tools = params.tools.unwrap_or(existing.tools);
            let new_target = params
                .target_collection
                .unwrap_or(existing.target_collection);
            let updated = update_subscription_async(
                context,
                params.id.clone(),
                params.name,
                new_schedule,
                new_prompt,
                new_tools,
                new_target,
            )
            .await
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            serde_json::to_value(serde_json::json!({
                "updated": updated
            }))
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "setSubscriptionEnabled" => {
            let params: SetSubscriptionEnabledParams = parse_params(&request.params)?;
            let updated =
                set_subscription_enabled_with_context(context, &params.id, params.enabled)
                    .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            serde_json::to_value(serde_json::json!({
                "updated": updated,
                "id": params.id,
                "enabled": params.enabled
            }))
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "runDueSubscriptions" => {
            let results = run_all_due_subscriptions(context).await;
            let count = results.len();
            serde_json::to_value(serde_json::json!({
                "ran": true,
                "count": count,
                "results": results
            }))
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "runSingleSubscription" => {
            let params: IdParams = parse_params(&request.params)?;
            let sub = get_subscription_async(context, params.id)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?
                .ok_or_else(|| "subscription not found".to_string())?;
            let result = run_single_subscription(context, &sub).await;
            serde_json::to_value(serde_json::json!({
                "ran": true,
                "result": result
            }))
            .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        // #2969: Quick Capture — append a timestamped bullet to today's daily
        // note or the inbox. Exposed over JSON-RPC so the WinUI QuickCaptureOverlay
        // (and future mobile quick-capture surfaces) can call it without spawning
        // the CLI. Reuses the library implementation that the CLI command also
        // uses, so behaviour is identical end-to-end.
        "capture" => {
            let params: CaptureParams = parse_params(&request.params)?;
            // Apply the same defaults the CLI uses when the caller omits them
            // (the WinUI overlay sends empty strings for "section" and switches
            // "target" between "daily" / "inbox" — but empty "target" should
            // behave like the CLI's default, not error out).
            let target = if params.target.is_empty() {
                "daily"
            } else {
                params.target.as_str()
            };
            let section = if params.section.is_empty() {
                "Quick Capture"
            } else {
                params.section.as_str()
            };
            let result =
                vaultpilot_lib::capture::handle_capture(context, &params.text, target, section)
                    .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            serde_json::to_value(result).map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        // ── Provider connection test (#3480) ──────────────────────
        "checkProviderConnection" => {
            let params: CheckProviderConnectionParams = parse_params(&request.params)?;
            let result = check_provider_connection(&params).await;
            serde_json::to_value(&result)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        // ── Flashcard / Spaced Repetition (#3763) ─────────────────
        "createFlashcard" => {
            let params: CreateFlashcardParams = parse_params(&request.params)?;
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let card = create_flashcard(
                &settings,
                params.front,
                params.back,
                params.source_note_id,
                params.tags,
            )
            .map_err(|e| vaultpilot_lib::sanitize_error(&e))?;
            serde_json::to_value(&card).map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "listFlashcards" => {
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let cards: Vec<Flashcard> =
                list_flashcards(&settings).map_err(|e| vaultpilot_lib::sanitize_error(&e))?;
            serde_json::to_value(&cards).map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "listDueFlashcards" => {
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let cards: Vec<Flashcard> =
                list_due_flashcards(&settings).map_err(|e| vaultpilot_lib::sanitize_error(&e))?;
            serde_json::to_value(&cards).map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "reviewFlashcard" => {
            let params: ReviewFlashcardParams = parse_params(&request.params)?;
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let result: ReviewResult = review_flashcard(&settings, &params.id, params.rating);
            serde_json::to_value(&result)
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        "getFlashcardStats" => {
            let settings = initialize_storage_async(context)
                .await
                .map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))?;
            let stats: FlashcardStats =
                get_stats(&settings).map_err(|e| vaultpilot_lib::sanitize_error(&e))?;
            serde_json::to_value(&stats).map_err(|e| vaultpilot_lib::sanitize_error(&e.to_string()))
        }
        method => Err(format!("unknown method: {method}")),
    }
}

// ── Provider connection test (#3480) ────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckProviderConnectionParams {
    /// API base URL (e.g. https://api.openai.com/v1)
    api_base: String,
    /// API key (may be masked for round-trip; if so, return an error so the
    /// caller knows to use the real key)
    api_key: String,
    /// Provider type string: "openai", "anthropic", "ollama"
    #[serde(default)]
    provider_type: String,
    /// Optional model name (not strictly needed for /models probe, but kept
    /// for future use such as a targeted chat-completion ping).
    #[serde(default)]
    #[allow(dead_code)]
    model: Option<String>,
    /// Optional timeout in milliseconds (default 8000)
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderConnectionResult {
    /// True when the upstream returned HTTP 2xx for the probe request.
    ok: bool,
    /// HTTP status code if we got an HTTP response, else null.
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
    /// Human-readable error message when ok=false.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// The probe URL that was hit, for diagnostics.
    #[serde(skip_serializing_if = "Option::is_none")]
    probe_url: Option<String>,
    /// Available model names discovered during the probe (#3489).
    ///
    /// Populated from Ollama `/api/tags` (`.models[].name`) or
    /// OpenAI-compatible `/models` (`data[].id`). Empty when the probe
    /// failed or the response body could not be parsed. This powers the
    /// "自动模型检测" requirement: the UI can offer a model picker without
    /// the user typing the model tag by hand.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    models: Vec<String>,
}

/// Probes the configured provider by hitting its `/models` endpoint.
///
/// This mirrors the mobile client's `checkApi()` logic so WinUI gets the same
/// "test connection" UX described in #3480. OpenAI-compatible providers answer
/// `GET /v1/models` with Bearer auth; Anthropic answers `GET /v1/models` with
/// `x-api-key`. Ollama has no auth on `/api/tags`.
async fn check_provider_connection(
    params: &CheckProviderConnectionParams,
) -> ProviderConnectionResult {
    use std::time::Duration;

    const DEFAULT_TIMEOUT_MS: u64 = 8_000;
    const MASK_SENTINEL: &str = "********";

    // Masked key indicates the caller sent the stored (masked) settings rather
    // than the live dialog input — reject so the WinUI client knows to send the
    // freshly typed key.
    if params.api_key.is_empty() {
        return ProviderConnectionResult {
            ok: false,
            status: None,
            error: Some("API Key 未填写".to_string()),
            probe_url: None,
            models: vec![],
        };
    }
    if params.api_key == MASK_SENTINEL {
        return ProviderConnectionResult {
            ok: false,
            status: None,
            error: Some("API Key 已掩码，无法测试；请重新输入完整 Key".to_string()),
            probe_url: None,
            models: vec![],
        };
    }

    let timeout = Duration::from_millis(params.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(e) => {
            return ProviderConnectionResult {
                ok: false,
                status: None,
                error: Some(format!("构造 HTTP 客户端失败: {e}")),
                probe_url: None,
                models: vec![],
            }
        }
    };

    let ptype = params.provider_type.to_ascii_lowercase();
    let base = params.api_base.trim_end_matches('/').to_string();

    let result = match ptype.as_str() {
        "anthropic" => {
            let url = format!("{}/v1/models", base);
            let resp = client
                .get(&url)
                .header("x-api-key", &params.api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await;
            (url, resp)
        }
        "ollama" => {
            // Ollama exposes /api/tags without auth
            let url = format!("{}/api/tags", base);
            let resp = client.get(&url).send().await;
            (url, resp)
        }
        // Default: OpenAI-compatible
        _ => {
            let url = format!("{}/models", base);
            let resp = client
                .get(&url)
                .header("Authorization", format!("Bearer {}", params.api_key))
                .send()
                .await;
            (url, resp)
        }
    };

    let (probe_url, resp_result) = result;
    match resp_result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            // Read the body once and reuse it for both status reporting and
            // model-list extraction (#3489 auto-detection).
            let body = resp.text().await.unwrap_or_default();
            if (200..300).contains(&status) {
                let models = match ptype.as_str() {
                    "ollama" => parse_ollama_tags_response(&body),
                    "anthropic" => parse_openai_models_response(&body),
                    _ => parse_openai_models_response(&body),
                };
                ProviderConnectionResult {
                    ok: true,
                    status: Some(status),
                    error: None,
                    probe_url: Some(probe_url),
                    models,
                }
            } else {
                let snippet = body.chars().take(200).collect::<String>();
                ProviderConnectionResult {
                    ok: false,
                    status: Some(status),
                    error: Some(format!("HTTP {status}: {snippet}")),
                    probe_url: Some(probe_url),
                    models: vec![],
                }
            }
        }
        Err(e) => {
            let msg = if e.is_connect() {
                "无法连接到供应商（连接被拒绝/DNS 失败）".to_string()
            } else if e.is_timeout() {
                "连接超时（请检查网络或增大超时设置）".to_string()
            } else {
                format!("{e}")
            };
            ProviderConnectionResult {
                ok: false,
                status: None,
                error: Some(msg),
                probe_url: Some(probe_url),
                models: vec![],
            }
        }
    }
}

// ── Model auto-detection parsers (#3489) ───────────────────────

/// Parse an Ollama `/api/tags` response body and return the list of
/// installed model names.
///
/// Ollama's response shape:
/// ```json
/// { "models": [ { "name": "llama3.2:latest", ... }, { "name": "mistral:7b", ... } ] }
/// ```
///
/// Returns an empty `Vec` on any parse failure (never panics) so that a
/// malformed upstream body degrades gracefully — the connection probe still
/// reports `ok: true` with the HTTP status, just without a model list.
fn parse_ollama_tags_response(body: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct TagsResponse {
        #[serde(default)]
        models: Vec<TagsModel>,
    }
    #[derive(serde::Deserialize)]
    struct TagsModel {
        name: String,
    }

    serde_json::from_str::<TagsResponse>(body)
        .map(|r| {
            r.models
                .into_iter()
                .map(|m| m.name)
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Parse an OpenAI-compatible (and Anthropic) `/models` (or `/v1/models`)
/// response body and return the list of model ids.
///
/// OpenAI shape: `{ "data": [ { "id": "gpt-4o-mini", ... } ] }`
/// Anthropic shape: `{ "data": [ { "id": "claude-3-5-sonnet-20241022", ... } ] }`
///
/// Returns an empty `Vec` on any parse failure (never panics).
fn parse_openai_models_response(body: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        #[serde(default)]
        data: Vec<ModelsEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ModelsEntry {
        id: String,
    }

    serde_json::from_str::<ModelsResponse>(body)
        .map(|r| {
            r.data
                .into_iter()
                .map(|m| m.id)
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureParams {
    /// The text to capture.
    text: String,
    /// "daily" (today's daily note) or "inbox". Defaults to "daily".
    #[serde(default)]
    target: String,
    /// Section heading to append under. Empty string uses the CLI default
    /// ("Quick Capture").
    #[serde(default)]
    section: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteAiActionParams {
    action: AiActionType,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    tone: Option<String>,
    #[serde(default)]
    note_id: Option<String>,
    #[serde(default)]
    instruction: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
}

// ── Meeting audio transcription params (#3629) ─────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscribeMeetingParams {
    /// Path to the audio file (e.g. mp3, m4a, wav, ogg, flac).
    audio_path: String,
    /// Optional language hint (ISO 639-1, e.g. "zh", "en").
    #[serde(default)]
    language: Option<String>,
    /// Optional meeting title override.
    #[serde(default)]
    title: Option<String>,
}

// ── Subscription method params (#2167) ──────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSubscriptionParams {
    name: String,
    #[serde(default = "default_schedule")]
    schedule: String,
    prompt: String,
    #[serde(default = "default_tools")]
    tools: String,
    #[serde(default = "default_target_collection")]
    target_collection: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdParams {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdWithLimitParams {
    id: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveNoteParams {
    note: NoteDocument,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSubscriptionEnabledParams {
    id: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSubscriptionAgentParams {
    id: String,
    name: String,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    tools: Option<String>,
    #[serde(default)]
    target_collection: Option<String>,
}

fn default_schedule() -> String {
    "0 0 * * *".to_string()
}

fn default_tools() -> String {
    "web_search".to_string()
}

fn default_target_collection() -> String {
    "Scheduled Research".to_string()
}

fn parse_params<T>(params: &Value) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(params.clone())
        .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
}

fn serialize_result<T>(result: anyhow::Result<T>) -> Result<Value, String>
where
    T: Serialize,
{
    result
        .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
        .and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
        })
}

fn serialize_string_result<T>(result: Result<T, anyhow::Error>) -> Result<Value, String>
where
    T: Serialize,
{
    result
        .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
        .and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| vaultpilot_lib::sanitize_error(&error.to_string()))
        })
}

/// Waits for a write-approval decision from the UI. Returns `false`
/// (deny the write) if the approval sender disconnects OR if the agent
/// process is shutting down after stdin EOF. The shutdown check matters
/// because once stdin hits EOF no more `respondToWriteApproval` requests
/// can be read, so a naive `rx.recv()` would block forever and main()'s
/// join() on exit would deadlock (#3788).
fn await_write_approval(rx: std::sync::mpsc::Receiver<bool>) -> bool {
    loop {
        if AGENT_SHUTDOWN.load(Ordering::SeqCst) {
            return false;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(approved) => return approved,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// Run an agent session in the background. Emits events via `writer` and
/// handles write-approval through the global `AGENT_APPROVAL` channel.
#[allow(clippy::too_many_arguments)]
async fn run_agent_task(
    settings: &AppSettings,
    context: &StorageContext,
    prompt: &str,
    images: &[String],
    history: &[vaultpilot_lib::models::ConversationTurn],
    max_steps: usize,
    auto_approve: bool,
    writer: Arc<SharedWriter>,
) {
    let config = AgentConfig {
        name: "vaultpilot-agent-ui".into(),
        permission: AgentPermission::ReadWrite,
        limits: AgentResourceLimits {
            max_duration: Duration::from_secs(300),
            max_tool_calls: max_steps as u64,
            max_tokens: 0,
        },
        ..AgentConfig::default()
    };

    let result = vaultpilot_lib::agent::run_agent(
        settings,
        context,
        prompt,
        images,
        history,
        config,
        |event| {
            match event {
                LibAgentEvent::Thinking { step } => {
                    emit_event(
                        &writer,
                        "thinking",
                        &format!("Step {step}"),
                        Some(*step),
                        None,
                        None,
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::ToolCall { step, tool, args } => {
                    emit_event(
                        &writer,
                        "toolCall",
                        &format!("Calling {tool}"),
                        Some(*step),
                        Some(tool),
                        Some(args),
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::ToolResult {
                    step,
                    tool,
                    result_preview,
                    is_error,
                } => {
                    emit_event(
                        &writer,
                        "toolResult",
                        result_preview,
                        Some(*step),
                        Some(tool),
                        None,
                        Some(result_preview),
                        Some(*is_error),
                    );
                    true
                }
                LibAgentEvent::FinalAnswer { text } => {
                    emit_event(&writer, "finalAnswer", text, None, None, None, None, None);
                    true
                }
                LibAgentEvent::StepLimitReached { steps } => {
                    emit_event(
                        &writer,
                        "stepLimitReached",
                        &format!("{steps} steps reached"),
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::TokenBudgetExceeded {
                    tokens_used,
                    budget,
                } => {
                    emit_event(
                        &writer,
                        "tokenBudgetExceeded",
                        &format!("{tokens_used}/{budget}"),
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::Timeout => {
                    emit_event(
                        &writer,
                        "timeout",
                        "Agent timed out",
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::Error { message } => {
                    emit_event(&writer, "error", message, None, None, None, None, None);
                    true
                }
                LibAgentEvent::WriteApprovalNeeded { tool, args } => {
                    emit_event(
                        &writer,
                        "writeApprovalNeeded",
                        &format!("Write approval needed for {tool}"),
                        None,
                        Some(tool),
                        Some(args),
                        None,
                        None,
                    );

                    if auto_approve {
                        return true;
                    }

                    // Wait for approval from the UI via respondToWriteApproval
                    let (tx, rx) = std::sync::mpsc::channel();
                    *AGENT_APPROVAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
                    // Block until approval received (this runs in a background thread,
                    // not on the main stdin loop, so blocking is fine). The wait also
                    // aborts (denying the write) once the process is shutting down
                    // after stdin EOF, preventing a join() deadlock on exit (#3788).
                    await_write_approval(rx)
                }
                LibAgentEvent::PlanProposed { plan } => {
                    emit_event(
                        &writer,
                        "planProposed",
                        &plan.render_markdown(),
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    true
                }
                LibAgentEvent::UnhealthyDetected {
                    reason,
                    suggestion,
                } => {
                    let payload = serde_json::json!({
                        "stage": "unhealthyDetected",
                        "detail": reason,
                        "suggestion": suggestion,
                        "timestamp": Utc::now().to_rfc3339()
                    });
                    writer.write_line(&serde_json::json!({
                        "event": "agentStatus",
                        "payload": payload
                    })
                    .to_string());
                    true
                }
            }
        },
        |_plan| {
            // Plan Mode approval through the UI sidecar (#3791).
            // Mirror write-approval: fire a channel to PLAN_APPROVAL,
            // wait for the client to call respondToPlanApproval.
            // Also aborts on shutdown to prevent join() deadlock (#3788).
            let (tx, rx) = std::sync::mpsc::channel();
            *PLAN_APPROVAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
            loop {
                if AGENT_SHUTDOWN.load(Ordering::SeqCst) {
                    eprintln!("[vaultpilot-agent] Plan Mode: shuting down — auto-rejecting");
                    return PlanDecision::Reject;
                }
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(decision) => return decision,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        eprintln!("[vaultpilot-agent] Plan Mode: approval channel disconnected — auto-rejecting");
                        return PlanDecision::Reject;
                    }
                }
            }
        },
    )
    .await;

    match result {
        Ok(result) => {
            let final_event = serde_json::json!({
                "event": "agentStatus",
                "payload": {
                    "stage": "agentCompleted",
                    "detail": result.answer,
                    "stepsUsed": result.steps_used,
                    "tokensUsed": result.tokens_used,
                    "timestamp": Utc::now().to_rfc3339()
                }
            });
            writer.write_line(&final_event.to_string());
        }
        Err(e) => {
            let err_event = serde_json::json!({
                "event": "agentStatus",
                "payload": {
                    "stage": "error",
                    "detail": vaultpilot_lib::sanitize_error(&e.to_string()),
                    "timestamp": Utc::now().to_rfc3339()
                }
            });
            writer.write_line(&err_event.to_string());
        }
    }

    // Clear any pending approval channel and active agent handle.
    *AGENT_APPROVAL.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *ACTIVE_AGENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[allow(clippy::too_many_arguments)]
fn emit_event(
    writer: &SharedWriter,
    stage: &str,
    detail: &str,
    step: Option<usize>,
    tool: Option<&str>,
    args: Option<&str>,
    result_preview: Option<&str>,
    is_error: Option<bool>,
) {
    let mut payload = serde_json::json!({
        "stage": stage,
        "detail": detail,
        "timestamp": Utc::now().to_rfc3339()
    });
    if let Some(s) = step {
        payload["step"] = serde_json::json!(s);
    }
    if let Some(t) = tool {
        payload["tool"] = serde_json::json!(t);
    }
    if let Some(a) = args {
        payload["args"] = serde_json::json!(a);
    }
    if let Some(r) = result_preview {
        payload["resultPreview"] = serde_json::json!(r);
    }
    if let Some(e) = is_error {
        payload["isError"] = serde_json::json!(e);
    }

    let event = serde_json::json!({ "event": "agentStatus", "payload": payload });
    writer.write_line(&event.to_string());
}

fn read_image_preview(path: &str) -> Result<String, String> {
    const MAX_IMAGE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_IMAGE_SIZE {
        return Err(format!(
            "image too large ({} bytes, limit is {} bytes): {}",
            metadata.len(),
            MAX_IMAGE_SIZE,
            path
        ));
    }

    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let media_type = match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => return Err("unsupported image format".to_string()),
    };

    Ok(format!(
        "data:{};base64,{}",
        media_type,
        STANDARD.encode(bytes)
    ))
}

fn open_vault_directory(path: &str) -> Result<(), String> {
    let target = Path::new(path);
    if !target.exists() {
        return Err("vault directory does not exist".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("opening directories is not supported on this platform".to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSettingsParams {
    settings: AppSettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveChatStateParams {
    state: ChatState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportMarkdownParams {
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathParams {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AskWithAiParams {
    question: String,
    #[serde(default)]
    history: Option<Vec<ConversationTurn>>,
    #[serde(default)]
    image_paths: Option<Vec<String>>,
    #[serde(default)]
    model_override: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompressChatHistoryParams {
    summary: Option<ConversationSummary>,
    history: Vec<ConversationTurn>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunAgentParams {
    prompt: String,
    #[serde(default)]
    max_steps: Option<usize>,
    #[serde(default)]
    auto_approve: Option<bool>,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    history: Vec<vaultpilot_lib::models::ConversationTurn>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RespondToWriteApprovalParams {
    approved: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RespondToPlanApprovalParams {
    action: String,
    #[serde(default)]
    #[allow(dead_code)]
    reason: Option<String>,
    #[serde(default)]
    modified_steps: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListSnapshotsParams {
    note_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetSnapshotParams {
    snapshot_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreSnapshotParams {
    note_id: String,
    snapshot_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiffSnapshotParams {
    note_id: String,
    snapshot_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateFlashcardParams {
    front: String,
    back: String,
    #[serde(default)]
    source_note_id: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewFlashcardParams {
    id: String,
    rating: ReviewRating,
}

impl AgentResponse {
    fn ok(id: String, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: String, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(AgentError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_deserializes_from_json() {
        let json = json!({
            "id": "req-001",
            "method": "ping",
            "params": {}
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(request.id, "req-001");
        assert_eq!(request.method, "ping");
    }

    #[test]
    fn request_deserializes_with_missing_params() {
        let json = json!({
            "id": "req-002",
            "method": "listNotes"
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(request.params, Value::Null);
    }

    #[test]
    fn response_ok_serializes_without_error() {
        let response = AgentResponse::ok("req-001".into(), json!({"ok": true}));
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("\"id\":\"req-001\""));
        assert!(serialized.contains("\"result\""));
        assert!(!serialized.contains("\"error\""));
    }

    #[test]
    fn response_error_serializes_without_result() {
        let response = AgentResponse::error("req-002".into(), "invalid_request", "bad input");
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("\"id\":\"req-002\""));
        assert!(serialized.contains("\"error\""));
        assert!(!serialized.contains("\"result\""));
    }

    // ── Regression: #3910 startupStats method ───────────────────────────

    #[test]
    fn startup_stats_to_json_serializes_phases_in_order() {
        use vaultpilot_lib::startup_stats::StartupPhase;

        let stats = StartupStats {
            phases: vec![
                StartupPhase {
                    name: "search_rules_load".into(),
                    elapsed: Duration::from_millis(5),
                },
                StartupPhase {
                    name: "runtime_build".into(),
                    elapsed: Duration::from_millis(12),
                },
                StartupPhase {
                    name: "storage_open".into(),
                    elapsed: Duration::from_millis(40),
                },
            ],
        };
        let json = startup_stats_to_json(&stats);
        let phases = json["phases"].as_array().expect("phases must be an array");
        assert_eq!(phases.len(), 3, "all phases must be serialized");
        let names: Vec<&str> = phases
            .iter()
            .map(|p| p["name"].as_str().expect("name must be a string"))
            .collect();
        assert_eq!(
            names,
            ["search_rules_load", "runtime_build", "storage_open"],
            "phases must be serialized in recording order"
        );
        assert_eq!(phases[0]["elapsed_ms"], 5.0);
        assert_eq!(phases[1]["elapsed_ms"], 7.0, "own duration = 12 - 5");
        assert_eq!(phases[2]["elapsed_ms"], 28.0, "own duration = 40 - 12");
        assert_eq!(
            json["total_ms"], 40.0,
            "total_ms must equal the last phase's cumulative elapsed (sum of increments)"
        );
    }

    #[test]
    fn startup_stats_to_json_empty_stats() {
        let stats = StartupStats::default();
        let json = startup_stats_to_json(&stats);
        assert_eq!(
            json["phases"]
                .as_array()
                .expect("phases must be an array")
                .len(),
            0,
            "empty stats must yield an empty phases array"
        );
        assert_eq!(json["total_ms"], 0.0);
    }

    #[test]
    fn startup_stats_response_returns_recorded_stats_when_set() {
        // STARTUP_STATS is a process-wide OnceLock; tests run without main(),
        // so it is guaranteed unset before this test sets it.
        use vaultpilot_lib::startup_stats::StartupPhase;

        let stats = StartupStats {
            phases: vec![StartupPhase {
                name: "ipc_ready".into(),
                elapsed: Duration::from_millis(7),
            }],
        };
        assert!(
            STARTUP_STATS.set(stats).is_ok(),
            "STARTUP_STATS must be unset before this test"
        );
        let json = startup_stats_response();
        assert_eq!(json["phases"][0]["name"], "ipc_ready");
        assert_eq!(json["phases"][0]["elapsed_ms"], 7.0);
        assert_eq!(json["total_ms"], 7.0);
    }

    #[test]
    fn startup_stats_method_is_dispatched_in_handle_request() {
        // Pin the match arm so a rename/removal of the "startupStats"
        // dispatch fails this regression test.
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bin/vaultpilot-agent.rs"
        ));
        assert!(
            source.contains("\"startupStats\" =>"),
            "handle_request must dispatch the startupStats method"
        );
        assert!(
            source.contains("startup_stats_response"),
            "startupStats arm must call startup_stats_response"
        );
    }

    #[test]
    fn response_error_contains_code_and_message() {
        let response = AgentResponse::error("x".into(), "timeout", "timed out after 120s");
        let value: Value = serde_json::to_value(&response).unwrap();
        let error = &value["error"];
        assert_eq!(error["code"], "timeout");
        assert!(error["message"].as_str().unwrap().contains("120s"));
    }

    #[test]
    fn agent_event_serializes_with_camel_case() {
        let event = AgentEvent {
            event: "agentStatus".to_string(),
            payload: AgentStatusPayload {
                stage: "analyzing".to_string(),
                detail: "Analyzing request".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
            },
        };
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(serialized.contains("\"agentStatus\""));
        assert!(serialized.contains("\"stage\""));
        assert!(serialized.contains("\"analyzing\""));
    }

    #[test]
    fn request_with_complex_params() {
        let json = json!({
            "id": "req-003",
            "method": "askWithAi",
            "params": {
                "question": "What is Rust?",
                "history": [{"role": "user", "content": "hello"}],
                "imagePaths": ["/tmp/img.png"]
            }
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(request.method, "askWithAi");
        assert!(request.params.is_object());
        assert_eq!(request.params["question"], "What is Rust?");
    }

    #[test]
    fn request_roundtrip_json() {
        let original = json!({
            "id": "test-123",
            "method": "saveChatState",
            "params": {"state": {"sessions": []}}
        });
        let request: AgentRequest = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(request.id, "test-123");
        assert_eq!(request.method, "saveChatState");
        assert!(request.params.is_object());
    }

    #[test]
    fn invalid_json_returns_parse_error() {
        let result = serde_json::from_str::<AgentRequest>("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn empty_method_name_parses() {
        let json = json!({ "id": "req-004", "method": "" });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert!(request.method.is_empty());
    }

    #[test]
    fn non_utf8_bytes_rejected_by_from_utf8() {
        // Simulates the exact validation used in the agent stdin loop.
        // Invalid continuation byte 0xFF is never valid in UTF-8.
        let invalid_bytes: Vec<u8> = vec![0xFF, 0xFE, 0x80, 0x00];
        let result = std::str::from_utf8(&invalid_bytes);
        assert!(result.is_err(), "expected invalid UTF-8 to be rejected");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("invalid"),
            "error message should mention invalid UTF-8: {err_msg}"
        );
    }

    #[test]
    fn sanitize_error_applied_to_utf8_message() {
        // Ensure the error message goes through sanitize_error as required.
        let raw = "invalid UTF-8 in request: invalid utf-8 sequence of 1 bytes from index 0";
        let sanitized = vaultpilot_lib::sanitize_error(raw);
        assert!(
            sanitized.contains("invalid UTF-8 in request"),
            "sanitized message should retain the key information: {sanitized}"
        );
    }

    #[test]
    fn load_note_params_deserializes() {
        let json =
            json!({ "id": "req-load", "method": "loadNote", "params": { "id": "note-123" } });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        let params: IdParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(params.id, "note-123");
    }

    #[test]
    fn delete_note_params_deserializes() {
        let json =
            json!({ "id": "req-del", "method": "deleteNote", "params": { "id": "note-456" } });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        let params: IdParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(params.id, "note-456");
    }

    #[test]
    fn save_note_params_deserializes() {
        let json = json!({
            "id": "req-save",
            "method": "saveNote",
            "params": {
                "note": {
                    "meta": {
                        "id": "note-789",
                        "title": "Test Note"
                    },
                    "body": "Hello world"
                }
            }
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        let params: SaveNoteParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(params.note.meta.id, "note-789");
        assert_eq!(params.note.meta.title, "Test Note");
        assert_eq!(params.note.body, "Hello world");
    }

    // ─── #2969: Quick Capture JSON-RPC method ──────────────────────────

    #[test]
    fn capture_params_deserializes_with_all_fields() {
        // WinUI QuickCaptureOverlay sends { text, target, section }.
        let json = json!({
            "id": "req-cap-1",
            "method": "capture",
            "params": {
                "text": "buy milk",
                "target": "inbox",
                "section": "Errands"
            }
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(request.method, "capture");
        let params: CaptureParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(params.text, "buy milk");
        assert_eq!(params.target, "inbox");
        assert_eq!(params.section, "Errands");
    }

    #[test]
    fn capture_params_deserializes_with_defaults() {
        // The WinUI overlay sends an empty string for "section" when the user
        // hasn't picked one — the request must still deserialize, and the
        // handler applies the CLI-equivalent default ("Quick Capture").
        let json = json!({
            "id": "req-cap-2",
            "method": "capture",
            "params": {
                "text": "quick idea",
                "target": "daily",
                "section": ""
            }
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        let params: CaptureParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(params.text, "quick idea");
        assert_eq!(params.target, "daily");
        assert_eq!(params.section, "");
    }

    #[test]
    fn capture_params_uses_camel_case_for_text_field() {
        // The struct uses #[serde(rename_all = "camelCase")] but "text" has no
        // case to rename — this test pins the wire format so future renames
        // don't silently break the WinUI overlay.
        let json = json!({
            "id": "req-cap-3",
            "method": "capture",
            "params": {
                "text": "hello",
                "target": "daily"
            }
        });
        let request: AgentRequest = serde_json::from_value(json).unwrap();
        let params: CaptureParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(params.text, "hello");
        // section defaults to empty string via #[serde(default)].
        assert_eq!(params.section, "");
    }

    #[test]
    fn capture_request_roundtrip_through_agent_request() {
        // End-to-end wire-format check: the request the WinUI overlay sends
        // must deserialize into AgentRequest + CaptureParams without loss.
        let original = json!({
            "id": "req-cap-rt",
            "method": "capture",
            "params": {
                "text": "meeting at 3pm",
                "target": "daily",
                "section": "Today"
            }
        });
        let request: AgentRequest = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(request.id, "req-cap-rt");
        assert_eq!(request.method, "capture");

        let params: CaptureParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(params.text, "meeting at 3pm");
        assert_eq!(params.target, "daily");
        assert_eq!(params.section, "Today");
    }

    #[test]
    fn get_snapshot_params_deserializes_snapshot_id() {
        // Regression for #3410: getSnapshot must accept { snapshotId } not { noteId }.
        // A client sending { "snapshotId": "abc-123" } should deserialize
        // into GetSnapshotParams without error.
        let json = json!({ "snapshotId": "abc-123" });
        let params: GetSnapshotParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.snapshot_id, "abc-123");
    }

    #[test]
    fn get_snapshot_params_rejects_note_id_only() {
        // Regression for #3410: the old buggy handler used ListSnapshotsParams
        // (noteId), so a correct getSnapshot call with snapshotId would fail.
        // Verify that sending { "noteId": "..." } (the old wrong format) is
        // now rejected by GetSnapshotParams — it requires snapshotId.
        let json = json!({ "noteId": "some-note-uuid" });
        let result: Result<GetSnapshotParams, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "GetSnapshotParams should NOT accept noteId — it requires snapshotId"
        );
    }

    #[test]
    fn get_snapshot_request_through_agent_request() {
        // End-to-end wire-format check for #3410: a getSnapshot request with
        // { snapshotId } must deserialize through AgentRequest → GetSnapshotParams.
        let wire = json!({
            "id": "req-getsnap",
            "method": "getSnapshot",
            "params": {
                "snapshotId": "snap-uuid-001"
            }
        });
        let request: AgentRequest = serde_json::from_value(wire).unwrap();
        assert_eq!(request.method, "getSnapshot");
        let params: GetSnapshotParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(params.snapshot_id, "snap-uuid-001");
    }

    // ── #3480: Provider connection test ───────────────────────────

    #[test]
    fn check_provider_connection_params_camel_case_roundtrip() {
        let wire = json!({
            "apiBase": "https://api.openai.com/v1",
            "apiKey": "sk-test-123",
            "providerType": "openai",
            "model": "gpt-4o-mini",
            "timeoutMs": 5000
        });
        let params: CheckProviderConnectionParams = serde_json::from_value(wire).unwrap();
        assert_eq!(params.api_base, "https://api.openai.com/v1");
        assert_eq!(params.api_key, "sk-test-123");
        assert_eq!(params.provider_type, "openai");
        assert_eq!(params.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(params.timeout_ms, Some(5000));
    }

    #[test]
    fn check_provider_connection_params_accepts_minimal_payload() {
        // Only apiBase + apiKey are required; the rest default.
        let wire = json!({
            "apiBase": "https://opencode.ai/zen/v1",
            "apiKey": "sk-min"
        });
        let params: CheckProviderConnectionParams = serde_json::from_value(wire).unwrap();
        assert_eq!(params.provider_type, ""); // serde default
        assert_eq!(params.model, None);
        assert_eq!(params.timeout_ms, None);
    }

    #[test]
    fn check_provider_connection_rejects_empty_api_key() {
        let params = CheckProviderConnectionParams {
            api_base: "https://api.openai.com/v1".into(),
            api_key: "".into(),
            provider_type: "openai".into(),
            model: None,
            timeout_ms: Some(1000),
        };
        // We can't easily await in #[test] without a runtime; exercise the
        // early-return path via a tokio runtime block_on.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(check_provider_connection(&params));
        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("API Key"));
    }

    #[test]
    fn check_provider_connection_rejects_masked_api_key() {
        let params = CheckProviderConnectionParams {
            api_base: "https://api.openai.com/v1".into(),
            api_key: "********".into(),
            provider_type: "openai".into(),
            model: None,
            timeout_ms: Some(1000),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(check_provider_connection(&params));
        assert!(!result.ok);
        assert!(
            result.error.unwrap_or_default().contains("掩码"),
            "masked key should yield a masked-error message"
        );
    }

    #[test]
    fn provider_connection_result_serializes_camel_case() {
        let r = ProviderConnectionResult {
            ok: true,
            status: Some(200),
            error: None,
            probe_url: Some("https://api.openai.com/v1/models".into()),
            models: vec![],
        };
        let v: Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["status"], 200);
        assert_eq!(v["probeUrl"], "https://api.openai.com/v1/models");
        // error should be skipped because it's None
        assert!(v.get("error").is_none() || v["error"].is_null());
        // empty models should be skipped (skip_serializing_if = "Vec::is_empty")
        assert!(v.get("models").is_none() || v["models"].is_null());
    }

    // ── #3489: Ollama / OpenAI model auto-detection ──────────────

    #[test]
    fn parse_ollama_tags_response_extracts_model_names() {
        let body = r#"{
            "models": [
                { "name": "llama3.2:latest", "size": 2000000000 },
                { "name": "mistral:7b", "size": 4100000000 },
                { "name": "qwen2.5:14b", "size": 9000000000 }
            ]
        }"#;
        let names = parse_ollama_tags_response(body);
        assert_eq!(names, vec!["llama3.2:latest", "mistral:7b", "qwen2.5:14b"]);
    }

    #[test]
    fn parse_ollama_tags_response_empty_models() {
        let body = r#"{ "models": [] }"#;
        assert!(parse_ollama_tags_response(body).is_empty());
    }

    #[test]
    fn parse_ollama_tags_response_missing_models_key() {
        // A server that returns an unexpected shape should not panic — we
        // degrade to an empty list so the connection probe still reports ok.
        let body = r#"{ "error": "something else" }"#;
        assert!(parse_ollama_tags_response(body).is_empty());
    }

    #[test]
    fn parse_ollama_tags_response_malformed_json() {
        assert!(parse_ollama_tags_response("not json at all").is_empty());
        assert!(parse_ollama_tags_response("").is_empty());
    }

    #[test]
    fn parse_ollama_tags_response_skips_empty_names() {
        let body = r#"{ "models": [ { "name": "" }, { "name": "real:latest" } ] }"#;
        assert_eq!(parse_ollama_tags_response(body), vec!["real:latest"]);
    }

    #[test]
    fn parse_openai_models_response_extracts_ids() {
        let body = r#"{
            "data": [
                { "id": "gpt-4o-mini", "object": "model" },
                { "id": "gpt-4o", "object": "model" }
            ]
        }"#;
        let ids = parse_openai_models_response(body);
        assert_eq!(ids, vec!["gpt-4o-mini", "gpt-4o"]);
    }

    #[test]
    fn parse_openai_models_response_anthropic_shape() {
        // Anthropic /v1/models uses the same { data: [ { id } ] } shape.
        let body = r#"{
            "data": [
                { "id": "claude-3-5-sonnet-20241022", "type": "model" },
                { "id": "claude-3-5-haiku-20241022", "type": "model" }
            ]
        }"#;
        let ids = parse_openai_models_response(body);
        assert_eq!(
            ids,
            vec!["claude-3-5-sonnet-20241022", "claude-3-5-haiku-20241022"]
        );
    }

    #[test]
    fn parse_openai_models_response_empty_and_malformed() {
        assert!(parse_openai_models_response(r#"{ "data": [] }"#).is_empty());
        assert!(parse_openai_models_response("garbage").is_empty());
        // Missing "data" key entirely
        assert!(parse_openai_models_response(r#"{ "object": "list" }"#).is_empty());
    }

    #[test]
    fn provider_connection_result_with_models_serializes() {
        // When models is non-empty it should appear in the JSON payload so the
        // WinUI / mobile client can render a model picker (#3489).
        let r = ProviderConnectionResult {
            ok: true,
            status: Some(200),
            error: None,
            probe_url: Some("http://localhost:11434/api/tags".into()),
            models: vec!["llama3.2:latest".into(), "mistral:7b".into()],
        };
        let v: Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(
            v["models"],
            serde_json::json!(["llama3.2:latest", "mistral:7b"])
        );
    }

    // ── Flashcard IPC params tests (#3763) ──────────────────────

    #[test]
    fn create_flashcard_params_deserialize() {
        let json = serde_json::json!({
            "front": "What is FSRS?",
            "back": "Free Spaced Repetition Scheduler",
            "sourceNoteId": "note-abc",
            "tags": ["algorithms", "memory"]
        });
        let params: CreateFlashcardParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.front, "What is FSRS?");
        assert_eq!(params.back, "Free Spaced Repetition Scheduler");
        assert_eq!(params.source_note_id.as_deref(), Some("note-abc"));
        assert_eq!(params.tags, vec!["algorithms", "memory"]);
    }

    #[test]
    fn create_flashcard_params_optional_fields_default() {
        // source_note_id and tags should default when omitted.
        let json = serde_json::json!({
            "front": "2 + 2?",
            "back": "4"
        });
        let params: CreateFlashcardParams = serde_json::from_value(json).unwrap();
        assert!(params.source_note_id.is_none());
        assert!(params.tags.is_empty());
    }

    #[test]
    fn review_flashcard_params_deserialize_all_ratings() {
        for rating in ["again", "hard", "good", "easy"] {
            let json = serde_json::json!({
                "id": "card-123",
                "rating": rating
            });
            let params: ReviewFlashcardParams = serde_json::from_value(json)
                .unwrap_or_else(|e| panic!("failed to deserialize rating '{rating}': {e}"));
            assert_eq!(params.id, "card-123");
        }
    }

    #[test]
    fn review_flashcard_params_rejects_invalid_rating() {
        let json = serde_json::json!({
            "id": "card-123",
            "rating": "medium"
        });
        assert!(serde_json::from_value::<ReviewFlashcardParams>(json).is_err());
    }

    // ── Regression: #3769 concurrent runAgent race on AGENT_APPROVAL ────
    // NOTE: These tests share the global ACTIVE_AGENT / AGENT_APPROVAL
    // statics, so they are combined into a single test to avoid races
    // between parallel test threads.

    /// Verifies the agent lifecycle state machine used by #3769 fix:
    /// - ACTIVE_AGENT starts None
    /// - Setting / clearing tracks agent lifecycle correctly
    /// - AGENT_APPROVAL and ACTIVE_AGENT are independently managed
    #[test]
    fn regression_3769_agent_approval_race_fix() {
        // ── Phase 1: initial state ──
        {
            let guard = ACTIVE_AGENT.lock().unwrap_or_else(|e| e.into_inner());
            assert!(guard.is_none(), "ACTIVE_AGENT should start as None");
        }

        // ── Phase 2: lifecycle — set, verify, clear ──
        let handle = std::thread::spawn(|| {});
        *ACTIVE_AGENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);

        {
            let guard = ACTIVE_AGENT.lock().unwrap_or_else(|e| e.into_inner());
            assert!(guard.is_some(), "should be busy while agent runs");
        }

        // Join and clear
        let handle = ACTIVE_AGENT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .expect("should have a handle");
        handle.join().unwrap();

        {
            let guard = ACTIVE_AGENT.lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                guard.is_none(),
                "ACTIVE_AGENT should be None after agent finishes"
            );
        }

        // ── Phase 3: approval channel independence ──
        let handle = std::thread::spawn(|| {});
        *ACTIVE_AGENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);

        let (tx, _rx) = std::sync::mpsc::channel();
        *AGENT_APPROVAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);

        // Clear AGENT_APPROVAL first (as run_agent_task does)
        *AGENT_APPROVAL.lock().unwrap_or_else(|e| e.into_inner()) = None;
        assert!(AGENT_APPROVAL
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none());

        // ACTIVE_AGENT should still be set (independent)
        assert!(ACTIVE_AGENT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some());

        // Cleanup
        let handle = ACTIVE_AGENT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap();
        handle.join().unwrap();
    }

    // ── Regression: #3788 shutdown deadlock + stale run-guard race ──────
    // NOTE: shares the AGENT_RUNNING / AGENT_SHUTDOWN statics; kept in a
    // single test so parallel test threads cannot race one another.

    /// Verifies the #3788 fixes:
    /// - Bug A: the write-approval wait promptly aborts (denies the write)
    ///   once the process is shutting down after stdin EOF, so main()'s
    ///   join() cannot deadlock on an approval that will never arrive.
    /// - Bug B: the concurrent-run guard is released after a task finishes,
    ///   so a fast task cannot leave a stale "running" state that rejects
    ///   every later runAgent call until process restart.
    #[test]
    fn regression_3788_agent_shutdown_and_run_guard() {
        // ── Phase 1: Bug A — approval wait aborts on shutdown ──
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx; // never send — simulates stdin EOF with a pending approval
            AGENT_SHUTDOWN.store(true, Ordering::SeqCst);
            let started = std::time::Instant::now();
            let decision = await_write_approval(rx);
            assert!(!decision, "approval must be denied once shutting down");
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "approval wait must not block forever after shutdown"
            );
            AGENT_SHUTDOWN.store(false, Ordering::SeqCst);
        }

        // ── Phase 2: Bug A — normal user approval still honored ──
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let waiter = std::thread::spawn(move || await_write_approval(rx));
            tx.send(true).unwrap();
            assert!(waiter.join().unwrap(), "user approval must be honored");
        }

        // ── Phase 3: Bug B — atomic concurrent-run guard ──
        {
            // First acquisition succeeds
            assert!(AGENT_RUNNING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok());
            // Concurrent acquisition is rejected
            assert!(AGENT_RUNNING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err());
            // Fast task finishes and clears the guard → next run is allowed
            AGENT_RUNNING.store(false, Ordering::SeqCst);
            assert!(AGENT_RUNNING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok());
            AGENT_RUNNING.store(false, Ordering::SeqCst);
        }
    }
}
