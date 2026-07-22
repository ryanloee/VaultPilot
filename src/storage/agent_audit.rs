//! Agent Audit Log — persistent tracking of AI Agent lifecycle events and
//! high-level operations (#3287).
//!
//! Each row records a business-level operation performed by an agent:
//! agent config creation/modification/deletion, note CRUD by agent, search
//! invocations, and other agent-triggered actions.
//!
//! This is separate from the in-memory `AgentAuditEntry` / `ToolProxy` audit
//! log, which tracks individual tool calls per-session. This table persists
//! across restarts and sessions.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::pool::StorageContext;

/// A single agent audit log entry stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAuditEntry {
    pub id: String,
    pub agent_name: String,
    pub session_id: String,
    /// The type of operation performed.
    /// Examples: "agent_created", "agent_modified", "agent_deleted",
    ///           "create_note", "update_note", "delete_note", "search",
    ///           "tool_call", "session_started", "session_completed"
    pub operation_type: String,
    /// Comma-separated note IDs affected (if applicable).
    pub note_ids: String,
    /// What triggered this operation: "user", "cron", "webhook", "system".
    pub trigger_source: String,
    /// Free-text details (diff summary, error message, extra context).
    pub details: String,
    /// ISO-8601 timestamp of the event.
    pub created_at: String,
}

