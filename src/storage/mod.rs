use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context, Result};
use image::{imageops::FilterType, ImageReader};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use crate::models::{AppSettings, ChatState, IndexStats, NoteDocument, NoteMeta, SearchQuery, SearchResult};

mod backup;
mod chat;
pub(crate) mod notes;
mod search;
mod settings;

// Re-export chat session public API so callers see no difference.
pub use chat::{load_chat_state_with_context, save_chat_state_with_context};
// Re-export settings public API so callers see no difference.
pub use settings::{load_settings_with_context, save_settings_with_context};
// Re-export search public API so callers see no difference.
pub use search::search_notes_with_context;
// Re-export notes public API so callers see no difference.
pub use notes::{
    delete_note_async, delete_note_with_context, export_all_notes_async,
    export_all_notes_with_context, export_note_markdown_async, export_note_markdown_with_context,
    find_related_notes_async, find_related_notes_with_context, import_markdown_async,
    import_markdown_with_context, load_context_notes_async, load_context_notes_with_context,
    load_note_with_context, load_recent_notes_for_overview, load_recent_notes_for_overview_async,
    ocr_image_text, ocr_image_text_async, rebuild_index_async, rebuild_index_with_context,
    save_note_async, save_note_with_context, save_note_with_images_async,
    save_note_with_images_with_context, search_candidate_notes_async,
    search_candidate_notes_with_context, vault_export_async, vault_export_with_context,
};

// Internal imports from search module (used by remaining functions in this file)

/// Type alias for a pooled SQLite connection.
type PooledConnection = r2d2::PooledConnection<SqliteConnectionManager>;

/// Write `data` to `path` atomically by writing to a temporary file first, then
/// renaming.  On the same filesystem `rename` is guaranteed to be atomic, so a
/// crash mid-write will never leave a truncated/corrupt file behind.
///
/// Uses a random UUID suffix for the temp file to prevent concurrent writers
/// from racing on the same deterministic temp filename.
pub(super) fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;

    let tmp_name = format!(
        "{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp"),
        Uuid::new_v4()
    );
    let tmp_path = path.with_file_name(tmp_name);
    // Create the temp file, then restrict permissions *before* writing any
    // sensitive data so that other users can never read the contents, even
    // in the brief window between file creation and rename (issue #186).
    //
    // We keep the file handle open and write through it to avoid a TOCTOU
    // race between dropping the handle and re-opening via fs::write (#475).
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        file.set_permissions(perms)?;
    }
    file.write_all(data).inspect_err(|_| {
        // Clean up the temp file on write failure to prevent disk accumulation
        let _ = fs::remove_file(&tmp_path);
    })?;
    file.sync_all().inspect_err(|_| {
        let _ = fs::remove_file(&tmp_path);
    })?;
    drop(file);
    fs::rename(&tmp_path, path).inspect_err(|_| {
        // #850: Clean up temp file on rename failure (cross-device move, permissions, disk full)
        let _ = fs::remove_file(&tmp_path);
    })?;
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
    /// Connection pool for SQLite database access.
    pool: Pool<SqliteConnectionManager>,
    /// Cached parsed AppSettings, shared across clones of the same context.
    cached_settings: Arc<Mutex<Option<AppSettings>>>,
}

impl StorageContext {
    fn with_pool(paths: AppPaths) -> Result<Self> {
        let db_path = paths.database_path.clone();
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Auto-backup SQLite database before opening (keep last 3 backups)
        backup::auto_backup_database(&db_path).unwrap_or_else(|e| {
            tracing::warn!("SQLite auto-backup failed: {e}");
        });

        let manager = SqliteConnectionManager::file(&db_path).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;",
            )
        });
        let pool = Pool::builder()
            .max_size(5)
            .build(manager)
            .with_context(|| "failed to create SQLite connection pool")?;
        Ok(Self {
            paths,
            pool,
            cached_settings: Arc::new(Mutex::new(None)),
        })
    }
}

/// Maximum note file size (10 MiB) — prevents OOM from oversized markdown files
/// during import or vault rebuild (#827).
const MAX_NOTE_FILE_SIZE: u64 = 10 * 1024 * 1024;

