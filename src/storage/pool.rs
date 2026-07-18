use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use tokio::sync::Mutex as TokioMutex;

use crate::models::AppSettings;

use super::backup;

/// Type alias for a pooled SQLite connection.
pub(super) type PooledConnection = r2d2::PooledConnection<SqliteConnectionManager>;

#[derive(Debug, Clone)]
pub(super) struct AppPaths {
    pub settings_path: PathBuf,
    pub database_path: PathBuf,
    pub chat_state_path: PathBuf,
    pub default_vault_dir: PathBuf,
    pub vault_dir_override: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct StorageContext {
    pub(super) paths: AppPaths,
    /// Connection pool for SQLite database access.
    pub(super) pool: Pool<SqliteConnectionManager>,
    /// Cached parsed AppSettings, shared across clones of the same context.
    pub(super) cached_settings: Arc<Mutex<Option<AppSettings>>>,
    /// Lock to serialize chat-state load-modify-save operations and prevent
    /// lost updates when concurrent MCP HTTP requests mutate chat state.
    pub chat_state_lock: Arc<TokioMutex<()>>,
}

impl StorageContext {
    pub(super) fn with_pool(paths: AppPaths) -> Result<Self> {
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
            chat_state_lock: Arc::new(TokioMutex::new(())),
        })
    }

    /// Returns the effective vault directory (override or default).
    pub fn vault_dir(&self) -> &std::path::Path {
        self.paths
            .vault_dir_override
            .as_deref()
            .unwrap_or(&self.paths.default_vault_dir)
    }

    /// Returns the path of the on-disk `settings.json` file. Exposed so that
    /// regression/callers can inspect the persisted bytes (e.g. to assert a
    /// secret is never written in plaintext, issue #2826).
    pub fn settings_path(&self) -> &std::path::Path {
        &self.paths.settings_path
    }

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

    /// OS-level config root used to keep per-vault CLI state (which holds the
    /// machine-bound encrypted API key) *outside* the synced vault.
    ///
    /// Storing the secret inside `<vault>/.vaultpilot/` meant it travelled with
    /// the vault across devices (git / Syncthing / Obsidian Sync / Dropbox) and
    /// could no longer be decrypted on the second machine (#2831). Keeping it in
    /// the OS config dir keeps the secret device-local while the vault (notes,
    /// prompts, capabilities, sessions) still syncs normally.
    pub(crate) fn cli_config_root() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .unwrap_or_else(std::env::temp_dir)
            .join("vaultpilot")
            .join("cli-vaults")
    }

    /// Stable per-vault namespace so each vault keeps its own local CLI state
    /// (preserving multi-vault support) without leaking the secret into sync.
    ///
    /// Uses SHA-256 (not `DefaultHasher`) so the namespace is stable across
    /// Rust releases — `DefaultHasher`'s algorithm is unspecified and may
    /// change between versions, silently orphaning the persisted API key
    /// directory (#2851).
    pub(crate) fn vault_namespace(vault_dir: &std::path::Path) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(vault_dir.to_string_lossy().as_bytes());
        let digest = hasher.finalize();
        // 16 bytes → 32 hex chars (128 bits of entropy, more than enough for
        // a directory name).
        digest[..16].iter().map(|b| format!("{b:02x}")).collect()
    }

    /// One-time migration: move pre-#2831 in-vault `.vaultpilot` state files
    /// (which carried the machine-bound secret) to the device-local config dir.
    fn migrate_legacy_cli_state(vault_dir: &std::path::Path, target_dir: &std::path::Path) {
        let legacy = vault_dir.join(".vaultpilot");
        for name in ["settings.json", "chat-state.json"] {
            let src = legacy.join(name);
            let dst = target_dir.join(name);
            if src.exists() && !dst.exists() {
                match fs::rename(&src, &dst) {
                    Ok(()) => {}
                    // rename() fails across filesystems (e.g. vault on a
                    // different mount than the OS config dir) — fall back to a
                    // copy and then delete the source so the secret leaves sync.
                    Err(e) => {
                        tracing::warn!(
                            "cli(#2831): rename of {name} failed ({e}); copying instead"
                        );
                        if fs::copy(&src, &dst).is_ok() {
                            let _ = fs::remove_file(&src);
                        }
                    }
                }
            }
        }
        // Best-effort cleanup of the now-empty legacy directory.
        if legacy.exists() {
            let _ = fs::remove_dir(&legacy);
        }
    }

    pub fn for_cli(vault_dir_override: Option<PathBuf>) -> Result<Self> {
        let mut ctx = Self::for_sidecar()?;
        if let Some(raw_vault_dir) = vault_dir_override {
            // Canonicalize existing directories so that different path spellings
            // (~/vault vs /home/user/vault, relative vs absolute) map to the same
            // namespace. Fall back to the lexical path if the dir doesn't exist
            // yet (e.g. `vp init` on a fresh vault).
            let vault_dir = raw_vault_dir.canonicalize().unwrap_or(raw_vault_dir);
            let cli_state_dir = Self::cli_config_root().join(Self::vault_namespace(&vault_dir));
            fs::create_dir_all(&cli_state_dir)?;
            // Migrate any pre-#2831 in-vault state so existing CLI users keep
            // their settings after the secret leaves the synced vault.
            Self::migrate_legacy_cli_state(&vault_dir, &cli_state_dir);
            // The secret + local conversation state live device-local (do NOT
            // sync), while the regenerable knowledge index stays in the vault.
            ctx.paths.settings_path = cli_state_dir.join("settings.json");
            ctx.paths.chat_state_path = cli_state_dir.join("chat-state.json");
            ctx.paths.database_path = vault_dir.join(".vaultpilot").join("knowledge-index.sqlite");
            ctx.paths.default_vault_dir = vault_dir.clone();
            ctx.paths.vault_dir_override = Some(vault_dir);
            // Rebuild the pool for the new database path
            ctx = Self::with_pool(ctx.paths)?;
        } else {
            ctx.paths.vault_dir_override = None;
        }
        Ok(ctx)
    }

    /// Get a pooled SQLite connection.
    /// Public so that sibling modules (e.g. `mail`) can access the database.
    pub fn get_connection(
        &self,
    ) -> anyhow::Result<r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>> {
        self.pool
            .get()
            .map_err(|e| anyhow::anyhow!("failed to get connection from pool: {e}"))
    }

    #[cfg(test)]
    pub(crate) fn for_test(temp: &std::path::Path) -> Self {
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

/// Get a database connection from the connection pool.
/// Returns a `PooledConnection` that is automatically returned to the pool on drop.
pub(crate) fn open_connection(context: &StorageContext) -> Result<(PooledConnection, AppSettings)> {
    let settings = super::settings::load_settings_with_context(context)?;
    let conn = context
        .pool
        .get()
        .with_context(|| "failed to get connection from pool")?;
    Ok((conn, settings))
}

pub(super) fn ensure_schema(connection: &Connection) -> Result<()> {
    // Fast path: skip schema creation if already initialized in this process.
    // PRAGMA user_version is a lightweight integer stored in the database header.
    let version: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;

    // ── Self-Organizing Vault tables (Feature #2176) ──
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS analysis_queue (
            id TEXT PRIMARY KEY,
            note_id TEXT NOT NULL,
            action TEXT NOT NULL DEFAULT 'created',
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_analysis_queue_status ON analysis_queue(status);
        CREATE INDEX IF NOT EXISTS idx_analysis_queue_note_id ON analysis_queue(note_id);

        CREATE TABLE IF NOT EXISTS weak_links (
            id TEXT PRIMARY KEY,
            source_note_id TEXT NOT NULL,
            target_note_id TEXT NOT NULL,
            link_type TEXT NOT NULL DEFAULT 'content_similarity',
            score REAL NOT NULL DEFAULT 0.0,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(source_note_id, target_note_id, link_type)
        );

        CREATE INDEX IF NOT EXISTS idx_weak_links_status ON weak_links(status);
        CREATE INDEX IF NOT EXISTS idx_weak_links_source ON weak_links(source_note_id);
        CREATE INDEX IF NOT EXISTS idx_weak_links_target ON weak_links(target_note_id);

        CREATE TABLE IF NOT EXISTS collection_rules (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            keyword TEXT NOT NULL,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_collection_rules_keyword ON collection_rules(keyword);

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        -- Flashcards for FSRS spaced repetition (#1912)
        CREATE TABLE IF NOT EXISTS flashcards (
            id TEXT PRIMARY KEY,
            front TEXT NOT NULL,
            back TEXT NOT NULL,
            note_id TEXT NOT NULL DEFAULT '',
            tags TEXT NOT NULL DEFAULT '',
            scheduling TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_flashcards_tags ON flashcards(tags);
        CREATE INDEX IF NOT EXISTS idx_flashcards_created_at ON flashcards(created_at DESC);

        -- Note snapshots for persistent edit history (#2855)
        CREATE TABLE IF NOT EXISTS note_snapshots (
            id          TEXT PRIMARY KEY,
            note_id     TEXT NOT NULL,
            body        TEXT NOT NULL,
            frontmatter TEXT NOT NULL,
            source      TEXT NOT NULL DEFAULT 'user',
            created_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_note_snapshots_note
            ON note_snapshots(note_id, created_at DESC);
        "#,
    )?;

    if version >= 1 {
        // Schema already exists; enable foreign keys, WAL mode, and busy timeout.
        connection.execute_batch(
            "PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;",
        )?;
        ensure_attachment_columns(connection)?;
        ensure_note_columns(connection)?;
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
            body_hash TEXT NOT NULL,
            semantic_vector TEXT NOT NULL DEFAULT ''
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

        -- Collections for many-to-many note grouping (#2042)
        CREATE TABLE IF NOT EXISTS collections (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS note_collections (
            note_id TEXT NOT NULL,
            collection_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (note_id, collection_id),
            FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE,
            FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_note_collections_note_id ON note_collections(note_id);
        CREATE INDEX IF NOT EXISTS idx_note_collections_collection_id ON note_collections(collection_id);

        -- Subscriptions for AI Scheduled Research (#2167)
        CREATE TABLE IF NOT EXISTS subscriptions (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            schedule TEXT NOT NULL,
            prompt TEXT NOT NULL,
            tools TEXT NOT NULL DEFAULT '',
            target_collection TEXT NOT NULL DEFAULT '',
            enabled INTEGER NOT NULL DEFAULT 1,
            last_run_at TEXT NOT NULL DEFAULT '',
            next_run_at TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            run_count INTEGER NOT NULL DEFAULT 0,
            last_status TEXT NOT NULL DEFAULT '',
            last_error TEXT NOT NULL DEFAULT ''
        );

        -- RSS/Atom/JSON Feed subscriptions for auto-ingestion (#3041)
        -- Each row is one external feed polled periodically; new entries are
        -- converted to Markdown and stored as vault notes (reusing the Web
        -- Clipper conversion pipeline).
        CREATE TABLE IF NOT EXISTS feeds (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL DEFAULT '',
            url TEXT NOT NULL,
            kind TEXT NOT NULL DEFAULT 'rss',
            collection TEXT NOT NULL DEFAULT '',
            tags TEXT NOT NULL DEFAULT '',
            interval_minutes INTEGER NOT NULL DEFAULT 60,
            enabled INTEGER NOT NULL DEFAULT 1,
            last_fetched_at TEXT NOT NULL DEFAULT '',
            etag TEXT NOT NULL DEFAULT '',
            last_modified TEXT NOT NULL DEFAULT '',
            last_entry_id TEXT NOT NULL DEFAULT '',
            last_entry_date TEXT NOT NULL DEFAULT '',
            last_status TEXT NOT NULL DEFAULT '',
            last_error TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_feeds_enabled ON feeds(enabled);

        -- Agent trigger rules for vault event automation (#2984)
        CREATE TABLE IF NOT EXISTS trigger_rules (
            id TEXT PRIMARY KEY,
            label TEXT NOT NULL,
            trigger_type TEXT NOT NULL,
            trigger_config TEXT NOT NULL,
            filter TEXT,
            action TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            custom_prompt TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- Trigger execution log (#3048): one row per fired rule.
        -- Populated by the trigger executor (src/orchestration/trigger_executor.rs).
        CREATE TABLE IF NOT EXISTS trigger_executions (
            id TEXT PRIMARY KEY,
            rule_id TEXT NOT NULL,
            label TEXT NOT NULL,
            action TEXT NOT NULL,
            fired_at TEXT NOT NULL,
            status TEXT NOT NULL,
            error TEXT NOT NULL DEFAULT '',
            detail TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_trigger_executions_rule_id ON trigger_executions(rule_id);
        CREATE INDEX IF NOT EXISTS idx_trigger_executions_fired_at ON trigger_executions(fired_at);
        "#,
    )?;

    // Idempotent migration: add execution-tracking columns to trigger_rules
    // for the cron executor introduced in #3048. Older databases created
    // before #3048 ship without these columns.
    ensure_trigger_rule_columns(connection)?;

    // Ensure mail tables for Email-to-Vault integration
    connection.execute_batch(crate::mail::MAIL_SCHEMA_DDL)?;
    connection.execute_batch("PRAGMA user_version = 1;")?;
    Ok(())
}

/// Add `last_fired_at`, `next_fire_at`, `run_count`, `last_status`, `last_error`
/// columns to the `trigger_rules` table if missing (#3048). Idempotent.
fn ensure_trigger_rule_columns(connection: &Connection) -> Result<()> {
    let table_exists: bool = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='trigger_rules'",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(());
    }

    let mut statement = connection.prepare("PRAGMA table_info(trigger_rules)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    drop(statement);

    for (column, ddl) in [
        (
            "last_fired_at",
            "ALTER TABLE trigger_rules ADD COLUMN last_fired_at TEXT NOT NULL DEFAULT ''",
        ),
        (
            "next_fire_at",
            "ALTER TABLE trigger_rules ADD COLUMN next_fire_at TEXT NOT NULL DEFAULT ''",
        ),
        (
            "run_count",
            "ALTER TABLE trigger_rules ADD COLUMN run_count INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "last_status",
            "ALTER TABLE trigger_rules ADD COLUMN last_status TEXT NOT NULL DEFAULT ''",
        ),
        (
            "last_error",
            "ALTER TABLE trigger_rules ADD COLUMN last_error TEXT NOT NULL DEFAULT ''",
        ),
    ] {
        if !columns.contains(column) {
            connection.execute_batch(ddl)?;
        }
    }
    Ok(())
}

fn ensure_attachment_columns(connection: &Connection) -> Result<()> {
    // Check if attachments table exists; skip migration if it doesn't.
    let table_exists: bool = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attachments'",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(());
    }

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

fn ensure_note_columns(connection: &Connection) -> Result<()> {
    let table_exists: bool = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='notes'",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(());
    }

    let mut statement = connection.prepare("PRAGMA table_info(notes)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;

    for (column, ddl) in [(
        "semantic_vector",
        "ALTER TABLE notes ADD COLUMN semantic_vector TEXT NOT NULL DEFAULT ''",
    )] {
        if !columns.contains(column) {
            connection.execute(ddl, [])?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn in_memory_conn() -> Connection {
        Connection::open_in_memory().expect("failed to open in-memory SQLite")
    }

    #[test]
    fn ensure_schema_creates_tables_on_fresh_db() {
        let conn = in_memory_conn();
        ensure_schema(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(tables.contains(&"notes".to_string()), "notes table missing");
        assert!(
            tables.contains(&"attachments".to_string()),
            "attachments table missing"
        );

        let vtables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE '%_fts%' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(
            vtables.iter().any(|t| t.contains("note_fts")),
            "note_fts missing"
        );
        assert!(
            vtables.iter().any(|t| t.contains("attachment_fts")),
            "attachment_fts missing"
        );

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn ensure_schema_is_idempotent() {
        let conn = in_memory_conn();
        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();
    }

    #[test]
    fn ensure_schema_skips_when_version_already_set() {
        let conn = in_memory_conn();
        conn.execute_batch("PRAGMA user_version = 1;").unwrap();
        ensure_schema(&conn).unwrap();

        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='notes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "should not create tables on fast path");
    }

    #[test]
    fn ensure_attachment_columns_adds_all_columns() {
        let conn = in_memory_conn();
        conn.execute_batch(
            "CREATE TABLE attachments (id TEXT PRIMARY KEY, note_id TEXT NOT NULL, path TEXT NOT NULL, created_at TEXT NOT NULL);",
        ).unwrap();

        ensure_attachment_columns(&conn).unwrap();

        let columns: HashSet<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(attachments)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };

        for col in [
            "file_name",
            "stem",
            "ocr_text",
            "semantic_vector",
            "perceptual_hash",
        ] {
            assert!(
                columns.contains(col),
                "column '{col}' missing after ensure_attachment_columns"
            );
        }
    }

    #[test]
    fn ensure_attachment_columns_is_idempotent() {
        let conn = in_memory_conn();
        conn.execute_batch(
            "CREATE TABLE attachments (id TEXT PRIMARY KEY, note_id TEXT NOT NULL, path TEXT NOT NULL, created_at TEXT NOT NULL);",
        ).unwrap();

        ensure_attachment_columns(&conn).unwrap();
        ensure_attachment_columns(&conn).unwrap();
    }

    #[test]
    fn ensure_note_columns_adds_semantic_vector() {
        let conn = in_memory_conn();
        conn.execute_batch(
            "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL, tags TEXT NOT NULL, keywords TEXT NOT NULL, platform TEXT NOT NULL, board TEXT NOT NULL, kernel TEXT NOT NULL, status TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, source TEXT NOT NULL, path TEXT NOT NULL UNIQUE, summary TEXT NOT NULL, body_hash TEXT NOT NULL);",
        ).unwrap();

        ensure_note_columns(&conn).unwrap();

        let columns: HashSet<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(notes)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };

        assert!(
            columns.contains("semantic_vector"),
            "column 'semantic_vector' missing after ensure_note_columns"
        );
    }

    #[test]
    fn ensure_note_columns_is_idempotent() {
        let conn = in_memory_conn();
        conn.execute_batch(
            "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL, tags TEXT NOT NULL, keywords TEXT NOT NULL, platform TEXT NOT NULL, board TEXT NOT NULL, kernel TEXT NOT NULL, status TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, source TEXT NOT NULL, path TEXT NOT NULL UNIQUE, summary TEXT NOT NULL, body_hash TEXT NOT NULL);",
        ).unwrap();

        ensure_note_columns(&conn).unwrap();
        ensure_note_columns(&conn).unwrap();
    }

    // ── Regression test for #2851 ─────────────────────────────────

    #[test]
    fn vault_namespace_is_deterministic_and_stable() {
        // Regression test for #2851: vault_namespace must use a stable hash
        // (SHA-256) so the same vault path always maps to the same namespace,
        // even across Rust releases. DefaultHasher's algorithm is unspecified.
        use std::path::Path;

        let p1 = Path::new("/home/user/myvault");
        let p2 = Path::new("/home/user/myvault");
        let p3 = Path::new("/home/user/othervault");

        // Same path → same namespace (deterministic).
        let ns1 = StorageContext::vault_namespace(p1);
        let ns2 = StorageContext::vault_namespace(p2);
        assert_eq!(ns1, ns2, "same path must produce same namespace");

        // Different path → different namespace.
        let ns3 = StorageContext::vault_namespace(p3);
        assert_ne!(
            ns1, ns3,
            "different paths must produce different namespaces"
        );

        // Must be a valid hex string (SHA-256 output).
        assert!(
            ns1.chars().all(|c| c.is_ascii_hexdigit()),
            "namespace must be hex, got: {ns1}"
        );
        assert_eq!(ns1.len(), 32, "namespace should be 32 hex chars (16 bytes)");

        // Must NOT match the old DefaultHasher output format (which used
        // format!("{:x}", u64)). The old format was up to 16 hex chars;
        // SHA-256 truncation gives 32.
        assert_ne!(
            ns1.len(),
            16,
            "must not produce old DefaultHasher-length output"
        );
    }
}