/// DDL for the agent_audit_log table. Called from `pool::ensure_schema`.
#[allow(dead_code)]
pub const AGENT_AUDIT_LOG_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS agent_audit_log (
    id TEXT PRIMARY KEY,
    agent_name TEXT NOT NULL,
    session_id TEXT NOT NULL DEFAULT '',
    operation_type TEXT NOT NULL,
    note_ids TEXT NOT NULL DEFAULT '',
    trigger_source TEXT NOT NULL DEFAULT '',
    details TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_audit_log_created_at ON agent_audit_log(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_agent_audit_log_agent_name ON agent_audit_log(agent_name);
CREATE INDEX IF NOT EXISTS idx_agent_audit_log_operation_type ON agent_audit_log(operation_type);
CREATE INDEX IF NOT EXISTS idx_agent_audit_log_session_id ON agent_audit_log(session_id);
"#;

/// Insert a new agent audit log entry.
pub fn insert_agent_audit_entry(conn: &Connection, entry: &AgentAuditEntry) -> Result<()> {
    conn.execute(
        "INSERT INTO agent_audit_log (id, agent_name, session_id, operation_type, note_ids, trigger_source, details, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            entry.id,
            entry.agent_name,
            entry.session_id,
            entry.operation_type,
            entry.note_ids,
            entry.trigger_source,
            entry.details,
            entry.created_at,
        ],
    )
    .with_context(|| "failed to insert agent_audit_log entry")?;
    Ok(())
}

/// Insert an audit entry using a StorageContext (convenience wrapper).
pub fn insert_agent_audit_entry_with_context(
    context: &StorageContext,
    agent_name: &str,
    session_id: &str,
    operation_type: &str,
    note_ids: &[String],
    trigger_source: &str,
    details: &str,
) -> Result<AgentAuditEntry> {
    let conn = context
        .pool
        .get()
        .with_context(|| "failed to get connection from pool")?;
    let now = chrono::Utc::now().to_rfc3339();
    let entry = AgentAuditEntry {
        id: Uuid::new_v4().to_string(),
        agent_name: agent_name.to_string(),
        session_id: session_id.to_string(),
        operation_type: operation_type.to_string(),
        note_ids: note_ids.join(","),
        trigger_source: trigger_source.to_string(),
        details: details.to_string(),
        created_at: now,
    };
    insert_agent_audit_entry(&conn, &entry)?;
    Ok(entry)
}

/// Query parameters for the audit log.
#[derive(Debug, Default)]
pub struct AuditLogQuery {
    pub agent_name: Option<String>,
    pub operation_type: Option<String>,
    pub session_id: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

/// Query the agent audit log with optional filters. Returns entries sorted by
/// `created_at DESC` (most recent first).
pub fn query_agent_audit_log(
    conn: &Connection,
    query: &AuditLogQuery,
) -> Result<Vec<AgentAuditEntry>> {
    let mut sql = String::from(
        "SELECT id, agent_name, session_id, operation_type, note_ids, trigger_source, details, created_at
         FROM agent_audit_log WHERE 1=1",
    );
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref agent_name) = query.agent_name {
        sql.push_str(" AND agent_name = ?");
        param_values.push(Box::new(agent_name.clone()));
    }
    if let Some(ref op_type) = query.operation_type {
        sql.push_str(" AND operation_type = ?");
        param_values.push(Box::new(op_type.clone()));
    }
    if let Some(ref session_id) = query.session_id {
        sql.push_str(" AND session_id = ?");
        param_values.push(Box::new(session_id.clone()));
    }
    if let Some(ref since) = query.since {
        sql.push_str(" AND created_at >= ?");
        param_values.push(Box::new(since.clone()));
    }
    if let Some(ref until) = query.until {
        sql.push_str(" AND created_at <= ?");
        param_values.push(Box::new(until.clone()));
    }

    sql.push_str(" ORDER BY created_at DESC");
    sql.push_str(&format!(
        " LIMIT {} OFFSET {}",
        query.limit.max(1).min(1000),
        query.offset
    ));

    let mut stmt = conn.prepare(&sql)?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok(AgentAuditEntry {
            id: row.get(0)?,
            agent_name: row.get(1)?,
            session_id: row.get(2)?,
            operation_type: row.get(3)?,
            note_ids: row.get(4)?,
            trigger_source: row.get(5)?,
            details: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

/// Query the audit log using a StorageContext (convenience wrapper).
pub fn query_agent_audit_log_with_context(
    context: &StorageContext,
    query: &AuditLogQuery,
) -> Result<Vec<AgentAuditEntry>> {
    let conn = context
        .pool
        .get()
        .with_context(|| "failed to get connection from pool")?;
    query_agent_audit_log(&conn, query)
}

/// Delete audit log entries older than `retention_days`. Returns the number
/// of rows deleted.
pub fn prune_agent_audit_log(conn: &Connection, retention_days: u64) -> Result<usize> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
    let cutoff_str = cutoff.to_rfc3339();
    let deleted = conn
        .execute(
            "DELETE FROM agent_audit_log WHERE created_at < ?1",
            params![cutoff_str],
        )
        .with_context(|| "failed to prune agent_audit_log")?;
    Ok(deleted)
}

/// Prune audit log entries using a StorageContext (convenience wrapper).
pub fn prune_agent_audit_log_with_context(
    context: &StorageContext,
    retention_days: u64,
) -> Result<usize> {
    let conn = context
        .pool
        .get()
        .with_context(|| "failed to get connection from pool")?;
    prune_agent_audit_log(&conn, retention_days)
}

/// Get the total count of audit log entries (with optional filter).
pub fn count_agent_audit_entries(conn: &Connection) -> Result<usize> {
    let count: usize = conn
        .query_row("SELECT COUNT(*) FROM agent_audit_log", [], |row| row.get(0))
        .with_context(|| "failed to count agent_audit_log entries")?;
    Ok(count)
}

/// Get distinct agent names that have audit log entries.
pub fn list_agent_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT agent_name FROM agent_audit_log ORDER BY agent_name")
        .with_context(|| "failed to prepare agent name query")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .with_context(|| "failed to collect agent names")?;
    Ok(names)
}

/// Get distinct operation types that have been recorded.
pub fn list_operation_types(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT operation_type FROM agent_audit_log ORDER BY operation_type")
        .with_context(|| "failed to prepare operation type query")?;
    let types = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .with_context(|| "failed to collect operation types")?;
    Ok(types)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::pool;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").ok();
        conn.execute_batch(AGENT_AUDIT_LOG_DDL).unwrap();
        conn
    }

    #[test]
    fn test_insert_and_query() {
        let conn = setup_test_db();

        let entry = AgentAuditEntry {
            id: "test-1".into(),
            agent_name: "claude-code".into(),
            session_id: "session-1".into(),
            operation_type: "agent_created".into(),
            note_ids: "".into(),
            trigger_source: "user".into(),
            details: "Agent created with read-only permissions".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        insert_agent_audit_entry(&conn, &entry).unwrap();

        let result = query_agent_audit_log(
            &conn,
            &AuditLogQuery {
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].agent_name, "claude-code");
        assert_eq!(result[0].operation_type, "agent_created");
    }

    #[test]
    fn test_query_filter_by_agent_name() {
        let conn = setup_test_db();

        for i in 0..3 {
            let entry = AgentAuditEntry {
                id: format!("test-agent-{}", i),
                agent_name: format!("agent-{}", i),
                session_id: "".into(),
                operation_type: "agent_created".into(),
                note_ids: "".into(),
                trigger_source: "user".into(),
                details: "".into(),
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            insert_agent_audit_entry(&conn, &entry).unwrap();
        }

        let result = query_agent_audit_log(
            &conn,
            &AuditLogQuery {
                agent_name: Some("agent-1".into()),
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].agent_name, "agent-1");
    }

    #[test]
    fn test_query_filter_by_operation_type() {
        let conn = setup_test_db();

        let ops = ["agent_created", "agent_modified", "agent_deleted"];
        for (i, op) in ops.iter().enumerate() {
            let entry = AgentAuditEntry {
                id: format!("test-op-{}", i),
                agent_name: "test-agent".into(),
                session_id: "".into(),
                operation_type: op.to_string(),
                note_ids: "".into(),
                trigger_source: "user".into(),
                details: "".into(),
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            insert_agent_audit_entry(&conn, &entry).unwrap();
        }

        let result = query_agent_audit_log(
            &conn,
            &AuditLogQuery {
                operation_type: Some("agent_modified".into()),
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].operation_type, "agent_modified");
    }

    #[test]
    fn test_query_pagination() {
        let conn = setup_test_db();

        for i in 0..5 {
            let entry = AgentAuditEntry {
                id: format!("test-page-{}", i),
                agent_name: "test".into(),
                session_id: "".into(),
                operation_type: "agent_created".into(),
                note_ids: "".into(),
                trigger_source: "user".into(),
                details: "".into(),
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            insert_agent_audit_entry(&conn, &entry).unwrap();
        }

        let page1 = query_agent_audit_log(
            &conn,
            &AuditLogQuery {
                limit: 2,
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page1.len(), 2);

        let page2 = query_agent_audit_log(
            &conn,
            &AuditLogQuery {
                limit: 2,
                offset: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page2.len(), 2);
        // Ensure different entries (different IDs)
        assert_ne!(page1[0].id, page2[0].id);
    }

    #[test]
    fn test_prune() {
        let conn = setup_test_db();

        let old_date = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        let entry = AgentAuditEntry {
            id: "old-entry".into(),
            agent_name: "test".into(),
            session_id: "".into(),
            operation_type: "agent_created".into(),
            note_ids: "".into(),
            trigger_source: "user".into(),
            details: "".into(),
            created_at: old_date,
        };
        insert_agent_audit_entry(&conn, &entry).unwrap();

        let recent_entry = AgentAuditEntry {
            id: "recent-entry".into(),
            agent_name: "test".into(),
            session_id: "".into(),
            operation_type: "agent_created".into(),
            note_ids: "".into(),
            trigger_source: "user".into(),
            details: "".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        insert_agent_audit_entry(&conn, &recent_entry).unwrap();

        // Prune entries older than 30 days
        let deleted = prune_agent_audit_log(&conn, 30).unwrap();
        assert_eq!(deleted, 1);

        let remaining = query_agent_audit_log(
            &conn,
            &AuditLogQuery {
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "recent-entry");
    }

    #[test]
    fn test_count_and_list_agent_names() {
        let conn = setup_test_db();

        assert_eq!(count_agent_audit_entries(&conn).unwrap(), 0);

        for name in &["alpha", "beta", "alpha"] {
            let entry = AgentAuditEntry {
                id: Uuid::new_v4().to_string(),
                agent_name: name.to_string(),
                session_id: "".into(),
                operation_type: "agent_created".into(),
                note_ids: "".into(),
                trigger_source: "user".into(),
                details: "".into(),
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            insert_agent_audit_entry(&conn, &entry).unwrap();
        }

        assert_eq!(count_agent_audit_entries(&conn).unwrap(), 3);

        let names = list_agent_names(&conn).unwrap();
        assert_eq!(names, vec!["alpha", "beta"]);
    }
}
