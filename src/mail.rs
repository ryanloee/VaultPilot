//! Email-to-Vault integration — IMAP connector, email storage, and note creation.
//!
//! # Phase 1 scope
//! - IMAP connector with TLS support
//! - Email parsing (MIME) and dedup via Message-ID
//! - Store emails in SQLite, link to vault notes
//! - CLI commands: `mail add`, `mail list`, `mail delete`, `mail sync`
//! - MCP tools: `email.search`, `email.get`
//!
//! # Architecture
//! Mail account credentials are encrypted on disk using the machine-bound
//! AES-256-GCM key (same as API keys in settings).  Emails are fetched via
//! IMAP, parsed, saved to the SQLite `emails` table, and also persisted as
//! vault markdown notes for full-text search and AI context.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use crate::crypto;
use crate::storage::StorageContext;

/// Get a pooled SQLite connection from the storage context.
fn db_conn(
    context: &StorageContext,
) -> Result<r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>> {
    // Access the pool via the public method we add to StorageContext
    context
        .get_connection()
        .context("failed to get database connection")
}

// ─── Data types ───────────────────────────────────────────────────

/// A configured mail account whose inbox we can sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAccount {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Encrypted on disk; decrypted in memory after load.
    #[serde(skip_serializing)]
    pub password: String,
    pub use_tls: bool,
    pub sync_enabled: bool,
    pub sync_frequency_minutes: u64,
    pub last_sync_at: String,
    pub created_at: String,
    pub updated_at: String,
}

/// An email that was imported and stored as a vault note.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredEmail {
    pub id: String,
    pub account_id: String,
    pub message_id: String,
    pub subject: String,
    pub from_addr: String,
    pub to_addrs: String,
    pub cc_addrs: String,
    pub date: String,
    pub body_text: String,
    /// Note ID of the vault note created from this email.
    pub note_id: String,
    pub imported_at: String,
}

/// Result of a sync operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub account_id: String,
    pub fetched: usize,
    pub imported: usize,
    pub skipped_duplicates: usize,
    pub errors: Vec<String>,
}

// ─── Mail account management ──────────────────────────────────────

/// Add a new mail account. Password is encrypted before storage.
#[allow(clippy::too_many_arguments)]
#[instrument(skip(context))]
pub fn add_mail_account(
    context: &StorageContext,
    name: &str,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    use_tls: bool,
    sync_frequency_minutes: u64,
) -> Result<MailAccount> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let id = Uuid::new_v4().to_string();

    let encrypted_pw =
        crypto::encrypt_secret(password).context("failed to encrypt mail account password")?;

    let account = MailAccount {
        id,
        name: name.to_string(),
        host: host.to_string(),
        port,
        username: username.to_string(),
        password: password.to_string(), // plaintext in memory
        use_tls,
        sync_enabled: true,
        sync_frequency_minutes,
        last_sync_at: String::new(),
        created_at: now.clone(),
        updated_at: now,
    };

    let conn = db_conn(context)?;
    conn.execute(
        "INSERT INTO mail_accounts (id, name, host, port, username, password, use_tls, sync_enabled, sync_frequency_minutes, last_sync_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            account.id,
            account.name,
            account.host,
            account.port,
            account.username,
            encrypted_pw,
            account.use_tls as i32,
            account.sync_enabled as i32,
            account.sync_frequency_minutes as i64,
            account.last_sync_at,
            account.created_at,
            account.updated_at,
        ],
    )?;

    Ok(account)
}