impl StorageContext {
    pub fn for_sidecar() -> Result<Self> {
        let config_root = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| {
                tracing::warn!("APPDATA/HOME unset, falling back to temp dir for config_root");
                std::env::temp_dir()
            })
            .join("com.local.vaultpilot");
        let data_root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| config_root.clone())
            .join("com.local.vaultpilot");
        let default_vault_dir = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| {
                tracing::warn!(
                    "USERPROFILE/HOME unset, falling back to temp dir for default_vault_dir"
                );
                std::env::temp_dir()
            })
            .join("Documents")
            .join("VaultPilotVault");

        Self::with_pool(AppPaths {
            settings_path: config_root.join("settings.json"),
            database_path: data_root.join("knowledge-index.sqlite"),
            chat_state_path: data_root.join("chat-state.json"),
            default_vault_dir,
            vault_dir_override: None,
        })
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
            // Rebuild the pool for the new database path
            ctx = Self::with_pool(ctx.paths)?;
        } else {
            ctx.paths.vault_dir_override = None;
        }
        Ok(ctx)
    }

    #[cfg(test)]
    pub(crate) fn for_test(temp: &Path) -> Self {
        Self::with_pool(AppPaths {
            settings_path: temp.join("settings.json"),
            database_path: temp.join("knowledge-index.sqlite"),
            chat_state_path: temp.join("chat-state.json"),
            default_vault_dir: temp.join("vault"),
            vault_dir_override: None,
        })
        .expect("failed to create test connection pool")
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

#[instrument(skip(context))]
pub fn initialize_storage_with_context(context: &StorageContext) -> Result<AppSettings> {
    let settings = settings::load_settings_with_context(context)?;
    // Obtain a connection from the pool (pool handles PRAGMAs via with_init).
    let connection = context
        .pool
        .get()
        .with_context(|| "failed to get connection from pool")?;
    ensure_schema(&connection)?;
    fs::create_dir_all(&settings.vault_dir)?;
    Ok(settings)
}

pub fn list_notes_with_context(context: &StorageContext) -> Result<Vec<NoteMeta>> {
    let result = search_notes_with_context(
        context,
        SearchQuery {
            text: String::new(),
            tags: Vec::new(),
            keywords: Vec::new(),
            limit: Some(50),
            ..Default::default()
        },
    )?;
    Ok(result.notes)
}

/// Returns `true` if the notes table contains at least one row.
///
/// This is much cheaper than [`list_notes_with_context`] which loads full
/// metadata — use this when you only need to know whether any notes exist.
#[instrument(skip(context))]
pub fn has_notes_with_context(context: &StorageContext) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let exists: bool =
        connection.query_row("SELECT EXISTS(SELECT 1 FROM notes LIMIT 1)", [], |row| {
            row.get(0)
        })?;
    Ok(exists)
}

