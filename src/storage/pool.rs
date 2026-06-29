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
pub(super) fn open_connection(context: &StorageContext) -> Result<(PooledConnection, AppSettings)> {
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
    if version >= 1 {
        // Schema already exists; enable foreign keys, WAL mode, and busy timeout.
        connection.execute_batch(
            "PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;",
        )?;
        ensure_attachment_columns(connection)?;
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
        "#,
    )?;
    ensure_attachment_columns(connection)?;
    connection.execute_batch("PRAGMA user_version = 1;")?;
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
}