/// List all configured mail accounts (passwords decrypted in memory).
#[instrument(skip(context))]
pub fn list_mail_accounts(context: &StorageContext) -> Result<Vec<MailAccount>> {
    let conn = db_conn(context)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, username, password, use_tls, sync_enabled, sync_frequency_minutes, last_sync_at, created_at, updated_at
         FROM mail_accounts ORDER BY created_at DESC",
    )?;

    let accounts = stmt
        .query_map([], |row| {
            let encrypted_pw: String = row.get(5)?;
            Ok(MailAccount {
                id: row.get(0)?,
                name: row.get(1)?,
                host: row.get(2)?,
                port: row.get(3)?,
                username: row.get(4)?,
                password: encrypted_pw, // will decrypt below
                use_tls: row.get::<_, i32>(6)? != 0,
                sync_enabled: row.get::<_, i32>(7)? != 0,
                sync_frequency_minutes: row.get::<_, i64>(8)? as u64,
                last_sync_at: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Decrypt passwords in memory
    let accounts = accounts
        .into_iter()
        .map(|mut a| {
            if !a.password.is_empty() {
                a.password = crypto::decrypt_secret(&a.password).unwrap_or_else(|_| {
                    tracing::warn!(account = %a.id, "failed to decrypt mail password");
                    String::new()
                });
            }
            a
        })
        .collect();

    Ok(accounts)
}

/// Delete a mail account and its imported emails (cascade).
#[instrument(skip(context))]
pub fn delete_mail_account(context: &StorageContext, id: &str) -> Result<bool> {
    let conn = db_conn(context)?;
    let rows = conn.execute(
        "DELETE FROM mail_accounts WHERE id = ?1",
        rusqlite::params![id],
    )?;
    Ok(rows > 0)
}

/// Get a single mail account by ID (password decrypted).
#[instrument(skip(context))]
pub fn get_mail_account(context: &StorageContext, id: &str) -> Result<Option<MailAccount>> {
    let conn = db_conn(context)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, username, password, use_tls, sync_enabled, sync_frequency_minutes, last_sync_at, created_at, updated_at
         FROM mail_accounts WHERE id = ?1",
    )?;

    let mut account = stmt
        .query_row(rusqlite::params![id], |row| {
            let encrypted_pw: String = row.get(5)?;
            Ok(MailAccount {
                id: row.get(0)?,
                name: row.get(1)?,
                host: row.get(2)?,
                port: row.get(3)?,
                username: row.get(4)?,
                password: encrypted_pw,
                use_tls: row.get::<_, i32>(6)? != 0,
                sync_enabled: row.get::<_, i32>(7)? != 0,
                sync_frequency_minutes: row.get::<_, i64>(8)? as u64,
                last_sync_at: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
            })
        })
        .ok();

    if let Some(ref mut a) = account {
        if !a.password.is_empty() {
            a.password = crypto::decrypt_secret(&a.password).unwrap_or_else(|_| {
                tracing::warn!(account = %a.id, "failed to decrypt mail password");
                String::new()
            });
        }
    }

    Ok(account)
}

/// Update `last_sync_at` for an account.
#[instrument(skip(context))]
pub fn update_account_sync_time(context: &StorageContext, id: &str) -> Result<()> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let conn = db_conn(context)?;
    conn.execute(
        "UPDATE mail_accounts SET last_sync_at = ?1, updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    )?;
    Ok(())
}

// ─── Email storage ────────────────────────────────────────────────

/// Store an imported email in the database.
#[allow(clippy::too_many_arguments)]
#[instrument(skip(context))]
pub fn store_email(
    context: &StorageContext,
    account_id: &str,
    message_id: &str,
    subject: &str,
    from_addr: &str,
    to_addrs: &str,
    cc_addrs: &str,
    date: &str,
    body_text: &str,
    note_id: &str,
) -> Result<StoredEmail> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let id = Uuid::new_v4().to_string();

    let conn = db_conn(context)?;
    conn.execute(
        "INSERT INTO emails (id, account_id, message_id, subject, from_addr, to_addrs, cc_addrs, date, body_text, note_id, imported_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![id, account_id, message_id, subject, from_addr, to_addrs, cc_addrs, date, body_text, note_id, now],
    )?;

    Ok(StoredEmail {
        id,
        account_id: account_id.to_string(),
        message_id: message_id.to_string(),
        subject: subject.to_string(),
        from_addr: from_addr.to_string(),
        to_addrs: to_addrs.to_string(),
        cc_addrs: cc_addrs.to_string(),
        date: date.to_string(),
        body_text: body_text.to_string(),
        note_id: note_id.to_string(),
        imported_at: now,
    })
}

/// Check if a Message-ID already exists for this account (dedup).
#[instrument(skip(context))]
pub fn email_exists(context: &StorageContext, account_id: &str, message_id: &str) -> Result<bool> {
    let conn = db_conn(context)?;
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM emails WHERE account_id = ?1 AND message_id = ?2)",
        rusqlite::params![account_id, message_id],
        |row| row.get(0),
    )?;
    Ok(exists)
}