pub fn load_note_body_from_meta(meta: &NoteMeta) -> Result<NoteDocument> {
    let path = Path::new(&meta.path);
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.len() > MAX_NOTE_FILE_SIZE {
        return Err(anyhow!(
            "note file too large ({} bytes, limit {} bytes): {}",
            metadata.len(),
            MAX_NOTE_FILE_SIZE,
            path.display()
        ));
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let normalized = raw.replace("\r\n", "\n");
    let (_, body) = notes::split_frontmatter(&normalized)?;
    Ok(NoteDocument {
        meta: meta.clone(),
        body: body.to_string(),
        search_snippet: None,
    })
}


/// Get a database connection from the connection pool.
/// Returns a `PooledConnection` that is automatically returned to the pool on drop.
pub(super) fn open_connection(context: &StorageContext) -> Result<(PooledConnection, AppSettings)> {
    let settings = load_settings_with_context(context)?;
    let conn = context
        .pool
        .get()
        .with_context(|| "failed to get connection from pool")?;
    Ok((conn, settings))
}

fn ensure_schema(connection: &Connection) -> Result<()> {
    // Fast path: skip schema creation if already initialized in this process.
    // PRAGMA user_version is a lightweight integer stored in the database header.
    let version: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version >= 1 {
        // Schema already exists; enable foreign keys, WAL mode, and busy timeout.
        connection.execute_batch(
            "PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;",
        )?;
        return Ok(());
    }

    connection.execute_batch(
        r#"
        PRAGMA busy_timeout = 5000;
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
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

        -- Indexes for ORDER BY updated_at DESC and date-range filters (#828)
        CREATE INDEX IF NOT EXISTS idx_notes_updated_at ON notes(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_notes_created_at ON notes(created_at DESC);

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

pub(super) fn compute_image_perceptual_hash(path: &Path) -> Option<u64> {
    // Guard: skip files larger than 50 MB to prevent OOM from crafted images (#718)
    const MAX_IMAGE_FILE_SIZE: u64 = 50 * 1024 * 1024;
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_IMAGE_FILE_SIZE {
        tracing::warn!(
            path = %path.display(),
            size = meta.len(),
            "image too large for perceptual hash, skipping"
        );
        return None;
    }

    // Guard: check decoded dimensions before full decode to prevent OOM from
    // gigapixel images with small compressed size (#718)
    const MAX_PIXELS: u64 = 4096 * 4096; // ~16 megapixels
    let (w, h) = image::image_dimensions(path).ok()?;
    if (w as u64).saturating_mul(h as u64) > MAX_PIXELS {
        tracing::warn!(
            path = %path.display(),
            width = w,
            height = h,
            "image dimensions too large for perceptual hash, skipping"
        );
        return None;
    }
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

// ---------------------------------------------------------------------------
// Async wrappers – spawn_blocking for all public sync storage functions
// These prevent synchronous SQLite / file I/O from blocking the Tokio runtime.
// ---------------------------------------------------------------------------

/// Spawn-blocking wrapper for [`initialize_storage_with_context`].
pub async fn initialize_storage_async(ctx: &StorageContext) -> Result<AppSettings> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || initialize_storage_with_context(&ctx))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`load_settings_with_context`].
pub async fn load_settings_async(ctx: &StorageContext) -> Result<AppSettings> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || load_settings_with_context(&ctx))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`save_settings_with_context`].
pub async fn save_settings_async(
    ctx: &StorageContext,
    settings: AppSettings,
) -> Result<AppSettings> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || save_settings_with_context(&ctx, settings))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`load_chat_state_with_context`].
pub async fn load_chat_state_async(ctx: &StorageContext) -> Result<ChatState> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || load_chat_state_with_context(&ctx))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`save_chat_state_with_context`].
pub async fn save_chat_state_async(ctx: &StorageContext, state: &ChatState) -> Result<ChatState> {
    let ctx = ctx.clone();
    let state = state.clone();
    tokio::task::spawn_blocking(move || save_chat_state_with_context(&ctx, &state))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`list_notes_with_context`].
pub async fn list_notes_async(ctx: &StorageContext) -> Result<Vec<NoteMeta>> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || list_notes_with_context(&ctx))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`has_notes_with_context`].
pub async fn has_notes_async(ctx: &StorageContext) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || has_notes_with_context(&ctx))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

