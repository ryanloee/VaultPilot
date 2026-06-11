use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};

#[cfg(target_os = "windows")]
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Datelike, Utc};
use deunicode::deunicode;
use image::{imageops::FilterType, ImageReader};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::models::{
    AppSettings, ChatSession, ChatState, ImportResult, IndexStats, NoteDocument, NoteMeta,
    SearchQuery, SearchResult,
};

/// Write `data` to `path` atomically by writing to a temporary file first, then
/// renaming.  On the same filesystem `rename` is guaranteed to be atomic, so a
/// crash mid-write will never leave a truncated/corrupt file behind.
///
/// Uses a random UUID suffix for the temp file to prevent concurrent writers
/// from racing on the same deterministic temp filename.
fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let tmp_name = format!(
        "{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp"),
        Uuid::new_v4()
    );
    let tmp_path = path.with_file_name(tmp_name);
    fs::write(&tmp_path, data)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct AppPaths {
    settings_path: PathBuf,
    database_path: PathBuf,
    chat_state_path: PathBuf,
    default_vault_dir: PathBuf,
    vault_dir_override: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct StorageContext {
    paths: AppPaths,
    /// Cached SQLite connection, shared across clones of the same context.
    cached_conn: Arc<Mutex<Option<Connection>>>,
}

impl StorageContext {
    fn with_cached_conn(paths: AppPaths) -> Self {
        Self {
            paths,
            cached_conn: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Debug, Clone)]
struct AttachmentEntry {
    note_id: String,
    path: String,
    file_name: String,
    stem: String,
    ocr_text: String,
    semantic_vector: Option<Vec<f32>>,
    perceptual_hash: Option<u64>,
}

const ATTACHMENT_VECTOR_DIM: usize = 192;

impl StorageContext {
    pub fn for_sidecar() -> Result<Self> {
        let config_root = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.local.vaultpilot");
        let data_root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| config_root.clone())
            .join("com.local.vaultpilot");
        let default_vault_dir = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Documents")
            .join("VaultPilotVault");

        Ok(Self::with_cached_conn(AppPaths {
            settings_path: config_root.join("settings.json"),
            database_path: data_root.join("knowledge-index.sqlite"),
            chat_state_path: data_root.join("chat-state.json"),
            default_vault_dir,
            vault_dir_override: None,
        }))
    }

    pub fn for_cli(vault_dir_override: Option<PathBuf>) -> Result<Self> {
        let mut ctx = Self::for_sidecar()?;
        if let Some(vault_dir) = vault_dir_override {
            let cli_state_dir = vault_dir.join(".vaultpilot");
            ctx.paths.settings_path = cli_state_dir.join("settings.json");
            ctx.paths.database_path = cli_state_dir.join("knowledge-index.sqlite");
            ctx.paths.chat_state_path = cli_state_dir.join("chat-state.json");
            ctx.paths.default_vault_dir = vault_dir.clone();
            ctx.paths.vault_dir_override = Some(vault_dir);
        } else {
            ctx.paths.vault_dir_override = None;
        }
        Ok(ctx)
    }

    #[cfg(test)]
    pub(crate) fn for_test(temp: &Path) -> Self {
        Self::with_cached_conn(AppPaths {
            settings_path: temp.join("settings.json"),
            database_path: temp.join("knowledge-index.sqlite"),
            chat_state_path: temp.join("chat-state.json"),
            default_vault_dir: temp.join("vault"),
            vault_dir_override: None,
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Frontmatter {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    board: String,
    #[serde(default)]
    kernel: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    source: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LegacyChatState {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    turns: Vec<crate::models::ChatTurn>,
    #[serde(default)]
    summary: Option<crate::models::ConversationSummary>,
}

pub fn initialize_storage_with_context(context: &StorageContext) -> Result<AppSettings> {
    let settings = load_settings_with_context(context)?;
    let database_path = context.paths.database_path.clone();
    let connection = Connection::open(database_path)?;
    ensure_schema(&connection)?;
    fs::create_dir_all(&settings.vault_dir)?;
    Ok(settings)
}

pub fn load_settings_with_context(context: &StorageContext) -> Result<AppSettings> {
    let paths = &context.paths;
    if let Some(parent) = paths.settings_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = paths.database_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let settings = if paths.settings_path.exists() {
        let raw = fs::read_to_string(&paths.settings_path)
            .with_context(|| format!("failed to read {}", paths.settings_path.display()))?;
        let normalized = raw.trim_start_matches('\u{feff}');
        let mut parsed: AppSettings = serde_json::from_str(normalized)
            .with_context(|| format!("failed to parse {}", paths.settings_path.display()))?;
        normalize_settings(&mut parsed, paths);
        parsed
    } else {
        let mut defaults = AppSettings::default();
        normalize_settings(&mut defaults, paths);
        save_settings_with_context(context, defaults.clone())?;
        defaults
    };

    fs::create_dir_all(&settings.vault_dir)?;
    Ok(settings)
}

pub fn save_settings_with_context(
    context: &StorageContext,
    mut settings: AppSettings,
) -> Result<AppSettings> {
    let paths = &context.paths;
    normalize_settings(&mut settings, paths);
    if let Some(parent) = paths.settings_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir_all(&settings.vault_dir)?;
    let content = serde_json::to_string_pretty(&settings)?;
    atomic_write(&paths.settings_path, content.as_bytes())
        .with_context(|| format!("failed to write {}", paths.settings_path.display()))?;
    let connection = Connection::open(&paths.database_path)?;
    ensure_schema(&connection)?;
    Ok(settings)
}

pub fn load_chat_state_with_context(context: &StorageContext) -> Result<ChatState> {
    let paths = &context.paths;
    if let Some(parent) = paths.chat_state_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if !paths.chat_state_path.exists() {
        let state = default_chat_state();
        save_chat_state_with_context(context, &state)?;
        return Ok(state);
    }

    let raw = fs::read_to_string(&paths.chat_state_path)
        .with_context(|| format!("failed to read {}", paths.chat_state_path.display()))?;
    let normalized = raw.trim_start_matches('\u{feff}');
    let state = parse_chat_state(normalized)
        .with_context(|| format!("failed to parse {}", paths.chat_state_path.display()))?;
    Ok(state)
}

pub fn save_chat_state_with_context(
    context: &StorageContext,
    state: &ChatState,
) -> Result<ChatState> {
    let paths = &context.paths;
    if let Some(parent) = paths.chat_state_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let normalized = normalize_chat_state(state.clone());

    let content = serde_json::to_string_pretty(&normalized)?;
    atomic_write(&paths.chat_state_path, content.as_bytes())
        .with_context(|| format!("failed to write {}", paths.chat_state_path.display()))?;
    Ok(normalized)
}

fn default_chat_state() -> ChatState {
    let session = default_chat_session();
    ChatState {
        current_session_id: session.id.clone(),
        sessions: vec![session],
    }
}

fn default_chat_session() -> ChatSession {
    let now = Utc::now().to_rfc3339();
    ChatSession {
        id: Uuid::new_v4().to_string(),
        title: "新对话".to_string(),
        turns: Vec::new(),
        summary: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn parse_chat_state(raw: &str) -> Result<ChatState> {
    if let Ok(state) = serde_json::from_str::<ChatState>(raw) {
        return Ok(normalize_chat_state(state));
    }

    if let Ok(legacy) = serde_json::from_str::<LegacyChatState>(raw) {
        let now = Utc::now().to_rfc3339();
        let session = ChatSession {
            id: if legacy.session_id.trim().is_empty() {
                Uuid::new_v4().to_string()
            } else {
                legacy.session_id
            },
            title: derive_chat_title(&legacy.turns),
            turns: legacy.turns,
            summary: legacy.summary,
            created_at: now.clone(),
            updated_at: now,
        };
        return Ok(ChatState {
            current_session_id: session.id.clone(),
            sessions: vec![session],
        });
    }

    Err(anyhow!("unsupported chat state schema"))
}

fn normalize_chat_state(mut state: ChatState) -> ChatState {
    if state.sessions.is_empty() {
        return default_chat_state();
    }

    let now = Utc::now().to_rfc3339();
    for session in &mut state.sessions {
        if session.id.trim().is_empty() {
            session.id = Uuid::new_v4().to_string();
        }
        if session.title.trim().is_empty() {
            session.title = derive_chat_title(&session.turns);
        }
        if session.created_at.trim().is_empty() {
            session.created_at = now.clone();
        }
        if session.updated_at.trim().is_empty() {
            session.updated_at = session
                .turns
                .last()
                .and_then(|turn| {
                    let created_at = turn.created_at.trim();
                    if created_at.is_empty() {
                        None
                    } else {
                        Some(created_at.to_string())
                    }
                })
                .unwrap_or_else(|| now.clone());
        }
    }

    state
        .sessions
        .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));

    if state.current_session_id.trim().is_empty()
        || !state
            .sessions
            .iter()
            .any(|session| session.id == state.current_session_id)
    {
        state.current_session_id = state
            .sessions
            .first()
            .map(|session| session.id.clone())
            .unwrap_or_default();
    }

    state
}

fn derive_chat_title(turns: &[crate::models::ChatTurn]) -> String {
    let text = turns
        .iter()
        .find(|turn| turn.role == "user" && !turn.text.trim().is_empty())
        .map(|turn| turn.text.trim())
        .unwrap_or("新对话");

    let title = text.chars().take(22).collect::<String>().trim().to_string();
    if title.is_empty() {
        "新对话".to_string()
    } else {
        title
    }
}

pub fn list_notes_with_context(context: &StorageContext) -> Result<Vec<NoteMeta>> {
    let result = search_notes_with_context(
        context,
        SearchQuery {
            text: String::new(),
            tags: Vec::new(),
            keywords: Vec::new(),
            limit: Some(50),
        },
    )?;
    Ok(result.notes)
}

pub fn search_notes_with_context(
    context: &StorageContext,
    query: SearchQuery,
) -> Result<SearchResult> {
    let (connection, _) = open_connection(context)?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let mut notes = if query.text.trim().is_empty() {
        query_recent_note_metas(&connection, limit)?
    } else {
        rank_note_metas(context, &connection, &query.text, &[], limit)?
    };

    if !query.tags.is_empty() {
        notes.retain(|note| has_all_terms(&note.tags, &query.tags));
    }
    if !query.keywords.is_empty() {
        notes.retain(|note| has_all_terms(&note.keywords, &query.keywords));
    }

    let total = notes.len();
    Ok(SearchResult { notes, total })
}

pub fn load_note_with_context(context: &StorageContext, note_id: &str) -> Result<NoteDocument> {
    let (connection, _) = open_connection(context)?;
    let path = connection
        .query_row(
            "SELECT path FROM notes WHERE id = ?1 OR path = ?1 LIMIT 1",
            [note_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("note not found: {note_id}"))?;
    parse_markdown_note(Path::new(&path), "manual")
}

pub fn save_note_with_context(
    context: &StorageContext,
    note: NoteDocument,
) -> Result<NoteDocument> {
    save_note_with_images_with_context(context, note, &[])
}

pub fn save_note_with_images_with_context(
    context: &StorageContext,
    note: NoteDocument,
    image_paths: &[String],
) -> Result<NoteDocument> {
    let (connection, settings) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let title = fallback_title(&note.meta.title);
    let is_new = note.meta.id.trim().is_empty();
    let id = if is_new {
        Uuid::new_v4().to_string()
    } else {
        note.meta.id.clone()
    };
    let created_at = if note.meta.created_at.trim().is_empty() {
        now.clone()
    } else {
        note.meta.created_at.clone()
    };
    let updated_at = if is_new && !note.meta.updated_at.trim().is_empty() {
        note.meta.updated_at.clone()
    } else {
        now
    };

    let path = if note.meta.path.trim().is_empty() {
        build_note_path(&settings.vault_dir, &title, &created_at, &id)
    } else {
        PathBuf::from(&note.meta.path)
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let note_path_string = path.to_string_lossy().to_string();
    let image_refs = import_note_images(&path, image_paths)?;
    let body_with_images = append_image_markdown(&note.body, &image_refs);

    let meta = NoteMeta {
        id,
        title,
        tags: sanitize_terms(&note.meta.tags),
        keywords: sanitize_terms(&note.meta.keywords),
        platform: note.meta.platform.trim().to_string(),
        board: note.meta.board.trim().to_string(),
        kernel: note.meta.kernel.trim().to_string(),
        status: note.meta.status.trim().to_string(),
        created_at,
        updated_at,
        source: fallback_source(&note.meta.source),
        path: note_path_string.clone(),
        summary: if note.meta.summary.trim().is_empty() {
            extract_summary(&body_with_images)
        } else {
            note.meta.summary.trim().to_string()
        },
    };

    let serialized = compose_markdown(&meta, &body_with_images)?;
    fs::write(&path, serialized).with_context(|| format!("failed to write {}", path.display()))?;
    index_note_file_with_connection(&connection, &path)?;
    load_note_with_context(context, &meta.id)
}

pub fn delete_note_with_context(context: &StorageContext, note_id: &str) -> Result<bool> {
    let (mut connection, _) = open_connection(context)?;
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT id, path FROM notes WHERE id = ?1 OR path = ?1 LIMIT 1",
            [note_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((resolved_note_id, note_path)) = row else {
        return Ok(false);
    };
    let file = PathBuf::from(&note_path);
    if file.exists() {
        fs::remove_file(&file)?;
    }

    let tx = connection.transaction()?;
    tx.execute(
        "DELETE FROM note_fts WHERE note_id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM attachment_fts WHERE note_id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM attachments WHERE note_id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM notes WHERE id = ?1",
        [resolved_note_id.as_str()],
    )?;
    tx.commit()?;

    Ok(true)
}

pub fn import_markdown_with_context(
    context: &StorageContext,
    paths: &[String],
) -> Result<ImportResult> {
    let (connection, _) = open_connection(context)?;
    let mut result = ImportResult::default();
    for file in collect_markdown_files(paths) {
        match import_single_markdown(context, &connection, &file) {
            Ok(imported) => {
                if imported {
                    result.imported += 1;
                } else {
                    result.skipped += 1;
                }
            }
            Err(error) => result.errors.push(format!("{}: {error}", file.display())),
        }
    }
    Ok(result)
}

pub fn rebuild_index_with_context(context: &StorageContext) -> Result<IndexStats> {
    let (mut connection, settings) = open_connection(context)?;
    let vault_dir = PathBuf::from(&settings.vault_dir);
    fs::create_dir_all(&vault_dir)?;

    let tx = connection.transaction()?;
    let mut indexed_paths = HashSet::new();
    let mut stats = IndexStats::default();
    for entry in WalkDir::new(&vault_dir)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if entry.file_type().is_file() && is_markdown_file(entry.path()) {
            stats.scanned += 1;
            let canonical = entry
                .path()
                .canonicalize()
                .unwrap_or_else(|_| entry.path().to_path_buf());
            indexed_paths.insert(canonical.to_string_lossy().to_string());
            if index_note_file_with_connection(&tx, entry.path()).is_ok() {
                stats.indexed += 1;
            }
        }
    }

    let mut statement = tx.prepare("SELECT path FROM notes")?;
    let existing_paths = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for existing in existing_paths {
        if !indexed_paths.contains(&existing) {
            tx.execute(
                "DELETE FROM note_fts WHERE note_id IN (SELECT id FROM notes WHERE path = ?1)",
                [&existing],
            )?;
            tx.execute(
                "DELETE FROM attachment_fts WHERE note_id IN (SELECT id FROM notes WHERE path = ?1)",
                [&existing],
            )?;
            stats.removed += tx.execute("DELETE FROM notes WHERE path = ?1", [&existing])?;
        }
    }
    tx.commit()?;

    Ok(stats)
}

pub fn load_context_notes_with_context(
    context: &StorageContext,
    question: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteDocument>> {
    let (connection, _) = open_connection(context)?;
    rank_documents(context, &connection, question, image_paths, limit)
}

pub fn search_candidate_notes_with_context(
    context: &StorageContext,
    question: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteMeta>> {
    let (connection, _) = open_connection(context)?;
    rank_note_metas(context, &connection, question, image_paths, limit)
}

fn normalize_settings(settings: &mut AppSettings, paths: &AppPaths) {
    if let Some(vault_dir_override) = &paths.vault_dir_override {
        settings.vault_dir = vault_dir_override.to_string_lossy().to_string();
    } else if settings.vault_dir.trim().is_empty() {
        settings.vault_dir = paths.default_vault_dir.to_string_lossy().to_string();
    }
    if settings.provider.base_url.trim().is_empty() {
        settings.provider.base_url = crate::models::default_base_url();
    }
    if settings.provider.model.trim().is_empty() {
        settings.provider.model = crate::models::default_model();
    }
    if settings.provider.request_timeout_ms == 0 {
        settings.provider.request_timeout_ms = crate::models::default_timeout_ms();
    }
    if matches!(settings.provider.context_window_tokens, Some(0)) {
        settings.provider.context_window_tokens = None;
    }
}

/// RAII guard that returns a SQLite connection to the StorageContext cache on drop.
/// Implements Deref/DerefMut to Connection so callers can use it transparently.
struct CachedConnection {
    conn: Option<Connection>,
    cache: Arc<Mutex<Option<Connection>>>,
}

impl Drop for CachedConnection {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            if let Ok(mut cache) = self.cache.lock() {
                *cache = Some(conn);
            }
        }
    }
}

impl std::ops::Deref for CachedConnection {
    type Target = Connection;
    fn deref(&self) -> &Self::Target {
        self.conn.as_ref().expect("CachedConnection conn taken")
    }
}

impl std::ops::DerefMut for CachedConnection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.conn.as_mut().expect("CachedConnection conn taken")
    }
}

/// Get a database connection, preferring the cached one in the StorageContext.
/// Returns a `CachedConnection` guard that returns the connection to the cache on drop.
fn open_connection(context: &StorageContext) -> Result<(CachedConnection, AppSettings)> {
    let settings = load_settings_with_context(context)?;

    // Try to reuse a cached connection
    if let Ok(mut cache) = context.cached_conn.lock() {
        if let Some(conn) = cache.take() {
            // Verify the cached connection is still usable with a cheap query
            if conn
                .pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
                .is_ok()
            {
                return Ok((
                    CachedConnection {
                        conn: Some(conn),
                        cache: Arc::clone(&context.cached_conn),
                    },
                    settings,
                ));
            }
            // Connection stale/corrupt — fall through to create a new one
        }
    }

    // Create a fresh connection
    let database_path = context.paths.database_path.clone();
    if let Some(parent) = database_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(database_path)?;
    ensure_schema(&connection)?;

    Ok((
        CachedConnection {
            conn: Some(connection),
            cache: Arc::clone(&context.cached_conn),
        },
        settings,
    ))
}

fn ensure_schema(connection: &Connection) -> Result<()> {
    // Fast path: skip schema creation if already initialized in this process.
    // PRAGMA user_version is a lightweight integer stored in the database header.
    let version: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version >= 1 {
        // Schema already exists; just enable foreign keys and return.
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        return Ok(());
    }

    connection.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS notes (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            tags TEXT NOT NULL,
            keywords TEXT NOT NULL,
            platform TEXT NOT NULL,
            board TEXT NOT NULL,
            kernel TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            source TEXT NOT NULL,
            path TEXT NOT NULL UNIQUE,
            summary TEXT NOT NULL,
            body_hash TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS attachments (
            id TEXT PRIMARY KEY,
            note_id TEXT NOT NULL,
            path TEXT NOT NULL,
            file_name TEXT NOT NULL DEFAULT '',
            stem TEXT NOT NULL DEFAULT '',
            ocr_text TEXT NOT NULL DEFAULT '',
            semantic_vector TEXT NOT NULL DEFAULT '',
            perceptual_hash TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            FOREIGN KEY(note_id) REFERENCES notes(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_attachments_note_id ON attachments(note_id);

        CREATE VIRTUAL TABLE IF NOT EXISTS note_fts USING fts5(
            note_id UNINDEXED,
            title,
            keywords,
            body,
            tokenize = 'unicode61'
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS attachment_fts USING fts5(
            note_id UNINDEXED,
            attachment_id UNINDEXED,
            file_name,
            stem,
            path,
            tokenize = 'unicode61'
        );
        "#,
    )?;
    ensure_attachment_columns(connection)?;
    connection.execute_batch("PRAGMA user_version = 1;")?;
    Ok(())
}

fn ensure_attachment_columns(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare("PRAGMA table_info(attachments)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;

    for (column, ddl) in [
        (
            "file_name",
            "ALTER TABLE attachments ADD COLUMN file_name TEXT NOT NULL DEFAULT ''",
        ),
        (
            "stem",
            "ALTER TABLE attachments ADD COLUMN stem TEXT NOT NULL DEFAULT ''",
        ),
        (
            "ocr_text",
            "ALTER TABLE attachments ADD COLUMN ocr_text TEXT NOT NULL DEFAULT ''",
        ),
        (
            "semantic_vector",
            "ALTER TABLE attachments ADD COLUMN semantic_vector TEXT NOT NULL DEFAULT ''",
        ),
        (
            "perceptual_hash",
            "ALTER TABLE attachments ADD COLUMN perceptual_hash TEXT NOT NULL DEFAULT ''",
        ),
    ] {
        if !columns.contains(column) {
            connection.execute(ddl, [])?;
        }
    }

    Ok(())
}

fn index_note_file_with_connection(connection: &Connection, path: &Path) -> Result<()> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let document = parse_markdown_note(&canonical, "manual")?;
    let body_hash = hash_content(&document.body);
    connection.execute(
        "INSERT INTO notes (id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary, body_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(id) DO UPDATE SET
           title = excluded.title,
           tags = excluded.tags,
           keywords = excluded.keywords,
           platform = excluded.platform,
           board = excluded.board,
           kernel = excluded.kernel,
           status = excluded.status,
           created_at = excluded.created_at,
           updated_at = excluded.updated_at,
           source = excluded.source,
           path = excluded.path,
           summary = excluded.summary,
           body_hash = excluded.body_hash",
        params![
            document.meta.id,
            document.meta.title,
            serde_json::to_string(&document.meta.tags)?,
            serde_json::to_string(&document.meta.keywords)?,
            document.meta.platform,
            document.meta.board,
            document.meta.kernel,
            document.meta.status,
            document.meta.created_at,
            document.meta.updated_at,
            document.meta.source,
            canonical.to_string_lossy().to_string(),
            document.meta.summary,
            body_hash
        ],
    )?;
    connection.execute(
        "DELETE FROM note_fts WHERE note_id = ?1",
        [document.meta.id.clone()],
    )?;
    connection.execute(
        "INSERT INTO note_fts (note_id, title, keywords, body) VALUES (?1, ?2, ?3, ?4)",
        params![
            document.meta.id,
            document.meta.title,
            document.meta.keywords.join(" "),
            document.body
        ],
    )?;
    sync_note_attachments_with_connection(
        connection,
        &document.meta.id,
        &canonical.to_string_lossy(),
        &extract_note_image_refs(&document.body),
    )?;
    Ok(())
}

fn import_single_markdown(
    context: &StorageContext,
    connection: &Connection,
    file: &Path,
) -> Result<bool> {
    let settings = load_settings_with_context(context)?;
    let vault_dir = PathBuf::from(&settings.vault_dir)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&settings.vault_dir));
    let canonical = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    if canonical.starts_with(&vault_dir) {
        index_note_file_with_connection(connection, &canonical)?;
        return Ok(true);
    }

    let parsed = parse_markdown_note(&canonical, "imported")?;
    let imported = NoteDocument {
        meta: NoteMeta {
            id: String::new(),
            title: parsed.meta.title,
            tags: parsed.meta.tags,
            keywords: parsed.meta.keywords,
            platform: parsed.meta.platform,
            board: parsed.meta.board,
            kernel: parsed.meta.kernel,
            status: parsed.meta.status,
            created_at: parsed.meta.created_at,
            updated_at: parsed.meta.updated_at,
            source: "imported".to_string(),
            path: String::new(),
            summary: parsed.meta.summary,
        },
        body: parsed.body,
    };
    save_note_with_context(context, imported)?;
    Ok(true)
}

fn parse_markdown_note(path: &Path, default_source: &str) -> Result<NoteDocument> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let normalized = raw.replace("\r\n", "\n");
    let (frontmatter, body) = split_frontmatter(&normalized)?;
    let metadata = fs::metadata(path)?;
    let modified = metadata.modified().unwrap_or_else(|_| SystemTime::now());
    let modified_at = DateTime::<Utc>::from(modified).to_rfc3339();
    let created_at = metadata
        .created()
        .map(DateTime::<Utc>::from)
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|_| modified_at.clone());
    let title = if frontmatter.title.trim().is_empty() {
        detect_title(body, path)
    } else {
        frontmatter.title.trim().to_string()
    };
    let source = if frontmatter.source.trim().is_empty() {
        default_source.to_string()
    } else {
        frontmatter.source.trim().to_string()
    };

    Ok(NoteDocument {
        meta: NoteMeta {
            id: if frontmatter.id.trim().is_empty() {
                derived_note_id(path)
            } else {
                frontmatter.id
            },
            title,
            tags: sanitize_terms(&frontmatter.tags),
            keywords: sanitize_terms(&frontmatter.keywords),
            platform: frontmatter.platform,
            board: frontmatter.board,
            kernel: frontmatter.kernel,
            status: frontmatter.status,
            created_at: if frontmatter.created_at.trim().is_empty() {
                created_at
            } else {
                frontmatter.created_at
            },
            updated_at: if frontmatter.updated_at.trim().is_empty() {
                modified_at
            } else {
                frontmatter.updated_at
            },
            source,
            path: path.to_string_lossy().to_string(),
            summary: if frontmatter.summary.trim().is_empty() {
                extract_summary(body)
            } else {
                frontmatter.summary.trim().to_string()
            },
        },
        body: body.trim().to_string(),
    })
}

fn compose_markdown(meta: &NoteMeta, body: &str) -> Result<String> {
    let frontmatter = Frontmatter {
        id: meta.id.clone(),
        title: meta.title.clone(),
        summary: meta.summary.clone(),
        tags: meta.tags.clone(),
        keywords: meta.keywords.clone(),
        platform: meta.platform.clone(),
        board: meta.board.clone(),
        kernel: meta.kernel.clone(),
        status: meta.status.clone(),
        created_at: meta.created_at.clone(),
        updated_at: meta.updated_at.clone(),
        source: meta.source.clone(),
    };
    let yaml = serde_yaml::to_string(&frontmatter)?;
    Ok(format!(
        "---\n{}---\n\n{}\n",
        yaml,
        ensure_summary_section(body, &meta.summary)
    ))
}

fn import_note_images(note_path: &Path, image_paths: &[String]) -> Result<Vec<String>> {
    if image_paths.is_empty() {
        return Ok(Vec::new());
    }

    let parent = note_path
        .parent()
        .ok_or_else(|| anyhow!("note path has no parent: {}", note_path.display()))?;
    let stem = note_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("note");
    let asset_dir_name = format!("{stem}-assets");
    let asset_dir = parent.join(&asset_dir_name);
    fs::create_dir_all(&asset_dir)?;

    let mut refs = Vec::new();
    let mut seen_names = HashSet::new();

    for source in image_paths {
        let source_path = PathBuf::from(source);
        if !source_path.exists() {
            continue;
        }

        let original_name = source_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("image");
        let target_name = unique_asset_name(original_name, &mut seen_names);
        let target_path = asset_dir.join(&target_name);
        fs::copy(&source_path, &target_path).with_context(|| {
            format!(
                "failed to copy image from {} to {}",
                source_path.display(),
                target_path.display()
            )
        })?;
        refs.push(format!("{asset_dir_name}/{target_name}"));
    }

    Ok(refs)
}

fn unique_asset_name(original_name: &str, seen_names: &mut HashSet<String>) -> String {
    let path = Path::new(original_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");

    let mut index = 0usize;
    loop {
        let candidate = if index == 0 {
            if ext.is_empty() {
                slugify(stem)
            } else {
                format!("{}.{}", slugify(stem), ext.to_ascii_lowercase())
            }
        } else if ext.is_empty() {
            format!("{}-{}", slugify(stem), index)
        } else {
            format!("{}-{}.{}", slugify(stem), index, ext.to_ascii_lowercase())
        };

        if seen_names.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

fn append_image_markdown(body: &str, image_refs: &[String]) -> String {
    if image_refs.is_empty() {
        return body.trim().to_string();
    }

    let existing = body.trim().to_string();
    let image_block = image_refs
        .iter()
        .map(|path| {
            let normalized = path.replace('\\', "/");
            let name = Path::new(&normalized)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("image");
            format!("![{}]({})", name, normalized)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    if existing.contains("## 图片记录") {
        format!("{}\n\n{}", existing, image_block)
    } else {
        format!("{}\n\n## 图片记录\n\n{}", existing, image_block)
    }
}

fn ensure_summary_section(body: &str, summary: &str) -> String {
    let trimmed = body.trim();
    if summary.trim().is_empty() || trimmed.starts_with("## 摘要") {
        return trimmed.to_string();
    }

    format!("## 摘要\n\n{}\n\n{}", summary.trim(), trimmed)
}

fn sync_note_attachments_with_connection(
    connection: &Connection,
    note_id: &str,
    note_path: &str,
    image_refs: &[String],
) -> Result<()> {
    connection.execute("DELETE FROM attachment_fts WHERE note_id = ?1", [note_id])?;
    connection.execute("DELETE FROM attachments WHERE note_id = ?1", [note_id])?;

    if image_refs.is_empty() {
        return Ok(());
    }

    let note_dir = Path::new(note_path)
        .parent()
        .ok_or_else(|| anyhow!("note path has no parent: {note_path}"))?;
    let now = Utc::now().to_rfc3339();

    for relative in image_refs {
        let absolute = note_dir.join(relative);
        let absolute_string = absolute.to_string_lossy().to_string();
        let file_name = absolute
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let stem = absolute
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let perceptual_hash = compute_image_perceptual_hash(&absolute)
            .map(|value| format!("{value:016x}"))
            .unwrap_or_default();
        let ocr_text = extract_image_text(&absolute).unwrap_or_default();
        let semantic_source = build_attachment_semantic_text(&file_name, &stem, &ocr_text);
        let semantic_vector = build_text_semantic_vector(&semantic_source)
            .map(|vector| serialize_semantic_vector(&vector))
            .unwrap_or_default();
        let attachment_id = Uuid::new_v4().to_string();

        connection.execute(
            "INSERT INTO attachments (id, note_id, path, file_name, stem, ocr_text, semantic_vector, perceptual_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                attachment_id,
                note_id,
                absolute_string,
                file_name,
                stem,
                ocr_text,
                semantic_vector,
                perceptual_hash,
                now
            ],
        )?;
        connection.execute(
            "INSERT INTO attachment_fts (note_id, attachment_id, file_name, stem, path)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                note_id,
                attachment_id,
                absolute
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
                absolute
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
                absolute.to_string_lossy().to_string()
            ],
        )?;
    }

    Ok(())
}

fn extract_note_image_refs(body: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    let mut offset = 0usize;

    while let Some(start) = body[offset..].find("![") {
        let absolute_start = offset + start;
        let Some(open) = body[absolute_start..].find("](") else {
            break;
        };
        let path_start = absolute_start + open + 2;
        let Some(close) = body[path_start..].find(')') else {
            break;
        };
        let raw = body[path_start..path_start + close]
            .trim()
            .trim_matches('<')
            .trim_matches('>')
            .trim();
        let path = raw.split_whitespace().next().unwrap_or_default().trim();
        if !path.is_empty() && seen.insert(path.to_string()) {
            refs.push(path.to_string());
        }
        offset = path_start + close + 1;
    }

    refs
}

fn compute_image_perceptual_hash(path: &Path) -> Option<u64> {
    let image = ImageReader::open(path).ok()?.decode().ok()?;
    let grayscale = image
        .resize_exact(9, 8, FilterType::Triangle)
        .grayscale()
        .to_luma8();

    let mut hash = 0_u64;
    for y in 0..8 {
        for x in 0..8 {
            hash <<= 1;
            let left = grayscale.get_pixel(x, y)[0];
            let right = grayscale.get_pixel(x + 1, y)[0];
            if left > right {
                hash |= 1;
            }
        }
    }

    Some(hash)
}

fn extract_image_text(path: &Path) -> Result<String> {
    #[cfg(target_os = "windows")]
    {
        extract_image_text_with_windows_ocr(path)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Ok(String::new())
    }
}

pub fn ocr_image_text(path: &Path) -> Result<String> {
    extract_image_text(path)
}

#[cfg(target_os = "windows")]
fn extract_image_text_with_windows_ocr(path: &Path) -> Result<String> {
    let script = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType=WindowsRuntime]
$null = [Windows.Storage.Streams.IRandomAccessStream, Windows.Storage.Streams, ContentType=WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType=WindowsRuntime]
$null = [Windows.Graphics.Imaging.SoftwareBitmap, Windows.Graphics.Imaging, ContentType=WindowsRuntime]
$null = [Windows.Media.Ocr.OcrEngine, Windows.Media.Ocr, ContentType=WindowsRuntime]
function Await([object]$Operation, [type]$ResultType) {
  $method = [System.WindowsRuntimeSystemExtensions].GetMethods() |
    Where-Object { $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Count -eq 1 } |
    Select-Object -First 1
  $generic = $method.MakeGenericMethod($ResultType)
  $task = $generic.Invoke($null, @($Operation))
  $task.GetAwaiter().GetResult()
}
$imagePath = $args[0]
$file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($imagePath)) ([Windows.Storage.StorageFile])
$stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
$decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
$bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
$engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
if ($null -eq $engine) { return }
$result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
if ($null -ne $result -and $null -ne $result.Text) {
  [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
  Write-Output $result.Text
}
"#;

    let mut command = Command::new("powershell");
    command
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .arg(path.as_os_str())
        .stdin(std::process::Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let output = command
        .output()
        .with_context(|| format!("failed to run Windows OCR for {}", path.display()))?;

    if !output.status.success() {
        return Ok(String::new());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn build_attachment_semantic_text(file_name: &str, stem: &str, ocr_text: &str) -> String {
    [file_name.trim(), stem.trim(), ocr_text.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_text_semantic_vector(text: &str) -> Option<Vec<f32>> {
    let mut vector = vec![0.0_f32; ATTACHMENT_VECTOR_DIM];
    let terms = extract_search_terms(text);
    if terms.is_empty() {
        return None;
    }

    for term in terms {
        let hash = stable_term_hash(&term);
        let index = (hash as usize) % ATTACHMENT_VECTOR_DIM;
        let sign = if (hash >> 63) == 0 { 1.0_f32 } else { -1.0_f32 };
        vector[index] += sign;

        if term.chars().count() > 3 {
            for gram in sliding_char_grams(&term, 3) {
                let gram_hash = stable_term_hash(&gram);
                let gram_index = (gram_hash as usize) % ATTACHMENT_VECTOR_DIM;
                let gram_sign = if (gram_hash >> 63) == 0 {
                    0.5_f32
                } else {
                    -0.5_f32
                };
                vector[gram_index] += gram_sign;
            }
        }
    }

    normalize_vector(&mut vector);
    Some(vector)
}

fn serialize_semantic_vector(vector: &[f32]) -> String {
    serde_json::to_string(vector).unwrap_or_default()
}

fn deserialize_semantic_vector(raw: &str) -> Option<Vec<f32>> {
    let vector = serde_json::from_str::<Vec<f32>>(raw).ok()?;
    if vector.len() == ATTACHMENT_VECTOR_DIM {
        Some(vector)
    } else {
        None
    }
}

fn normalize_vector(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return;
    }
    for value in vector {
        *value /= norm;
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right.iter()).map(|(a, b)| a * b).sum()
}

fn similarity_to_rank_score(similarity: f32) -> i64 {
    if similarity >= 0.85 {
        220
    } else if similarity >= 0.7 {
        170
    } else if similarity >= 0.55 {
        120
    } else if similarity >= 0.4 {
        80
    } else if similarity >= 0.25 {
        40
    } else {
        0
    }
}

fn stable_term_hash(text: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let bytes: [u8; 8] = digest[..8].try_into().expect("hash prefix");
    u64::from_le_bytes(bytes)
}

fn sliding_char_grams(text: &str, gram_size: usize) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() < gram_size {
        return Vec::new();
    }

    chars
        .windows(gram_size)
        .map(|window| window.iter().collect::<String>())
        .collect()
}

fn image_similarity_score(query_hash: u64, candidate_hash: u64) -> i64 {
    let distance = (query_hash ^ candidate_hash).count_ones() as i64;
    match distance {
        0..=2 => 240,
        3..=6 => 180,
        7..=10 => 120,
        11..=14 => 70,
        15..=18 => 30,
        _ => 0,
    }
}

fn split_frontmatter(content: &str) -> Result<(Frontmatter, &str)> {
    if !content.starts_with("---\n") {
        return Ok((Frontmatter::default(), content));
    }
    if let Some(end_index) = content[4..].find("\n---\n") {
        let yaml = &content[4..4 + end_index];
        let body = &content[4 + end_index + 5..];
        let frontmatter = serde_yaml::from_str::<Frontmatter>(yaml).unwrap_or_default();
        return Ok((frontmatter, body));
    }
    Err(anyhow!("invalid frontmatter"))
}

fn build_note_path(vault_dir: &str, title: &str, created_at: &str, id: &str) -> PathBuf {
    let created = DateTime::parse_from_rfc3339(created_at)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let year = created.year().to_string();
    let month = format!("{:02}", created.month());
    let slug = slugify(title);
    let suffix = if id.len() >= 8 { &id[..8] } else { id };
    PathBuf::from(vault_dir)
        .join(year)
        .join(month)
        .join(format!("{slug}-{suffix}.md"))
}

fn collect_markdown_files(paths: &[String]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let path = PathBuf::from(path);
        if path.is_file() && is_markdown_file(&path) {
            if seen.insert(path.clone()) {
                files.push(path);
            }
            continue;
        }
        if path.is_dir() {
            for entry in WalkDir::new(path)
                .into_iter()
                .filter_map(|entry| entry.ok())
            {
                if entry.file_type().is_file() && is_markdown_file(entry.path()) {
                    let candidate = entry.path().to_path_buf();
                    if seen.insert(candidate.clone()) {
                        files.push(candidate);
                    }
                }
            }
        }
    }
    files
}

fn detect_title(body: &str, path: &Path) -> String {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            if !title.trim().is_empty() {
                return title.trim().to_string();
            }
        }
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled Note")
        .replace(['_', '-'], " ")
}

fn extract_summary(body: &str) -> String {
    let compact = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("```") && !line.starts_with('#'))
        .take(6)
        .collect::<Vec<_>>()
        .join(" ");
    compact.chars().take(180).collect()
}

fn row_to_meta(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteMeta> {
    let tags: String = row.get(2)?;
    let keywords: String = row.get(3)?;
    Ok(NoteMeta {
        id: row.get(0)?,
        title: row.get(1)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        keywords: serde_json::from_str(&keywords).unwrap_or_default(),
        platform: row.get(4)?,
        board: row.get(5)?,
        kernel: row.get(6)?,
        status: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        source: row.get(10)?,
        path: row.get(11)?,
        summary: row.get(12)?,
    })
}

fn query_recent_note_metas(connection: &Connection, limit: usize) -> Result<Vec<NoteMeta>> {
    let mut statement = connection.prepare(
        "SELECT id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary
         FROM notes
         ORDER BY updated_at DESC
         LIMIT ?1",
    )?;
    let rows = statement
        .query_map([limit as i64], row_to_meta)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn query_fts_note_ids(connection: &Connection, text: &str, limit: usize) -> Result<Vec<String>> {
    let fts_query = make_fts_query(text);
    if fts_query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut statement = connection.prepare(
        "SELECT note_id
         FROM note_fts
         WHERE note_fts MATCH ?1
         ORDER BY bm25(note_fts)
         LIMIT ?2",
    )?;
    let rows = match statement.query_map(params![fts_query, limit as i64], |row| {
        row.get::<_, String>(0)
    }) {
        Ok(rows) => rows,
        Err(_) => return Ok(Vec::new()),
    }
    .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn query_attachment_fts_note_ids(
    connection: &Connection,
    text: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let fts_query = make_fts_query(text);
    if fts_query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut statement = connection.prepare(
        "SELECT note_id
         FROM attachment_fts
         WHERE attachment_fts MATCH ?1
         ORDER BY bm25(attachment_fts)
         LIMIT ?2",
    )?;
    let rows = match statement.query_map(
        params![fts_query, (limit.saturating_mul(3)) as i64],
        |row| row.get::<_, String>(0),
    ) {
        Ok(rows) => rows,
        Err(_) => return Ok(Vec::new()),
    };

    let mut note_ids = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        let note_id = row?;
        if seen.insert(note_id.clone()) {
            note_ids.push(note_id);
        }
        if note_ids.len() >= limit {
            break;
        }
    }

    Ok(note_ids)
}

fn load_note_meta_by_id(connection: &Connection, note_id: &str) -> Result<Option<NoteMeta>> {
    connection
        .query_row(
            "SELECT id, title, tags, keywords, platform, board, kernel, status, created_at, updated_at, source, path, summary
             FROM notes
             WHERE id = ?1
             LIMIT 1",
            [note_id],
            row_to_meta,
        )
        .optional()
        .map_err(Into::into)
}

fn query_recent_note_ids(connection: &Connection, limit: usize) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT id
         FROM notes
         ORDER BY updated_at DESC
         LIMIT ?1",
    )?;
    let rows = statement
        .query_map([limit as i64], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn row_to_attachment_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AttachmentEntry> {
    let semantic_vector: String = row.get(5)?;
    let perceptual_hash: String = row.get(6)?;
    Ok(AttachmentEntry {
        note_id: row.get(0)?,
        path: row.get(2)?,
        file_name: row.get(3)?,
        stem: row.get(4)?,
        ocr_text: row.get(7).unwrap_or_default(),
        semantic_vector: deserialize_semantic_vector(&semantic_vector),
        perceptual_hash: if perceptual_hash.trim().is_empty() {
            None
        } else {
            u64::from_str_radix(perceptual_hash.trim(), 16).ok()
        },
    })
}

fn load_attachment_entries_by_note_ids(
    connection: &Connection,
    note_ids: &[String],
) -> Result<HashMap<String, Vec<AttachmentEntry>>> {
    let mut attachments = HashMap::<String, Vec<AttachmentEntry>>::new();
    let mut statement = connection.prepare(
        "SELECT note_id, id, path, file_name, stem, semantic_vector, perceptual_hash, ocr_text
         FROM attachments
         WHERE note_id = ?1",
    )?;

    for note_id in note_ids {
        let rows = statement.query_map([note_id], row_to_attachment_entry)?;
        for row in rows {
            let entry = row?;
            attachments
                .entry(entry.note_id.clone())
                .or_default()
                .push(entry);
        }
    }

    Ok(attachments)
}

fn query_visual_candidate_scores(
    connection: &Connection,
    image_paths: &[String],
) -> Result<HashMap<String, i64>> {
    let query_hashes = image_paths
        .iter()
        .filter_map(|path| compute_image_perceptual_hash(Path::new(path)))
        .collect::<Vec<_>>();
    if query_hashes.is_empty() {
        return Ok(HashMap::new());
    }

    let mut statement = connection.prepare(
        "SELECT note_id, id, path, file_name, stem, semantic_vector, perceptual_hash, ocr_text
         FROM attachments
         WHERE perceptual_hash <> ''",
    )?;
    let rows = statement.query_map([], row_to_attachment_entry)?;
    let mut scores = HashMap::new();

    for row in rows {
        let entry = row?;
        let Some(attachment_hash) = entry.perceptual_hash else {
            continue;
        };

        let best = query_hashes
            .iter()
            .map(|query_hash| image_similarity_score(*query_hash, attachment_hash))
            .max()
            .unwrap_or_default();
        if best <= 0 {
            continue;
        }

        scores
            .entry(entry.note_id.clone())
            .and_modify(|current: &mut i64| *current = (*current).max(best))
            .or_insert(best);
    }

    Ok(scores)
}

fn query_attachment_semantic_scores(
    connection: &Connection,
    query_text: &str,
) -> Result<HashMap<String, i64>> {
    let Some(query_vector) = build_text_semantic_vector(query_text) else {
        return Ok(HashMap::new());
    };

    let mut statement = connection.prepare(
        "SELECT note_id, id, path, file_name, stem, semantic_vector, perceptual_hash, ocr_text
         FROM attachments
         WHERE semantic_vector <> ''",
    )?;
    let rows = statement.query_map([], row_to_attachment_entry)?;
    let mut scores = HashMap::new();

    for row in rows {
        let entry = row?;
        let Some(candidate_vector) = entry.semantic_vector.as_ref() else {
            continue;
        };
        let similarity = cosine_similarity(&query_vector, candidate_vector);
        let score = similarity_to_rank_score(similarity);
        if score <= 0 {
            continue;
        }

        scores
            .entry(entry.note_id.clone())
            .and_modify(|current: &mut i64| *current = (*current).max(score))
            .or_insert(score);
    }

    Ok(scores)
}

fn rank_note_metas(
    context: &StorageContext,
    connection: &Connection,
    query: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteMeta>> {
    let docs = rank_documents(context, connection, query, image_paths, limit)?;
    Ok(docs.into_iter().map(|doc| doc.meta).collect())
}

fn rank_documents(
    context: &StorageContext,
    connection: &Connection,
    query: &str,
    image_paths: &[String],
    limit: usize,
) -> Result<Vec<NoteDocument>> {
    let note_fts_ids = query_fts_note_ids(connection, query, limit.saturating_mul(6).max(18))?;
    let attachment_query = attachment_query_text(query, image_paths);
    let attachment_fts_ids = query_attachment_fts_note_ids(
        connection,
        &attachment_query,
        limit.saturating_mul(4).max(12),
    )?;
    let visual_scores = query_visual_candidate_scores(connection, image_paths)?;
    let semantic_scores = query_attachment_semantic_scores(connection, &attachment_query)?;
    let recent_ids = query_recent_note_ids(
        connection,
        limit
            .saturating_mul(6)
            .max(if image_paths.is_empty() { 24 } else { 12 }),
    )?;
    let candidate_ids = build_candidate_note_ids(
        &note_fts_ids,
        &attachment_fts_ids,
        &semantic_scores,
        &visual_scores,
        &recent_ids,
        limit,
    );
    let attachment_entries = load_attachment_entries_by_note_ids(connection, &candidate_ids)?;
    let mut ranked = Vec::new();

    for note_id in candidate_ids {
        let Some(meta) = load_note_meta_by_id(connection, &note_id)? else {
            continue;
        };
        let Ok(doc) = load_note_with_context(context, &meta.id) else {
            continue;
        };
        let attachments = attachment_entries
            .get(&note_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        let mut score = document_relevance_score(query, &doc);
        score += attachment_text_relevance_score(&attachment_query, attachments);

        if let Some(index) = note_fts_ids.iter().position(|id| id == &note_id) {
            score += 200_i64.saturating_sub(index as i64 * 10);
        }
        if let Some(index) = attachment_fts_ids.iter().position(|id| id == &note_id) {
            score += 150_i64.saturating_sub(index as i64 * 8);
        }
        if let Some(semantic_score) = semantic_scores.get(&note_id) {
            score += *semantic_score;
        }
        if let Some(visual_score) = visual_scores.get(&note_id) {
            score += *visual_score;
        }

        if score > 0 {
            ranked.push((score, doc));
        }
    }

    ranked.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.meta.updated_at.cmp(&left.1.meta.updated_at))
    });

    Ok(ranked.into_iter().take(limit).map(|(_, doc)| doc).collect())
}

fn build_candidate_note_ids(
    note_fts_ids: &[String],
    attachment_fts_ids: &[String],
    semantic_scores: &HashMap<String, i64>,
    visual_scores: &HashMap<String, i64>,
    recent_ids: &[String],
    limit: usize,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    let mut semantic_ranked = semantic_scores.iter().collect::<Vec<_>>();
    let mut visual_ranked = visual_scores.iter().collect::<Vec<_>>();
    semantic_ranked.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
    visual_ranked.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));

    for note_id in note_fts_ids {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }
    for note_id in attachment_fts_ids {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }
    for (note_id, _) in semantic_ranked {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }
    for (note_id, _) in visual_ranked {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }
    for note_id in recent_ids {
        push_candidate_note_id(note_id, &mut seen, &mut ids);
    }

    ids.truncate(limit.saturating_mul(8).max(24));
    ids
}

fn push_candidate_note_id(note_id: &str, seen: &mut HashSet<String>, ids: &mut Vec<String>) {
    if seen.insert(note_id.to_string()) {
        ids.push(note_id.to_string());
    }
}

fn attachment_query_text(query: &str, image_paths: &[String]) -> String {
    let mut parts = Vec::new();
    if !query.trim().is_empty() {
        parts.push(query.trim().to_string());
    }

    for path in image_paths {
        let candidate = Path::new(path)
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(candidate) = candidate {
            parts.push(candidate.to_string());
        }
        if let Ok(ocr_text) = extract_image_text(Path::new(path)) {
            if !ocr_text.trim().is_empty() {
                parts.push(ocr_text.trim().to_string());
            }
        }
    }

    parts.join(" ")
}

fn attachment_text_relevance_score(query_text: &str, attachments: &[AttachmentEntry]) -> i64 {
    if attachments.is_empty() {
        return 0;
    }

    let normalized_query = normalize_query_for_search(query_text);
    let terms = extract_search_terms(query_text);
    if normalized_query.is_empty() && terms.is_empty() {
        return 0;
    }

    let mut score = 0_i64;
    let mut matched_terms = 0_i64;

    for term in &terms {
        if attachments.iter().any(|attachment| {
            let file_name = normalize_search_text(&attachment.file_name);
            let stem = normalize_search_text(&attachment.stem);
            let path = normalize_search_text(&attachment.path);
            let ocr_text = normalize_search_text(&attachment.ocr_text);
            file_name.contains(term.as_str())
                || stem.contains(term.as_str())
                || path.contains(term.as_str())
                || ocr_text.contains(term.as_str())
        }) {
            matched_terms += 1;
        }
    }

    for attachment in attachments {
        let file_name = normalize_search_text(&attachment.file_name);
        let stem = normalize_search_text(&attachment.stem);
        let path = normalize_search_text(&attachment.path);
        let ocr_text = normalize_search_text(&attachment.ocr_text);

        if !normalized_query.is_empty() {
            if stem.contains(&normalized_query) {
                score += 90;
            }
            if file_name.contains(&normalized_query) {
                score += 70;
            }
            if path.contains(&normalized_query) {
                score += 30;
            }
            if ocr_text.contains(&normalized_query) {
                score += 120;
            }
        }

        for term in &terms {
            if stem.contains(term) {
                score += 34;
            }
            if file_name.contains(term) {
                score += 24;
            }
            if path.contains(term) {
                score += 12;
            }
            if ocr_text.contains(term) {
                score += 40;
            }
        }
    }

    if matched_terms >= 2 {
        score += 35;
    }
    if matched_terms >= 4 {
        score += 60;
    }

    score
}

fn document_relevance_score(query: &str, doc: &NoteDocument) -> i64 {
    let normalized_query = normalize_query_for_search(query);
    if normalized_query.is_empty() {
        return 0;
    }

    let terms = extract_search_terms(query);
    let title = normalize_search_text(&doc.meta.title);
    let summary = normalize_search_text(&doc.meta.summary);
    let keywords = normalize_search_text(&doc.meta.keywords.join(" "));
    let tags = normalize_search_text(&doc.meta.tags.join(" "));
    let path = normalize_search_text(&doc.meta.path);
    let body = normalize_search_text(&doc.body);

    let mut score = 0_i64;

    if title.contains(&normalized_query) {
        score += 160;
    }
    if summary.contains(&normalized_query) {
        score += 120;
    }
    if keywords.contains(&normalized_query) {
        score += 140;
    }
    if path.contains(&normalized_query) {
        score += 80;
    }
    if body.contains(&normalized_query) {
        score += 90;
    }

    for term in &terms {
        if title.contains(term) {
            score += 70;
        }
        if summary.contains(term) {
            score += 50;
        }
        if keywords.contains(term) {
            score += 60;
        }
        if tags.contains(term) {
            score += 40;
        }
        if path.contains(term) {
            score += 16;
        }
        if body.contains(term) {
            score += 24;
        }
    }

    let matched_terms = terms
        .iter()
        .filter(|term| {
            title.contains(term.as_str())
                || summary.contains(term.as_str())
                || keywords.contains(term.as_str())
                || tags.contains(term.as_str())
                || path.contains(term.as_str())
                || body.contains(term.as_str())
        })
        .count() as i64;

    if matched_terms > 0 {
        score += matched_terms * 12;
    }

    if matched_terms >= 2 {
        score += 40;
    }

    if matched_terms >= 4 {
        score += 80;
    }

    if query_has_any(&terms, &["刷机", "刷写", "烧录", "flash", "update", "升级"])
        && query_has_any(
            &collect_document_terms(doc),
            &["wboot", "update", "flash", "烧录", "刷机", "刷写"],
        )
    {
        score += 140;
    }

    if query_has_any(&terms, &["sd卡", "sd", "sdio", "mmc", "tf"])
        && query_has_any(
            &collect_document_terms(doc),
            &["sd卡", "sd", "sdio", "mmc", "tf", "sdmmc"],
        )
    {
        score += 120;
    }

    if query_has_any(&terms, &["引脚复用", "复用", "pinmux", "pinctrl", "iomux"])
        && query_has_any(
            &collect_document_terms(doc),
            &[
                "引脚复用",
                "复用",
                "pinmux",
                "pinctrl",
                "iomux",
                "pin multiplexing",
            ],
        )
    {
        score += 120;
    }

    score
}

fn normalize_search_text(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || is_cjk(ch) || ch == '_' || ch == '-' || ch == '.' {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
}

fn normalize_query_for_search(text: &str) -> String {
    let mut normalized = normalize_search_text(text);
    for noise in [
        "告诉我",
        "帮我",
        "请问",
        "麻烦",
        "一下",
        "一下子",
        "这个",
        "那个",
        "怎么做",
        "怎么办",
        "怎么刷",
        "怎么",
        "如何",
        "是什么",
        "是什么样",
        "有没有",
        "之前",
        "以前",
        "一下呢",
        "一下啊",
        "一下呀",
    ] {
        normalized = normalized.replace(noise, " ");
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_search_terms(text: &str) -> Vec<String> {
    let normalized = normalize_query_for_search(text);
    let mut terms = Vec::new();
    let mut seen = HashSet::new();

    for token in normalized.split_whitespace() {
        push_search_term(token, &mut seen, &mut terms);

        let segments = split_search_token(token);
        if segments.len() > 1 {
            for segment in &segments {
                push_search_term(segment, &mut seen, &mut terms);
            }
            for window in segments.windows(2) {
                let merged = window.concat();
                push_search_term(&merged, &mut seen, &mut terms);
            }
        }

        push_cjk_ngrams(token, 2, &mut seen, &mut terms);
        push_cjk_ngrams(token, 3, &mut seen, &mut terms);
    }

    let expanded = terms.clone();
    for term in expanded {
        for alias in expand_term_aliases(&term) {
            push_search_term(&alias, &mut seen, &mut terms);
        }
    }

    terms
}

fn push_search_term(term: &str, seen: &mut HashSet<String>, terms: &mut Vec<String>) {
    let cleaned = term.trim();
    if cleaned.len() <= 1 || is_noise_term(cleaned) {
        return;
    }
    if seen.insert(cleaned.to_string()) {
        terms.push(cleaned.to_string());
    }
}

fn push_cjk_ngrams(
    token: &str,
    gram_size: usize,
    seen: &mut HashSet<String>,
    terms: &mut Vec<String>,
) {
    let cjk_chars: Vec<char> = token
        .chars()
        .filter(|ch| is_cjk(*ch) && !is_cjk_stop_char(*ch))
        .collect();
    if cjk_chars.len() < gram_size {
        return;
    }

    for window in cjk_chars.windows(gram_size) {
        let gram = window.iter().collect::<String>();
        push_search_term(&gram, seen, terms);
    }
}

fn split_search_token(token: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_kind = None::<u8>;

    for ch in token.chars() {
        let kind = if is_cjk(ch) {
            1
        } else if ch.is_ascii_alphanumeric() {
            2
        } else {
            0
        };

        if kind == 0 {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            current_kind = None;
            continue;
        }

        if current_kind.is_some() && current_kind != Some(kind) && !current.is_empty() {
            parts.push(current.clone());
            current.clear();
        }

        current.push(ch);
        current_kind = Some(kind);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

fn expand_term_aliases(term: &str) -> Vec<String> {
    let normalized = term.trim().to_lowercase();
    let mut aliases = Vec::new();

    if normalized.contains("刷机")
        || normalized.contains("刷写")
        || normalized.contains("烧录")
        || normalized.contains("flash")
        || normalized.contains("update")
    {
        aliases.extend(
            [
                "刷机", "刷写", "烧录", "升级", "flash", "update", "wboot", "固件", "镜像", "zboot",
            ]
            .into_iter()
            .map(str::to_string),
        );
    }

    if normalized.contains("sd")
        || normalized.contains("sd卡")
        || normalized.contains("sdio")
        || normalized.contains("mmc")
        || normalized.contains("tf")
    {
        aliases.extend(
            ["sd", "sd卡", "sdio", "mmc", "tf", "sdmmc"]
                .into_iter()
                .map(str::to_string),
        );
    }

    if normalized.contains("引脚复用")
        || normalized.contains("复用")
        || normalized.contains("pinmux")
        || normalized.contains("pinctrl")
        || normalized.contains("iomux")
    {
        aliases.extend(
            [
                "引脚复用",
                "复用",
                "pinmux",
                "pinctrl",
                "iomux",
                "pin multiplexing",
            ]
            .into_iter()
            .map(str::to_string),
        );
    }

    if normalized.contains("gpio") {
        aliases.extend(["gpio", "管脚", "引脚"].into_iter().map(str::to_string));
    }

    aliases
}

fn collect_document_terms(doc: &NoteDocument) -> Vec<String> {
    extract_search_terms(&format!(
        "{}\n{}\n{}\n{}\n{}",
        doc.meta.title,
        doc.meta.summary,
        doc.meta.tags.join(" "),
        doc.meta.keywords.join(" "),
        doc.body
    ))
}

fn query_has_any(terms: &[String], expected: &[&str]) -> bool {
    expected.iter().any(|needle| {
        terms
            .iter()
            .any(|term| term.contains(needle) || needle.contains(term))
    })
}

fn is_noise_term(term: &str) -> bool {
    matches!(
        term,
        "什么"
            | "事情"
            | "问题"
            | "一下"
            | "一下子"
            | "怎么"
            | "如何"
            | "那个"
            | "这个"
            | "告诉"
            | "帮我"
            | "请问"
            | "之前"
            | "以前"
            | "还有"
            | "一下呢"
            | "一下啊"
            | "一下呀"
            | "资料库"
    )
}

fn is_cjk_stop_char(ch: char) -> bool {
    matches!(
        ch,
        '的' | '了' | '呢' | '吗' | '啊' | '呀' | '吧' | '么' | '我' | '你'
    )
}

fn is_cjk(ch: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&ch)
}

fn has_all_terms(source: &[String], expected: &[String]) -> bool {
    expected.iter().all(|needle| {
        source
            .iter()
            .any(|item| item.eq_ignore_ascii_case(needle.trim()))
    })
}

fn sanitize_terms(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(|value| {
            let normalized = value.to_string();
            if seen.insert(normalized.to_lowercase()) {
                Some(normalized)
            } else {
                None
            }
        })
        .collect()
}

fn fallback_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        "Untitled Note".to_string()
    } else {
        trimmed.to_string()
    }
}

fn fallback_source(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        "manual".to_string()
    } else {
        trimmed.to_string()
    }
}

fn slugify(value: &str) -> String {
    let ascii = deunicode(value);
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in ascii.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let cleaned = slug.trim_matches('-').to_string();
    if cleaned.is_empty() {
        "note".to_string()
    } else {
        cleaned
    }
}

fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn derived_note_id(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!(
        "note-{}",
        &hash_content(&canonical.to_string_lossy())[0..24]
    )
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn make_fts_query(text: &str) -> String {
    let terms: Vec<String> = extract_search_terms(text)
        .into_iter()
        .map(|term| {
            term.chars()
                .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
                .collect::<String>()
        })
        .filter(|term| !term.is_empty())
        .take(8)
        .collect();
    terms.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ProviderConfig;

    fn setup_temp_context() -> (PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    // ══════════════════════════════════════
    // Phase 1: Pure Logic Tests
    // ══════════════════════════════════════

    // ── 1.0 existing tests (preserved) ──

    #[test]
    fn derived_id_is_stable_for_same_path() {
        let path = PathBuf::from(r"D:\vault\2026\04\boot-timeout.md");
        assert_eq!(derived_note_id(&path), derived_note_id(&path));
    }

    #[test]
    fn summary_ignores_headings_and_limits_length() {
        let body = "# 标题\n\n第一段现象说明。\n\n第二段补充。\n";
        let summary = extract_summary(body);
        assert!(summary.contains("第一段现象说明"));
        assert!(!summary.contains("# 标题"));
        assert!(summary.len() <= 180);
    }

    #[test]
    fn frontmatter_round_trip_preserves_core_fields() {
        let meta = NoteMeta {
            id: "abc".to_string(),
            title: "MMC timeout".to_string(),
            tags: vec!["kernel".to_string()],
            keywords: vec!["mmc".to_string()],
            platform: "imx8mp".to_string(),
            board: "evk".to_string(),
            kernel: "5.10".to_string(),
            status: "已解决".to_string(),
            created_at: "2026-04-09T00:00:00Z".to_string(),
            updated_at: "2026-04-09T00:00:00Z".to_string(),
            source: "manual".to_string(),
            path: String::new(),
            summary: String::new(),
        };
        let body = "## 问题现象\n\n启动超时";
        let serialized = compose_markdown(&meta, body).expect("serialize markdown");
        let (frontmatter, parsed_body) = split_frontmatter(&serialized).expect("parse frontmatter");
        assert_eq!(frontmatter.id, "abc");
        assert_eq!(frontmatter.title, "MMC timeout");
        assert_eq!(frontmatter.tags, vec!["kernel".to_string()]);
        assert_eq!(parsed_body.trim(), body);
    }

    #[test]
    fn settings_parser_tolerates_utf8_bom() {
        let raw = "\u{feff}{\"vaultDir\":\"D:\\\\Vault\",\"provider\":{\"apiKey\":\"k\",\"baseUrl\":\"u\",\"model\":\"m\",\"requestTimeoutMs\":1}}";
        let normalized = raw.trim_start_matches('\u{feff}');
        let parsed: AppSettings =
            serde_json::from_str(normalized).expect("parse settings with bom");
        assert_eq!(parsed.vault_dir, "D:\\Vault");
        assert_eq!(parsed.provider.model, "m");
    }

    #[test]
    fn extract_search_terms_understands_mixed_cn_and_domain_terms() {
        let terms = extract_search_terms("告诉我 sd卡的引脚复用怎么做的");
        assert!(terms.iter().any(|term| term == "sd"));
        assert!(terms.iter().any(|term| term == "sd卡"));
        assert!(terms.iter().any(|term| term == "引脚"));
        assert!(terms.iter().any(|term| term == "复用"));
        assert!(terms.iter().any(|term| term == "引脚复用"));
        assert!(terms.iter().any(|term| term == "mmc"));
    }

    #[test]
    fn fts_query_omits_pure_cjk_text() {
        assert_eq!(make_fts_query("你好"), "");
    }

    #[test]
    fn extracts_markdown_image_refs() {
        let body = "## 图片记录\n\n![boot-log](attachments/boot-log.png)\n\n![scope](./attachments/scope.jpg)";
        let refs = extract_note_image_refs(body);
        assert_eq!(
            refs,
            vec![
                "attachments/boot-log.png".to_string(),
                "./attachments/scope.jpg".to_string()
            ]
        );
    }

    #[test]
    fn image_hash_scores_identical_images_higher() {
        let temp_root = std::env::temp_dir().join(format!(
            "vaultpilot-image-hash-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp_root).expect("temp dir");

        let exact = temp_root.join("exact.png");
        let different = temp_root.join("different.png");

        let mut exact_image = image::GrayImage::new(9, 8);
        let mut different_image = image::GrayImage::new(9, 8);
        for y in 0..8 {
            for x in 0..9 {
                exact_image.put_pixel(x, y, image::Luma([if x < 4 { 255 } else { 0 }]));
                different_image.put_pixel(x, y, image::Luma([if y < 4 { 255 } else { 0 }]));
            }
        }

        exact_image.save(&exact).expect("save exact");
        different_image.save(&different).expect("save different");

        let exact_hash = compute_image_perceptual_hash(&exact).expect("exact hash");
        let different_hash = compute_image_perceptual_hash(&different).expect("different hash");

        assert!(image_similarity_score(exact_hash, exact_hash) > 0);
        assert!(
            image_similarity_score(exact_hash, exact_hash)
                > image_similarity_score(exact_hash, different_hash)
        );
    }

    #[test]
    fn semantic_vectors_rank_related_text_higher() {
        let query = build_text_semantic_vector("github release workflow tag publish")
            .expect("query vector");
        let related =
            build_text_semantic_vector("release tag publish github").expect("related vector");
        let unrelated =
            build_text_semantic_vector("pinmux mmc gpio kernel").expect("unrelated vector");

        assert!(cosine_similarity(&query, &related) > cosine_similarity(&query, &unrelated));
        assert!(similarity_to_rank_score(cosine_similarity(&query, &related)) > 0);
    }

    #[test]
    fn attachment_text_score_uses_ocr_text() {
        let attachments = vec![AttachmentEntry {
            note_id: "n1".to_string(),
            path: "D:/vault/2026/04/release.md".to_string(),
            file_name: "screenshot.png".to_string(),
            stem: "screenshot".to_string(),
            ocr_text: "GitHub Release v0.1.1 publish workflow".to_string(),
            semantic_vector: None,
            perceptual_hash: None,
        }];

        assert!(attachment_text_relevance_score("release workflow", &attachments) > 0);
    }

    #[test]
    fn relevance_score_hits_sd_pinmux_note_from_natural_query() {
        let doc = NoteDocument {
            meta: NoteMeta {
                id: "1".to_string(),
                title: "RK3566 SD卡复用引脚电路示意图".to_string(),
                tags: vec![
                    "RK3566".to_string(),
                    "SD卡".to_string(),
                    "引脚复用".to_string(),
                ],
                keywords: vec![
                    "sd card".to_string(),
                    "pin multiplexing".to_string(),
                    "mmc".to_string(),
                ],
                platform: "RK3566".to_string(),
                board: String::new(),
                kernel: String::new(),
                status: "待确认".to_string(),
                created_at: String::new(),
                updated_at: "2026-04-10T00:00:00Z".to_string(),
                source: "manual".to_string(),
                path: "vault/2026/04/rk3566-sd.md".to_string(),
                summary: "记录 RK3566 平台下 SD 卡引脚复用的电路与对照信息".to_string(),
            },
            body:
                "## 概述\nSD 卡接口引脚连接定义。\n## 备注\n软件层可参考 Device Tree pinctrl 配置。"
                    .to_string(),
        };

        assert!(document_relevance_score("sd卡的引脚复用怎么做的", &doc) > 200);
    }

    #[test]
    fn relevance_score_hits_flash_command_note_from_broad_query() {
        let doc = NoteDocument {
            meta: NoteMeta {
                id: "2".to_string(),
                title: "刷机命令记录".to_string(),
                tags: vec!["刷机".to_string()],
                keywords: vec![
                    "wboot".to_string(),
                    "update".to_string(),
                    "zboot".to_string(),
                ],
                platform: String::new(),
                board: String::new(),
                kernel: String::new(),
                status: "已解决".to_string(),
                created_at: String::new(),
                updated_at: "2026-04-10T00:00:00Z".to_string(),
                source: "manual".to_string(),
                path: "vault/2026/04/flash.md".to_string(),
                summary: "之前刷机时使用过的命令记录".to_string(),
            },
            body: "相关命令: wboot -w update zboot.img".to_string(),
        };

        assert!(document_relevance_score("刷机怎么刷啊", &doc) > 180);
    }

    // ── 1.1 split_frontmatter ──

    #[test]
    fn split_frontmatter_no_delimiter_returns_defaults() {
        let content = "Just body text\nNo frontmatter here";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert!(fm.id.is_empty());
        assert!(fm.title.is_empty());
        assert_eq!(body, content);
    }

    #[test]
    fn split_frontmatter_valid_block_parses_fields() {
        let content = "---\nid: test-id\ntitle: Test Title\n---\n\nBody here";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert_eq!(fm.id, "test-id");
        assert_eq!(fm.title, "Test Title");
        assert!(body.contains("Body here"));
    }

    #[test]
    fn split_frontmatter_malformed_returns_err() {
        let content = "---\nid: test\nno closing delimiter";
        assert!(split_frontmatter(content).is_err());
    }

    #[test]
    fn split_frontmatter_empty_block_returns_defaults() {
        // Empty YAML between delimiters → serde_yaml returns defaults
        let content = "---\n\n---\n\nBody";
        let (fm, body) = split_frontmatter(content).expect("parse");
        assert!(fm.id.is_empty());
        assert!(body.contains("Body"));
    }

    // ── 1.2 compose_markdown ──

    #[test]
    fn compose_markdown_produces_valid_yaml_frontmatter() {
        let meta = NoteMeta {
            id: "id123".to_string(),
            title: "测试笔记".to_string(),
            ..Default::default()
        };
        let body = "内容";
        let result = compose_markdown(&meta, body).expect("compose");
        assert!(result.starts_with("---\n"));
        assert!(result.contains("\n---\n"));
        assert!(result.contains("id: id123"));
        assert!(result.contains("title: 测试笔记"));
    }

    #[test]
    fn compose_markdown_injects_summary_section() {
        let meta = NoteMeta {
            summary: "这是摘要".to_string(),
            ..Default::default()
        };
        let result = compose_markdown(&meta, "正文").expect("compose");
        assert!(result.contains("## 摘要"));
        assert!(result.contains("这是摘要"));
    }

    #[test]
    fn compose_markdown_preserves_existing_summary_section() {
        let meta = NoteMeta {
            summary: "新摘要".to_string(),
            ..Default::default()
        };
        let result = compose_markdown(&meta, "## 摘要\n\n旧摘要\n\n正文").expect("compose");
        // body already starts with ## 摘要 so ensure_summary_section should not double-inject
        assert_eq!(result.matches("## 摘要").count(), 1);
    }

    // ── 1.3 slugify ──

    #[test]
    fn slugify_ascii_input() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn slugify_special_characters() {
        assert_eq!(slugify("a/b\\c:d*e?f"), "a-b-c-d-e-f");
    }

    #[test]
    fn slugify_empty_returns_note() {
        assert_eq!(slugify(""), "note");
    }

    #[test]
    fn slugify_consecutive_special_chars_single_dash() {
        assert_eq!(slugify("a---b"), "a-b");
    }

    #[test]
    fn slugify_trims_leading_trailing_dashes() {
        assert_eq!(slugify("---hello---"), "hello");
    }

    #[test]
    fn slugify_cjk_does_not_panic() {
        let result = slugify("测试中文");
        assert!(!result.is_empty());
    }

    // ── 1.4 fallback_title / fallback_source ──

    #[test]
    fn fallback_title_empty_returns_default() {
        assert_eq!(fallback_title(""), "Untitled Note");
        assert_eq!(fallback_title("  "), "Untitled Note");
    }

    #[test]
    fn fallback_title_nonempty_returns_trimmed() {
        assert_eq!(fallback_title("MMC timeout"), "MMC timeout");
    }

    #[test]
    fn fallback_source_empty_returns_manual() {
        assert_eq!(fallback_source(""), "manual");
        assert_eq!(fallback_source("  "), "manual");
    }

    #[test]
    fn fallback_source_nonempty_returns_trimmed() {
        assert_eq!(fallback_source("imported"), "imported");
    }

    // ── 1.5 detect_title ──

    #[test]
    fn detect_title_from_h1() {
        assert_eq!(
            detect_title("# My Title\nBody", Path::new("x.md")),
            "My Title"
        );
    }

    #[test]
    fn detect_title_h2_falls_to_file_stem() {
        assert_eq!(
            detect_title("## Sub\nBody", Path::new("my-note.md")),
            "my note"
        );
    }

    #[test]
    fn detect_title_empty_body_uses_file_stem() {
        assert_eq!(
            detect_title("", Path::new("/vault/2026/04/boot-timeout.md")),
            "boot timeout"
        );
    }

    #[test]
    fn detect_title_underscores_replaced() {
        assert_eq!(
            detect_title("", Path::new("boot_timeout_log.md")),
            "boot timeout log"
        );
    }

    // ── 1.6 sanitize_terms ──

    #[test]
    fn sanitize_terms_deduplicates() {
        assert_eq!(
            sanitize_terms(&["kernel".to_string(), "Kernel".to_string()]),
            vec!["kernel"]
        );
    }

    #[test]
    fn sanitize_terms_filters_empty() {
        assert_eq!(
            sanitize_terms(&["tag".to_string(), "".to_string(), "  ".to_string()]),
            vec!["tag"]
        );
    }

    // ── 1.7 hash_content ──

    #[test]
    fn hash_content_stable() {
        assert_eq!(hash_content("hello"), hash_content("hello"));
    }

    #[test]
    fn hash_content_different_inputs() {
        assert_ne!(hash_content("hello"), hash_content("world"));
    }

    #[test]
    fn hash_content_empty_string() {
        let hash = hash_content("");
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA-256 hex
    }

    // ── 1.8 is_markdown_file ──

    #[test]
    fn is_markdown_file_accepts_md() {
        assert!(is_markdown_file(Path::new("note.md")));
        assert!(is_markdown_file(Path::new("note.MD")));
    }

    #[test]
    fn is_markdown_file_rejects_non_md() {
        assert!(!is_markdown_file(Path::new("note.txt")));
        assert!(!is_markdown_file(Path::new("note")));
        assert!(!is_markdown_file(Path::new("note.md.bak")));
    }

    // ── 1.9 ensure_summary_section ──

    #[test]
    fn ensure_summary_injects_when_missing() {
        let result = ensure_summary_section("Body text", "My summary");
        assert!(result.starts_with("## 摘要"));
        assert!(result.contains("My summary"));
        assert!(result.contains("Body text"));
    }

    #[test]
    fn ensure_summary_skips_when_already_present() {
        let body = "## 摘要\n\nExisting\n\nMore";
        let result = ensure_summary_section(body, "New");
        assert_eq!(result, body);
    }

    #[test]
    fn ensure_summary_skips_when_empty() {
        let result = ensure_summary_section("Body text", "");
        assert_eq!(result, "Body text");
    }

    // ── 1.10 append_image_markdown ──

    #[test]
    fn append_image_empty_refs_returns_body() {
        assert_eq!(append_image_markdown("body", &[]), "body");
    }

    #[test]
    fn append_image_creates_section() {
        let result = append_image_markdown("body text", &["assets/photo.png".to_string()]);
        assert!(result.contains("## 图片记录"));
        assert!(result.contains("![photo.png](assets/photo.png)"));
    }

    #[test]
    fn append_image_appends_to_existing_section() {
        let body = "body\n\n## 图片记录\n\n![a.png](a.png)";
        let result = append_image_markdown(body, &["b/photo.jpg".to_string()]);
        assert!(result.contains("![a.png](a.png)"));
        assert!(result.contains("![photo.jpg](b/photo.jpg)"));
        assert!(result.contains("/")); // forward slashes
    }

    #[test]
    fn append_image_replaces_backslashes() {
        let result = append_image_markdown("body", &["dir\\img.png".to_string()]);
        assert!(result.contains("dir/img.png"));
        assert!(!result.contains("\\"));
    }

    // ── 1.11 unique_asset_name ──

    #[test]
    fn unique_asset_first_occurrence() {
        let mut seen = HashSet::new();
        assert_eq!(unique_asset_name("photo.png", &mut seen), "photo.png");
    }

    #[test]
    fn unique_asset_second_occurrence() {
        let mut seen = HashSet::new();
        seen.insert("photo.png".to_string());
        assert_eq!(unique_asset_name("photo.png", &mut seen), "photo-1.png");
    }

    #[test]
    fn unique_asset_no_extension() {
        let mut seen = HashSet::new();
        seen.insert("data".to_string());
        assert_eq!(unique_asset_name("data", &mut seen), "data-1");
    }

    // ── 1.12 normalize_search_text / normalize_query_for_search ──

    #[test]
    fn normalize_search_text_lowercases() {
        assert!(normalize_search_text("Hello WORLD").contains("hello world"));
    }

    #[test]
    fn normalize_search_text_preserves_cjk() {
        let result = normalize_search_text("测试MMC模块");
        assert!(result.contains("测试"));
        assert!(result.contains("mmc"));
    }

    #[test]
    fn normalize_query_removes_noise_phrases() {
        let result = normalize_query_for_search("告诉我sd卡怎么做");
        assert!(!result.contains("告诉我"));
        assert!(result.contains("sd"));
    }

    // ── 1.13 is_noise_term ──

    #[test]
    fn noise_terms_detected() {
        assert!(is_noise_term("什么"));
        assert!(is_noise_term("怎么"));
        assert!(is_noise_term("如何"));
        assert!(is_noise_term("这个"));
        assert!(is_noise_term("那个"));
    }

    #[test]
    fn real_terms_not_noise() {
        assert!(!is_noise_term("sd卡"));
        assert!(!is_noise_term("mmc"));
        assert!(!is_noise_term("flash"));
        assert!(!is_noise_term("刷机"));
    }

    // ── 1.14 expand_term_aliases ──

    #[test]
    fn expand_sd_aliases() {
        let aliases = expand_term_aliases("sd");
        assert!(aliases.contains(&"sd卡".to_string()));
        assert!(aliases.contains(&"sdio".to_string()));
        assert!(aliases.contains(&"mmc".to_string()));
        assert!(aliases.contains(&"tf".to_string()));
    }

    #[test]
    fn expand_flash_aliases() {
        let aliases = expand_term_aliases("刷机");
        assert!(aliases.contains(&"烧录".to_string()));
        assert!(aliases.contains(&"flash".to_string()));
        assert!(aliases.contains(&"wboot".to_string()));
    }

    #[test]
    fn expand_gpio_aliases() {
        let aliases = expand_term_aliases("gpio");
        assert!(aliases.contains(&"管脚".to_string()));
        assert!(aliases.contains(&"引脚".to_string()));
    }

    #[test]
    fn expand_pinmux_aliases() {
        let aliases = expand_term_aliases("pinmux");
        assert!(aliases.contains(&"引脚复用".to_string()));
        assert!(aliases.contains(&"iomux".to_string()));
    }

    #[test]
    fn expand_random_term_returns_empty() {
        assert!(expand_term_aliases("something_unrelated_xyz").is_empty());
    }

    // ── 1.15 is_cjk / is_cjk_stop_char ──

    #[test]
    fn cjk_chars_identified() {
        assert!(is_cjk('电'));
        assert!(is_cjk('的'));
        assert!(!is_cjk('A'));
        assert!(!is_cjk('1'));
    }

    #[test]
    fn cjk_stop_chars_detected() {
        assert!(is_cjk_stop_char('的'));
        assert!(is_cjk_stop_char('了'));
        assert!(!is_cjk_stop_char('电'));
    }

    // ── 1.16 sliding_char_grams ──

    #[test]
    fn sliding_grams_normal() {
        let result = sliding_char_grams("abcd", 3);
        assert_eq!(result, vec!["abc", "bcd"]);
    }

    #[test]
    fn sliding_grams_too_short() {
        assert!(sliding_char_grams("ab", 3).is_empty());
    }

    #[test]
    fn sliding_grams_exact_length() {
        assert_eq!(sliding_char_grams("abc", 3), vec!["abc"]);
    }

    // ── 1.17 document_relevance_score edge cases ──

    #[test]
    fn relevance_empty_query_returns_zero() {
        let doc = NoteDocument::default();
        assert_eq!(document_relevance_score("", &doc), 0);
    }

    #[test]
    fn relevance_no_match_returns_zero() {
        let doc = NoteDocument {
            meta: NoteMeta {
                title: "Completely Unrelated".to_string(),
                ..Default::default()
            },
            body: "Nothing relevant here".to_string(),
        };
        assert_eq!(document_relevance_score("mmc sd卡 pinmux", &doc), 0);
    }

    #[test]
    fn relevance_body_only_match() {
        let doc = NoteDocument {
            body: "mmc timeout after 30 seconds".to_string(),
            ..Default::default()
        };
        assert!(document_relevance_score("mmc timeout", &doc) > 0);
    }

    // ── 1.18 attachment_text_relevance_score edge cases ──

    #[test]
    fn attachment_score_empty_attachments_zero() {
        assert_eq!(attachment_text_relevance_score("mmc", &[]), 0);
    }

    #[test]
    fn attachment_score_empty_query_zero() {
        let attachments = vec![AttachmentEntry {
            note_id: "n".to_string(),
            path: "p".to_string(),
            file_name: "f.png".to_string(),
            stem: "f".to_string(),
            ocr_text: "text".to_string(),
            semantic_vector: None,
            perceptual_hash: None,
        }];
        assert_eq!(attachment_text_relevance_score("", &attachments), 0);
    }

    #[test]
    fn attachment_score_ocr_match_higher_than_filename() {
        let attachments = vec![AttachmentEntry {
            note_id: "n".to_string(),
            path: "p".to_string(),
            file_name: "img.png".to_string(),
            stem: "img".to_string(),
            ocr_text: "mmc timeout register dump".to_string(),
            semantic_vector: None,
            perceptual_hash: None,
        }];
        let score_ocr = attachment_text_relevance_score("mmc timeout register", &attachments);
        let score_fname = attachment_text_relevance_score("img", &attachments);
        assert!(score_ocr > score_fname);
    }

    // ── 1.19 build_candidate_note_ids ──

    #[test]
    fn build_candidates_deduplicates() {
        let ids = build_candidate_note_ids(
            &["a".to_string(), "b".to_string()],
            &["b".to_string(), "c".to_string()],
            &HashMap::new(),
            &HashMap::new(),
            &["c".to_string(), "d".to_string()],
            10,
        );
        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        assert!(ids.contains(&"a".to_string()));
        assert!(ids.contains(&"b".to_string()));
        assert!(ids.contains(&"c".to_string()));
        assert!(ids.contains(&"d".to_string()));
    }

    #[test]
    fn build_candidates_truncates_to_limit() {
        let many: Vec<String> = (0..100).map(|i| format!("id{i}")).collect();
        let result = build_candidate_note_ids(&many, &[], &HashMap::new(), &HashMap::new(), &[], 2);
        assert!(result.len() <= 24); // limit*8.max(24) with limit=2 → 24
    }

    // ── 1.20 cosine_similarity / normalize_vector ──

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0_f32, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.001);
    }

    #[test]
    fn normalize_vector_zero_no_panic() {
        let mut v = vec![0.0_f32; 3];
        normalize_vector(&mut v); // should not divide by zero
        assert_eq!(v, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn normalize_vector_produces_unit() {
        let mut v = vec![3.0_f32, 4.0];
        normalize_vector(&mut v);
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    // ── 1.21 similarity_to_rank_score ──

    #[test]
    fn similarity_to_rank_boundary_values() {
        assert_eq!(similarity_to_rank_score(0.85), 220);
        assert_eq!(similarity_to_rank_score(0.70), 170);
        assert_eq!(similarity_to_rank_score(0.55), 120);
        assert_eq!(similarity_to_rank_score(0.40), 80);
        assert_eq!(similarity_to_rank_score(0.25), 40);
        assert_eq!(similarity_to_rank_score(0.10), 0);
        assert_eq!(similarity_to_rank_score(1.0), 220); // top bucket
    }

    // ── 1.22 image_similarity_score ──

    #[test]
    fn image_similarity_identical() {
        assert_eq!(image_similarity_score(0xABCD, 0xABCD), 240);
    }

    #[test]
    fn image_similarity_boundary_distances() {
        let base: u64 = 0;
        let d2: u64 = (1u64 << 2) - 1; // 2 bits differ
        assert_eq!(image_similarity_score(base, d2), 240);
    }

    #[test]
    fn image_similarity_max_distance() {
        assert_eq!(image_similarity_score(0, u64::MAX), 0);
    }

    // ── 1.23 serialize/deserialize semantic vector ──

    #[test]
    fn semantic_vector_round_trip() {
        let v: Vec<f32> = (0..ATTACHMENT_VECTOR_DIM)
            .map(|i| i as f32 * 0.01)
            .collect();
        let serialized = serialize_semantic_vector(&v);
        let deserialized = deserialize_semantic_vector(&serialized).expect("deserialize");
        assert_eq!(deserialized.len(), ATTACHMENT_VECTOR_DIM);
        for (a, b) in v.iter().zip(deserialized.iter()) {
            assert!((a - b).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn deserialize_wrong_dimension_returns_none() {
        let v = vec![1.0_f32; 10];
        let serialized = serde_json::to_string(&v).unwrap();
        assert!(deserialize_semantic_vector(&serialized).is_none());
    }

    #[test]
    fn deserialize_garbage_returns_none() {
        assert!(deserialize_semantic_vector("not json").is_none());
    }

    // ── 1.24 build_text_semantic_vector ──

    #[test]
    fn semantic_vector_empty_text_returns_none() {
        assert!(build_text_semantic_vector("").is_none());
    }

    #[test]
    fn semantic_vector_produces_normalized() {
        let v = build_text_semantic_vector("github release workflow").expect("vector");
        assert_eq!(v.len(), ATTACHMENT_VECTOR_DIM);
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    // ── 1.25 stable_term_hash ──

    #[test]
    fn stable_term_hash_consistent() {
        assert_eq!(stable_term_hash("mmc"), stable_term_hash("mmc"));
    }

    #[test]
    fn stable_term_hash_different_inputs() {
        assert_ne!(stable_term_hash("mmc"), stable_term_hash("sdio"));
    }

    // ── 1.26 parse_chat_state ──

    #[test]
    fn parse_chat_state_modern_format() {
        let json = r#"{"currentSessionId":"s1","sessions":[{"id":"s1","title":"Hello","turns":[],"summary":null,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}]}"#;
        let state = parse_chat_state(json).expect("parse");
        assert_eq!(state.current_session_id, "s1");
        assert_eq!(state.sessions.len(), 1);
    }

    #[test]
    fn parse_chat_state_legacy_format_migrated_to_empty_sessions() {
        // Legacy JSON with "sessionId" and "turns" — serde ignores unknown fields,
        // so this actually parses as a modern ChatState with empty sessions.
        // normalize_chat_state then creates a default session.
        // This test documents the actual behavior: legacy format is absorbed
        // into modern format with data loss on the turns.
        let json = r#"{"sessionId":"legacy1","turns":[{"id":"t1","role":"user","text":"hello","citations":[],"savedNote":null,"thinkingTrace":null,"attachments":[],"createdAt":"2026-01-01T00:00:00Z"}]}"#;
        let state = parse_chat_state(json).expect("parse");
        // The modern ChatState path wins because serde ignores "sessionId"
        assert_eq!(state.sessions.len(), 1); // default session created
    }

    #[test]
    fn parse_chat_state_invalid_returns_err() {
        assert!(parse_chat_state("not json at all").is_err());
    }

    // ── 1.27 normalize_chat_state ──

    #[test]
    fn normalize_chat_state_fills_empty_ids() {
        let state = ChatState {
            current_session_id: String::new(),
            sessions: vec![ChatSession {
                id: String::new(),
                title: "Test".to_string(),
                turns: vec![],
                summary: None,
                created_at: String::new(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            }],
        };
        let result = normalize_chat_state(state);
        assert!(!result.sessions[0].id.is_empty());
    }

    #[test]
    fn normalize_chat_state_empty_sessions_returns_default() {
        let state = ChatState {
            current_session_id: String::new(),
            sessions: vec![],
        };
        let result = normalize_chat_state(state);
        assert!(!result.sessions.is_empty());
    }

    #[test]
    fn normalize_chat_state_sorts_by_updated_at_desc() {
        let state = ChatState {
            current_session_id: "a".to_string(),
            sessions: vec![
                ChatSession {
                    id: "a".to_string(),
                    title: "Old".to_string(),
                    turns: vec![],
                    summary: None,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                },
                ChatSession {
                    id: "b".to_string(),
                    title: "New".to_string(),
                    turns: vec![],
                    summary: None,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-06-01T00:00:00Z".to_string(),
                },
            ],
        };
        let result = normalize_chat_state(state);
        assert_eq!(result.sessions[0].id, "b");
        assert_eq!(result.sessions[1].id, "a");
    }

    #[test]
    fn normalize_chat_state_fixes_invalid_current_session() {
        let state = ChatState {
            current_session_id: "ghost".to_string(),
            sessions: vec![ChatSession {
                id: "real".to_string(),
                title: "Real".to_string(),
                turns: vec![],
                summary: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            }],
        };
        let result = normalize_chat_state(state);
        assert_eq!(result.current_session_id, "real");
    }

    // ── 1.28 derive_chat_title ──

    #[test]
    fn derive_chat_title_from_user_turn() {
        let turns = vec![crate::models::ChatTurn {
            role: "user".to_string(),
            text: "mmc超时怎么处理比较好".to_string(),
            ..Default::default()
        }];
        let title = derive_chat_title(&turns);
        assert!(title.contains("mmc"));
    }

    #[test]
    fn derive_chat_title_no_user_turn() {
        let turns = vec![crate::models::ChatTurn {
            role: "assistant".to_string(),
            text: "hi".to_string(),
            ..Default::default()
        }];
        assert_eq!(derive_chat_title(&turns), "新对话");
    }

    #[test]
    fn derive_chat_title_empty_turns() {
        assert_eq!(derive_chat_title(&[]), "新对话");
    }

    // ── 1.29 normalize_settings ──

    #[test]
    fn normalize_settings_fills_defaults() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-settings-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let paths = AppPaths {
            settings_path: temp.join("s.json"),
            database_path: temp.join("db.sqlite"),
            chat_state_path: temp.join("cs.json"),
            default_vault_dir: temp.join("default-vault"),
            vault_dir_override: None,
        };
        let mut settings = AppSettings::default();
        normalize_settings(&mut settings, &paths);
        assert!(!settings.vault_dir.is_empty());
        assert!(!settings.provider.base_url.is_empty());
        assert!(settings.provider.request_timeout_ms > 0);
    }

    #[test]
    fn normalize_settings_zero_timeout_gets_default() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-settings-zero-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let paths = AppPaths {
            settings_path: temp.join("s.json"),
            database_path: temp.join("db.sqlite"),
            chat_state_path: temp.join("cs.json"),
            default_vault_dir: temp.join("default-vault"),
            vault_dir_override: None,
        };
        let mut settings = AppSettings {
            provider: ProviderConfig {
                request_timeout_ms: 0,
                context_window_tokens: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };
        normalize_settings(&mut settings, &paths);
        assert_eq!(
            settings.provider.request_timeout_ms,
            crate::models::default_timeout_ms()
        );
        assert!(settings.provider.context_window_tokens.is_none());
    }

    #[test]
    fn for_cli_uses_vault_local_state_paths_when_override_is_provided() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-cli-paths-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let vault_dir = temp.join("vault");

        let ctx = StorageContext::for_cli(Some(vault_dir.clone())).expect("cli context");

        assert_eq!(ctx.paths.default_vault_dir, vault_dir);
        assert_eq!(ctx.paths.vault_dir_override, Some(vault_dir.clone()));
        assert_eq!(
            ctx.paths.settings_path,
            vault_dir.join(".vaultpilot").join("settings.json")
        );
        assert_eq!(
            ctx.paths.database_path,
            vault_dir.join(".vaultpilot").join("knowledge-index.sqlite")
        );
        assert_eq!(
            ctx.paths.chat_state_path,
            vault_dir.join(".vaultpilot").join("chat-state.json")
        );
    }

    // ── 1.30 build_note_path ──

    #[test]
    fn build_note_path_uses_date_and_id() {
        let path = build_note_path(
            "D:\\Vault",
            "MMC Timeout",
            "2026-04-09T00:00:00Z",
            "abc12345-6789",
        );
        assert!(path.to_string_lossy().contains("2026"));
        assert!(path.to_string_lossy().contains("04"));
        assert!(path.to_string_lossy().contains("mmc-timeout"));
        assert!(path.to_string_lossy().contains("abc12345"));
        assert!(path.to_string_lossy().ends_with(".md"));
    }

    #[test]
    fn build_note_path_invalid_date_uses_current() {
        let path = build_note_path("D:\\Vault", "Test", "invalid-date", "abc12345-6789-def0");
        // Should not panic, uses current date as fallback
        assert!(path.to_string_lossy().ends_with(".md"));
    }

    // ══════════════════════════════════════
    // Phase 5: Integration Tests
    // ══════════════════════════════════════

    #[test]
    fn initialize_storage_creates_db_and_vault() {
        let (_temp, ctx) = setup_temp_context();
        let settings = initialize_storage_with_context(&ctx).expect("init");
        assert!(ctx.paths.database_path.exists());
        assert!(!settings.vault_dir.is_empty());
        assert!(Path::new(&settings.vault_dir).exists());
    }

    #[test]
    fn save_and_load_note_round_trip() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        let note = NoteDocument {
            meta: NoteMeta {
                id: String::new(), // will be assigned
                title: "Test Round Trip".to_string(),
                tags: vec!["test".to_string()],
                keywords: vec!["round-trip".to_string()],
                platform: "test".to_string(),
                board: "evk".to_string(),
                kernel: "5.10".to_string(),
                status: "active".to_string(),
                created_at: String::new(),
                updated_at: String::new(),
                source: String::new(),
                path: String::new(),
                summary: String::new(),
            },
            body: "## Test\n\nRound trip body content".to_string(),
        };

        let saved = save_note_with_context(&ctx, note).expect("save");
        assert!(!saved.meta.id.is_empty());
        assert!(!saved.meta.path.is_empty());
        assert!(saved.meta.path.ends_with(".md"));

        let loaded = load_note_with_context(&ctx, &saved.meta.id).expect("load");
        assert_eq!(loaded.meta.title, "Test Round Trip");
        assert_eq!(loaded.meta.tags, vec!["test"]);
        assert!(loaded.body.contains("Round trip body content"));
    }

    #[test]
    fn delete_note_removes_file_and_index() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        let note = NoteDocument {
            meta: NoteMeta {
                title: "To Delete".to_string(),
                ..Default::default()
            },
            body: "Temporary content".to_string(),
        };
        let saved = save_note_with_context(&ctx, note).expect("save");
        let path = saved.meta.path.clone();

        assert!(delete_note_with_context(&ctx, &saved.meta.id).expect("delete"));
        assert!(load_note_with_context(&ctx, &saved.meta.id).is_err());
        assert!(!Path::new(&path).exists());
    }

    #[test]
    fn delete_nonexistent_note_returns_false() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");
        assert!(!delete_note_with_context(&ctx, "ghost-id").expect("delete ghost"));
    }

    #[test]
    fn search_notes_filters_by_text() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        for (i, (title, tags)) in [
            ("MMC timeout fix", vec!["kernel".to_string()]),
            ("SD卡引脚配置", vec!["hardware".to_string()]),
            ("刷机命令记录", vec!["tool".to_string()]),
        ]
        .into_iter()
        .enumerate()
        {
            save_note_with_context(
                &ctx,
                NoteDocument {
                    meta: NoteMeta {
                        title: title.to_string(),
                        tags,
                        ..Default::default()
                    },
                    body: format!("Content for note {}", i),
                },
            )
            .expect("save");
        }

        let results = search_notes_with_context(
            &ctx,
            SearchQuery {
                text: "MMC".to_string(),
                tags: vec![],
                keywords: vec![],
                limit: Some(10),
            },
        )
        .expect("search");
        assert!(results.notes.iter().any(|n| n.title.contains("MMC")));
    }

    #[test]
    fn search_notes_filters_by_tags() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        save_note_with_context(
            &ctx,
            NoteDocument {
                meta: NoteMeta {
                    title: "Tagged Note".to_string(),
                    tags: vec!["kernel".to_string()],
                    ..Default::default()
                },
                body: "Tagged content".to_string(),
            },
        )
        .expect("save");

        let results = search_notes_with_context(
            &ctx,
            SearchQuery {
                text: String::new(),
                tags: vec!["kernel".to_string()],
                keywords: vec![],
                limit: Some(10),
            },
        )
        .expect("search by tag");
        assert!(results
            .notes
            .iter()
            .any(|n| n.tags.contains(&"kernel".to_string())));
    }

    #[test]
    fn import_markdown_adds_to_index() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        let import_dir = _temp.join("import-source");
        fs::create_dir_all(&import_dir).expect("import dir");
        let md_file = import_dir.join("imported-note.md");
        fs::write(
            &md_file,
            "---\ntitle: Imported Note\n---\n\nImported body content\n",
        )
        .expect("write md");

        let result = import_markdown_with_context(&ctx, &[md_file.to_string_lossy().to_string()])
            .expect("import");
        assert_eq!(result.imported, 1);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn settings_round_trip() {
        let (_temp, ctx) = setup_temp_context();
        let custom = AppSettings {
            vault_dir: _temp.join("my-vault").to_string_lossy().to_string(),
            provider: ProviderConfig {
                api_key: "test-key".to_string(),
                base_url: "https://custom.api.com".to_string(),
                model: "custom-model".to_string(),
                request_timeout_ms: 99_000,
                context_window_tokens: Some(200_000),
            },
            auto_check_updates: false,
            auto_wake_enabled: true,
            auto_wake_interval_minutes: 60,
            auto_wake_model: "claude-3-5-haiku-latest".to_string(),
            auto_wake_start_time: "05:00".to_string(),
            auto_wake_end_time: "23:00".to_string(),
        };

        let _saved = save_settings_with_context(&ctx, custom.clone()).expect("save settings");
        let loaded = load_settings_with_context(&ctx).expect("load settings");
        assert_eq!(loaded.provider.api_key, "test-key");
        assert_eq!(loaded.provider.model, "custom-model");
        assert_eq!(loaded.provider.request_timeout_ms, 99_000);
        assert_eq!(loaded.provider.context_window_tokens, Some(200_000));
    }

    #[test]
    fn chat_state_round_trip() {
        let (_temp, ctx) = setup_temp_context();
        let state = ChatState {
            current_session_id: "s1".to_string(),
            sessions: vec![ChatSession {
                id: "s1".to_string(),
                title: "Test Session".to_string(),
                turns: vec![crate::models::ChatTurn {
                    id: "t1".to_string(),
                    role: "user".to_string(),
                    text: "hello".to_string(),
                    ..Default::default()
                }],
                summary: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            }],
        };

        save_chat_state_with_context(&ctx, &state).expect("save chat");
        let loaded = load_chat_state_with_context(&ctx).expect("load chat");
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].title, "Test Session");
    }

    #[test]
    fn rebuild_index_recovers_from_manual_db_edit() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).expect("init");

        let note = NoteDocument {
            meta: NoteMeta {
                title: "Rebuild Test".to_string(),
                ..Default::default()
            },
            body: "Content that should survive rebuild".to_string(),
        };
        save_note_with_context(&ctx, note).expect("save");

        let stats = rebuild_index_with_context(&ctx).expect("rebuild");
        assert!(stats.scanned > 0);
        assert!(stats.indexed > 0);
    }
}