/// Search imported emails by subject, from, or body text.
#[instrument(skip(context))]
pub fn search_emails(
    context: &StorageContext,
    query: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<StoredEmail>> {
    let conn = db_conn(context)?;
    let pattern = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT id, account_id, message_id, subject, from_addr, to_addrs, cc_addrs, date, body_text, note_id, imported_at
         FROM emails
         WHERE subject LIKE ?1 OR from_addr LIKE ?1 OR body_text LIKE ?1
         ORDER BY date DESC
         LIMIT ?2 OFFSET ?3",
    )?;

    let emails = stmt
        .query_map(
            rusqlite::params![pattern, limit as i64, offset as i64],
            |row| {
                Ok(StoredEmail {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    message_id: row.get(2)?,
                    subject: row.get(3)?,
                    from_addr: row.get(4)?,
                    to_addrs: row.get(5)?,
                    cc_addrs: row.get(6)?,
                    date: row.get(7)?,
                    body_text: row.get(8)?,
                    note_id: row.get(9)?,
                    imported_at: row.get(10)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(emails)
}

/// Get a single stored email by ID.
#[instrument(skip(context))]
pub fn get_email(context: &StorageContext, id: &str) -> Result<Option<StoredEmail>> {
    let conn = db_conn(context)?;
    let mut stmt = conn.prepare(
        "SELECT id, account_id, message_id, subject, from_addr, to_addrs, cc_addrs, date, body_text, note_id, imported_at
         FROM emails WHERE id = ?1",
    )?;

    let email = stmt
        .query_row(rusqlite::params![id], |row| {
            Ok(StoredEmail {
                id: row.get(0)?,
                account_id: row.get(1)?,
                message_id: row.get(2)?,
                subject: row.get(3)?,
                from_addr: row.get(4)?,
                to_addrs: row.get(5)?,
                cc_addrs: row.get(6)?,
                date: row.get(7)?,
                body_text: row.get(8)?,
                note_id: row.get(9)?,
                imported_at: row.get(10)?,
            })
        })
        .ok();

    Ok(email)
}

// ─── IMAP sync ────────────────────────────────────────────────────

/// Sync a mail account: fetch unseen emails, create vault notes, store records.
///
/// This is a blocking operation that performs IMAP network I/O.
/// Callers should use `tokio::task::spawn_blocking` if calling from async context.
#[instrument(skip(context))]
pub fn sync_mail_account(context: &StorageContext, account_id: &str) -> Result<SyncResult> {
    let account = get_mail_account(context, account_id)?
        .ok_or_else(|| anyhow::anyhow!("mail account not found: {account_id}"))?;

    if !account.sync_enabled {
        return Ok(SyncResult {
            account_id: account_id.to_string(),
            fetched: 0,
            imported: 0,
            skipped_duplicates: 0,
            errors: vec![],
        });
    }

    let mut result = SyncResult {
        account_id: account_id.to_string(),
        fetched: 0,
        imported: 0,
        skipped_duplicates: 0,
        errors: vec![],
    };

    // Connect to IMAP server with TLS (IMAPS on port 993)
    let tls = native_tls::TlsConnector::builder()
        .build()
        .context("failed to build TLS connector")?;

    let tcp = std::net::TcpStream::connect((account.host.as_str(), account.port))
        .with_context(|| format!("failed to connect to {}:{}", account.host, account.port))?;

    if account.use_tls {
        let tls_stream = tls
            .connect(&account.host, tcp)
            .context("TLS handshake failed")?;
        let client = imap::Client::new(tls_stream);
        let mut session = client
            .login(&account.username, &account.password)
            .map_err(|(e, _)| anyhow::anyhow!("IMAP login failed: {e}"))?;
        result = sync_inbox(&mut session, context, account_id, result)?;
    } else {
        let client = imap::Client::new(tcp);
        let mut session = client
            .login(&account.username, &account.password)
            .map_err(|(e, _)| anyhow::anyhow!("IMAP login failed: {e}"))?;
        result = sync_inbox(&mut session, context, account_id, result)?;
    }

    // Update last_sync_at
    update_account_sync_time(context, account_id)?;

    Ok(result)
}

/// Inner sync logic that works with any IMAP session type.
fn sync_inbox<T>(
    session: &mut imap::Session<T>,
    context: &StorageContext,
    account_id: &str,
    mut result: SyncResult,
) -> Result<SyncResult>
where
    T: std::io::Read + std::io::Write,
{
    session.select("INBOX").context("failed to select INBOX")?;

    // Search for unseen messages
    let uids = session.uid_search("UNSEEN").context("UID SEARCH failed")?;
    let uid_vec: Vec<u32> = uids.into_iter().collect();
    result.fetched = uid_vec.len();

    for uid in &uid_vec {
        let uid_str = uid.to_string();
        match fetch_and_process_email(context, session, account_id, *uid) {
            Ok(imported) => {
                if imported {
                    result.imported += 1;
                } else {
                    result.skipped_duplicates += 1;
                }
            }
            Err(e) => {
                let err_msg = format!("failed to process UID {uid_str}: {e}");
                tracing::warn!("{err_msg}");
                result.errors.push(err_msg);
            }
        }
    }

    session.logout().ok();
    Ok(result)
}

/// Fetch a single email by UID, parse it, dedup, and create a vault note.
fn fetch_and_process_email<T>(
    context: &StorageContext,
    session: &mut imap::Session<T>,
    account_id: &str,
    uid: u32,
) -> Result<bool>
where
    T: std::io::Read + std::io::Write,
{
    let fetch = session
        .uid_fetch(format!("{uid}"), "BODY[]")
        .context("UID FETCH failed")?;

    let Some(msg) = fetch.iter().next() else {
        return Ok(false);
    };

    let body = msg.body().context("email body is empty")?;

    let parsed = mailparse::parse_mail(body).context("failed to parse MIME email")?;

    // Extract Message-ID for dedup
    let message_id = get_header_first(&parsed, "Message-ID").unwrap_or_default();
    let message_id_clean = message_id.trim_matches(|c| c == '<' || c == '>' || c == ' ');

    // Dedup
    if !message_id_clean.is_empty() && email_exists(context, account_id, message_id_clean)? {
        return Ok(false);
    }

    let subject = get_header_first(&parsed, "Subject").unwrap_or_default();
    let from_addr = get_header_first(&parsed, "From").unwrap_or_default();
    let to_addrs = get_header_first(&parsed, "To").unwrap_or_default();
    let cc_addrs = get_header_first(&parsed, "Cc").unwrap_or_default();
    let date_str = get_header_first(&parsed, "Date").unwrap_or_default();

    // Extract text body
    let body_text = extract_text_body(&parsed);

    // Create a vault note from the email
    let note_title = if subject.is_empty() {
        format!("Email from {}", from_addr)
    } else {
        subject.to_string()
    };

    let note_body = format!(
        "---\nid: \"\"\ntitle: \"{title}\"\nsummary: \"Email imported from {from_addr}\"\ntags: [email, imported]\nsource: \"email:{account_id}\"\n---\n\n# {title}\n\n**From:** {from_addr}\n**To:** {to_addrs}\n**Cc:** {cc_addrs}\n**Date:** {date}\n**Message-ID:** {msg_id}\n\n---\n\n{body}",
        title = note_title,
        from_addr = from_addr,
        to_addrs = to_addrs,
        cc_addrs = cc_addrs,
        date = date_str,
        msg_id = message_id_clean,
        body = body_text,
        account_id = account_id,
    );

    let note_doc = crate::models::NoteDocument {
        meta: crate::models::NoteMeta {
            title: note_title.clone(),
            summary: format!("Email imported from {from_addr}"),
            tags: vec!["email".to_string(), "imported".to_string()],
            source: format!("email:{account_id}"),
            ..Default::default()
        },
        body: note_body,
        search_snippet: None,
    };

    let saved_note = crate::storage::notes::save_note_with_context(context, note_doc)?;

    // Store email record
    store_email(
        context,
        account_id,
        message_id_clean,
        &subject,
        &from_addr,
        &to_addrs,
        &cc_addrs,
        &date_str,
        &body_text,
        &saved_note.meta.id,
    )?;

    Ok(true)
}

/// Extract the first value of a header (decoded).
fn get_header_first(parsed: &mailparse::ParsedMail, name: &str) -> Option<String> {
    parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case(name))
        .and_then(|h| {
            let val = h.get_value();
            if val.is_empty() {
                None
            } else {
                Some(val)
            }
        })
}

/// Extract plain text from a MIME email (prefer text/plain, fallback to text/html stripped).
fn extract_text_body(parsed: &mailparse::ParsedMail) -> String {
    // Try direct text/plain first
    let content_type = parsed.ctype.mimetype.to_lowercase();
    if content_type == "text/plain" {
        if let Ok(body) = parsed.get_body() {
            return body;
        }
    }

    // Try multipart — find first text/plain or text/html part
    if parsed
        .ctype
        .mimetype
        .eq_ignore_ascii_case("multipart/alternative")
        || parsed
            .ctype
            .mimetype
            .eq_ignore_ascii_case("multipart/mixed")
    {
        return extract_from_parts(&parsed.subparts);
    }

    // Try subparts directly
    if !parsed.subparts.is_empty() {
        return extract_from_parts(&parsed.subparts);
    }

    // Last resort: raw body
    parsed.get_body().unwrap_or_default()
}

/// Recursively extract text from MIME parts.
fn extract_from_parts(parts: &[mailparse::ParsedMail]) -> String {
    for part in parts {
        let ct = part.ctype.mimetype.to_lowercase();
        if ct == "text/plain" {
            if let Ok(body) = part.get_body() {
                return body;
            }
        }
    }
    // Fallback to first text/html
    for part in parts {
        let ct = part.ctype.mimetype.to_lowercase();
        if ct == "text/html" || ct.starts_with("text/") {
            if let Ok(body) = part.get_body() {
                return strip_html_tags(&body);
            }
        }
    }
    // Recurse into subparts
    for part in parts {
        if !part.subparts.is_empty() {
            let result = extract_from_parts(&part.subparts);
            if !result.is_empty() {
                return result;
            }
        }
    }
    String::new()
}

/// Minimal HTML tag stripper for email body extraction.
fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_entity = false;
    let mut entity_buf = String::new();

    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
            }
            '>' if in_tag => {
                in_tag = false;
            }
            '&' if !in_tag => {
                in_entity = true;
                entity_buf.clear();
            }
            ';' if in_entity => {
                in_entity = false;
                let decoded = match entity_buf.as_str() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "nbsp" => " ",
                    _ => "",
                };
                out.push_str(decoded);
            }
            _ => {
                if in_tag {
                    continue;
                }
                if in_entity {
                    entity_buf.push(c);
                } else {
                    // Collapse multiple whitespace into single space
                    if c.is_whitespace() && out.ends_with(' ') {
                        continue;
                    }
                    if c.is_whitespace() {
                        out.push(' ');
                    } else {
                        out.push(c);
                    }
                }
            }
        }
    }
    out.trim().to_string()
}