/// Spawn-blocking wrapper for [`search_notes_with_context`].
pub async fn search_notes_async(ctx: &StorageContext, query: SearchQuery) -> Result<SearchResult> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || search_notes_with_context(&ctx, query))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}
pub async fn load_note_async(ctx: &StorageContext, note_id: &str) -> Result<NoteDocument> {
    let ctx = ctx.clone();
    let note_id = note_id.to_owned();
    tokio::task::spawn_blocking(move || load_note_with_context(&ctx, &note_id))
        .await
        .map_err(|e| anyhow!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::search::{image_similarity_score, derived_note_id, slugify, fallback_title, fallback_source, sanitize_terms, hash_content, is_markdown_file};
    use super::settings::normalize_settings;
    use super::*;
    use crate::models::{ChatSession, ProviderConfig};
    use chrono::Utc;

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
    fn settings_parser_tolerates_utf8_bom() {
        let raw = "\u{feff}{\"vaultDir\":\"D:\\\\Vault\",\"provider\":{\"apiKey\":\"k\",\"baseUrl\":\"u\",\"model\":\"m\",\"requestTimeoutMs\":1}}";
        let normalized = raw.trim_start_matches('\u{feff}');
        let parsed: AppSettings =
            serde_json::from_str(normalized).expect("parse settings with bom");
        assert_eq!(parsed.vault_dir, "D:\\Vault");
        assert_eq!(parsed.provider.model, "m");
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
        let result = slugify("");
        assert!(
            result.starts_with("note-"),
            "empty input should produce note-<hash>, got: {result}"
        );
        assert!(
            result.len() > "note-".len(),
            "hash suffix should be present"
        );
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
    fn slugify_cjk_transliterated() {
        // Common CJK characters transliterate to pinyin via deunicode
        let result = slugify("测试中文");
        assert_eq!(result, "ce-shi-zhong-wen");
    }

    #[test]
    fn slugify_cjk_fallback_with_hash() {
        // CJK punctuation that deunicode cannot transliterate to alphanumeric
        // should produce a distinguishable "note-<hash>" slug instead of bare "note".
        let result = slugify("\u{3001}\u{3002}\u{300C}\u{300D}");
        assert!(
            result.starts_with("note-"),
            "non-transliterable input should produce note-<hash>, got: {result}"
        );
        assert!(result.len() > "note-".len());
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
    fn normalize_settings_vault_dir_override_wins() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-settings-override-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp).expect("temp dir");
        let override_dir = temp.join("my-vault");
        let paths = AppPaths {
            settings_path: temp.join("s.json"),
            database_path: temp.join("db.sqlite"),
            chat_state_path: temp.join("cs.json"),
            default_vault_dir: temp.join("default-vault"),
            vault_dir_override: Some(override_dir.clone()),
        };
        let mut settings = AppSettings {
            vault_dir: "/old/path".into(),
            ..Default::default()
        };
        normalize_settings(&mut settings, &paths);
        assert_eq!(settings.vault_dir, override_dir.to_string_lossy());
    }

    #[test]
    fn normalize_settings_clamps_active_provider_index() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-settings-clamp-{}",
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
            providers: vec![ProviderConfig::default()],
            active_provider_index: 5,
            ..Default::default()
        };
        normalize_settings(&mut settings, &paths);
        assert_eq!(settings.active_provider_index, 0);
    }

    #[test]
    fn normalize_settings_multi_provider_gets_defaults() {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-settings-multi-{}",
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
            providers: vec![
                ProviderConfig {
                    base_url: "".into(),
                    model: "".into(),
                    request_timeout_ms: 0,
                    context_window_tokens: Some(0),
                    ..Default::default()
                },
                ProviderConfig {
                    base_url: "https://custom.api".into(),
                    model: "custom-model".into(),
                    request_timeout_ms: 30_000,
                    context_window_tokens: Some(4096),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        normalize_settings(&mut settings, &paths);
        // First provider: empty fields get defaults
        assert_eq!(
            settings.providers[0].base_url,
            crate::models::default_base_url()
        );
        assert_eq!(settings.providers[0].model, crate::models::default_model());
        assert_eq!(
            settings.providers[0].request_timeout_ms,
            crate::models::default_timeout_ms()
        );
        assert!(settings.providers[0].context_window_tokens.is_none());
        // Second provider: non-empty fields preserved
        assert_eq!(settings.providers[1].base_url, "https://custom.api");
        assert_eq!(settings.providers[1].model, "custom-model");
        assert_eq!(settings.providers[1].request_timeout_ms, 30_000);
        assert_eq!(settings.providers[1].context_window_tokens, Some(4096));
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
            search_snippet: None,
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
            search_snippet: None,
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
                name: "test".to_string(),
                api_key: "test-key".to_string(),
                base_url: "https://custom.api.com".to_string(),
                model: "custom-model".to_string(),
                request_timeout_ms: 99_000,
                context_window_tokens: Some(200_000),
                max_output_tokens: None,
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
        };

        let _saved = save_settings_with_context(&ctx, custom.clone()).expect("save settings");
        let loaded = load_settings_with_context(&ctx).expect("load settings");
        assert_eq!(loaded.provider.api_key, "test-key");
        assert_eq!(loaded.provider.model, "custom-model");
        assert_eq!(loaded.provider.request_timeout_ms, 99_000);
        assert_eq!(loaded.provider.context_window_tokens, Some(200_000));
    }

    #[test]
    fn settings_api_key_encrypted_on_disk() {
        let (_temp, ctx) = setup_temp_context();
        let custom = AppSettings {
            vault_dir: _temp.join("vault-enc").to_string_lossy().to_string(),
            provider: ProviderConfig {
                name: "test".to_string(),
                api_key: "sk-sec...2345".to_string(),
                base_url: "https://custom.api.com".to_string(),
                model: "custom-model".to_string(),
                request_timeout_ms: 99_000,
                context_window_tokens: None,
                max_output_tokens: None,
                provider_type: None,
            },
            ..Default::default()
        };

        save_settings_with_context(&ctx, custom).expect("save settings");

        // Read the raw file content — the API key must NOT appear in plaintext.
        let raw = fs::read_to_string(&ctx.paths.settings_path).expect("read settings file");
        assert!(
            !raw.contains("sk-sec...2345"),
            "API key must not appear in plaintext on disk"
        );
        assert!(
            raw.contains("ENC:v1:"),
            "settings file must contain encrypted API key"
        );

        // (a) Parse the on-disk JSON to verify the on-disk value differs from plaintext.
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("parse on-disk settings JSON");
        let on_disk_key = parsed["provider"]["apiKey"]
            .as_str()
            .expect("api_key should be a string");
        assert_ne!(
            on_disk_key, "sk-sec...2345",
            "on-disk API key must differ from plaintext"
        );
        assert!(
            on_disk_key.starts_with("ENC:v1:"),
            "on-disk API key should have ENC:v1: prefix"
        );

        // (b) Decrypt the on-disk value and verify round-trip.
        let decrypted = crate::crypto::decrypt_secret(on_disk_key).expect("decrypt on-disk key");
        assert_eq!(
            decrypted, "sk-sec...2345",
            "decrypted on-disk key must match original plaintext"
        );

        // Also verify the full load pipeline returns the plaintext key.
        let loaded = load_settings_with_context(&ctx).expect("load settings");
        assert_eq!(loaded.provider.api_key, "sk-sec...2345");
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
            search_snippet: None,
        };
        save_note_with_context(&ctx, note).expect("save");

        let stats = rebuild_index_with_context(&ctx).expect("rebuild");
        assert!(stats.scanned > 0);
        assert!(stats.indexed > 0);
    }
    #[test]
    fn export_id_prefix_safe_for_short_ids() {
        // Regression test for #675: [..8] panics on IDs shorter than 8 bytes
        // Updated for #843: chars().take(8) prevents UTF-8 boundary panic
        let short_id = "ab";
        let id_prefix: String = short_id.chars().take(8).collect();
        assert_eq!(id_prefix, "ab");

        let exact_8 = "12345678";
        let id_prefix: String = exact_8.chars().take(8).collect();
        assert_eq!(id_prefix, "12345678");

        let long_id = "1234567890abcdef";
        let id_prefix: String = long_id.chars().take(8).collect();
        assert_eq!(id_prefix, "12345678");

        let empty_id = "";
        let id_prefix: String = empty_id.chars().take(8).collect();
        assert_eq!(id_prefix, "");
    }

    #[test]
    fn export_id_prefix_safe_for_cjk_ids() {
        // Regression test for #843: byte-slice panics on non-ASCII note IDs
        // CJK characters are 3 bytes each; byte [..8] would split 語
        let cjk_id = "日本語abcdefghij";
        let id_prefix: String = cjk_id.chars().take(8).collect();
        assert_eq!(id_prefix, "日本語abcde"); // 日,本,語,a,b,c,d,e = 8 chars

        let short_cjk = "日本語";
        let id_prefix: String = short_cjk.chars().take(8).collect();
        assert_eq!(id_prefix, "日本語");

        let mixed_cjk = "abc日本語def";
        let id_prefix: String = mixed_cjk.chars().take(8).collect();
        assert_eq!(id_prefix, "abc日本語de"); // a,b,c,日,本,語,d,e = 8 chars
    }
}