// ─── Schema ───────────────────────────────────────────────────────

/// DDL statements for mail-related tables. Called from `ensure_schema`.
pub(crate) const MAIL_SCHEMA_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS mail_accounts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    username TEXT NOT NULL,
    password TEXT NOT NULL,
    use_tls INTEGER NOT NULL DEFAULT 1,
    sync_enabled INTEGER NOT NULL DEFAULT 1,
    sync_frequency_minutes INTEGER NOT NULL DEFAULT 30,
    last_sync_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS emails (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    subject TEXT NOT NULL,
    from_addr TEXT NOT NULL,
    to_addrs TEXT NOT NULL DEFAULT '',
    cc_addrs TEXT NOT NULL DEFAULT '',
    date TEXT NOT NULL DEFAULT '',
    body_text TEXT NOT NULL DEFAULT '',
    note_id TEXT NOT NULL DEFAULT '',
    imported_at TEXT NOT NULL,
    FOREIGN KEY(account_id) REFERENCES mail_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_emails_account_id ON emails(account_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_emails_message_id ON emails(account_id, message_id);
CREATE INDEX IF NOT EXISTS idx_emails_date ON emails(date DESC);
CREATE INDEX IF NOT EXISTS idx_emails_subject ON emails(subject);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_context() -> StorageContext {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-mail-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        // Ensure schema (includes mail tables via ensure_schema)
        crate::storage::initialize_storage_with_context(&ctx).expect("init storage");
        ctx
    }

    #[test]
    fn test_add_and_list_mail_accounts() {
        let ctx = setup_test_context();

        let account = add_mail_account(
            &ctx,
            "Test Gmail",
            "imap.gmail.com",
            993,
            "user@gmail.com",
            "app-password",
            true,
            30,
        )
        .unwrap();
        assert_eq!(account.name, "Test Gmail");
        assert_eq!(account.password, "app-password");

        let accounts = list_mail_accounts(&ctx).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].host, "imap.gmail.com");
        assert_eq!(accounts[0].password, "app-password");
    }

    #[test]
    fn test_delete_mail_account() {
        let ctx = setup_test_context();
        let account =
            add_mail_account(&ctx, "Del Test", "imap.test.com", 993, "u", "p", true, 30).unwrap();
        assert!(delete_mail_account(&ctx, &account.id).unwrap());
        assert!(!delete_mail_account(&ctx, "nonexistent").unwrap());
    }

    #[test]
    fn test_email_dedup() {
        let ctx = setup_test_context();
        let account =
            add_mail_account(&ctx, "Dedup Test", "imap.test.com", 993, "u", "p", true, 30).unwrap();

        assert!(!email_exists(&ctx, &account.id, "<msg1@test>").unwrap());

        store_email(
            &ctx,
            &account.id,
            "<msg1@test>",
            "Subject",
            "from@test.com",
            "to@test.com",
            "",
            "2026-01-01",
            "body",
            "note-1",
        )
        .unwrap();

        assert!(email_exists(&ctx, &account.id, "<msg1@test>").unwrap());
    }

    #[test]
    fn test_search_emails() {
        let ctx = setup_test_context();
        let account = add_mail_account(
            &ctx,
            "Search Test",
            "imap.test.com",
            993,
            "u",
            "p",
            true,
            30,
        )
        .unwrap();

        store_email(
            &ctx,
            &account.id,
            "<msg1>",
            "Hello World",
            "alice@test.com",
            "bob@test.com",
            "",
            "2026-01-01",
            "This is the body",
            "note-1",
        )
        .unwrap();
        store_email(
            &ctx,
            &account.id,
            "<msg2>",
            "Meeting Tomorrow",
            "carol@test.com",
            "bob@test.com",
            "",
            "2026-01-02",
            "Agenda attached",
            "note-2",
        )
        .unwrap();

        let results = search_emails(&ctx, "Hello", 10, 0).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].subject, "Hello World");

        let results = search_emails(&ctx, "test.com", 10, 0).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_get_email() {
        let ctx = setup_test_context();
        let account =
            add_mail_account(&ctx, "Get Test", "imap.test.com", 993, "u", "p", true, 30).unwrap();

        let stored = store_email(
            &ctx,
            &account.id,
            "<msg-get>",
            "Get Test",
            "f@t.com",
            "t@t.com",
            "",
            "2026-01-01",
            "body text",
            "note-id",
        )
        .unwrap();

        let fetched = get_email(&ctx, &stored.id).unwrap().expect("should exist");
        assert_eq!(fetched.subject, "Get Test");
        assert_eq!(fetched.note_id, "note-id");
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(strip_html_tags("<p>Hello</p>"), "Hello");
        assert_eq!(strip_html_tags("Hello &amp; World"), "Hello & World");
        assert_eq!(
            strip_html_tags("<div><b>Bold</b> and <i>italic</i></div>"),
            "Bold and italic"
        );
        assert_eq!(strip_html_tags("Plain text"), "Plain text");
        assert_eq!(strip_html_tags(""), "");
        assert_eq!(strip_html_tags("&lt;tag&gt;"), "<tag>");
    }

    #[test]
    fn test_account_round_trip_preserves_fields() {
        let ctx = setup_test_context();
        let account = add_mail_account(
            &ctx,
            "Round Trip",
            "imap.example.com",
            143,
            "user@example.com",
            "secret!",
            false,
            60,
        )
        .unwrap();

        let fetched = get_mail_account(&ctx, &account.id)
            .unwrap()
            .expect("should exist");
        assert_eq!(fetched.name, "Round Trip");
        assert_eq!(fetched.host, "imap.example.com");
        assert_eq!(fetched.port, 143);
        assert_eq!(fetched.username, "user@example.com");
        assert_eq!(fetched.password, "secret!");
        assert!(!fetched.use_tls);
        assert!(fetched.sync_enabled);
        assert_eq!(fetched.sync_frequency_minutes, 60);
    }
}
