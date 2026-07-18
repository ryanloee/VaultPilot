//! Feed CRUD — RSS/Atom/JSON Feed subscription persistence (#3041).
//!
//! Feeds are external syndication sources (blogs, news, podcasts) polled
//! periodically. New entries are converted to Markdown and stored as vault
//! notes, reusing the Web Clipper conversion pipeline
//! (`src/bin/vaultpilot-cli/http_bridge.rs` `html_to_markdown`).
//!
//! Incremental fetching is supported via ETag / If-Modified-Since and a
//! per-feed "high-water mark" of the most recent seen entry (id + date), so
//! re-polls only ingest genuinely new items.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use tracing::instrument;
use uuid::Uuid;

use crate::models::FeedSubscription;

use super::pool::open_connection;
use super::StorageContext;

// ────────────────────────────────────────────────────────
// Feed CRUD
// ────────────────────────────────────────────────────────

/// Create a new feed subscription. Returns the created feed.
#[instrument(skip(context))]
pub fn create_feed_with_context(
    context: &StorageContext,
    url: &str,
    title: &str,
    kind: &str,
    collection: &str,
    tags: &str,
    interval_minutes: i64,
) -> Result<FeedSubscription> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    let kind = if kind.is_empty() { "rss" } else { kind };
    let interval = if interval_minutes <= 0 {
        60
    } else {
        interval_minutes
    };

    connection.execute(
        "INSERT INTO feeds (id, title, url, kind, collection, tags, interval_minutes, enabled, last_fetched_at, etag, last_modified, last_entry_id, last_entry_date, last_status, last_error, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, '', '', '', '', '', '', '', ?8, ?9)",
        params![id, title, url, kind, collection, tags, interval, now, now],
    ).with_context(|| format!("failed to create feed '{url}'"))?;

    Ok(FeedSubscription {
        id,
        title: title.to_string(),
        url: url.to_string(),
        kind: kind.to_string(),
        collection: collection.to_string(),
        tags: tags.to_string(),
        interval_minutes: interval,
        enabled: true,
        last_fetched_at: String::new(),
        etag: String::new(),
        last_modified: String::new(),
        last_entry_id: String::new(),
        last_entry_date: String::new(),
        last_status: String::new(),
        last_error: String::new(),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Delete a feed by ID. Returns `true` if a row was deleted.
#[instrument(skip(context))]
pub fn delete_feed_with_context(context: &StorageContext, id: &str) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let rows = connection
        .execute("DELETE FROM feeds WHERE id = ?1", params![id])
        .with_context(|| format!("failed to delete feed '{id}'"))?;
    Ok(rows > 0)
}

/// List all feeds, most recently created first.
#[instrument(skip(context))]
pub fn list_feeds_with_context(context: &StorageContext) -> Result<Vec<FeedSubscription>> {
    let (connection, _) = open_connection(context)?;
    let mut stmt = connection.prepare(
        r#"
        SELECT id, title, url, kind, collection, tags, interval_minutes,
               enabled, last_fetched_at, etag, last_modified,
               last_entry_id, last_entry_date, last_status, last_error,
               created_at, updated_at
        FROM feeds
        ORDER BY created_at DESC
        "#,
    )?;
    let feeds = stmt
        .query_map([], |row| Ok(read_feed_row(row)))?
        .filter_map(|r| r.ok())
        .collect::<Vec<FeedSubscription>>();
    Ok(feeds)
}

/// Get a single feed by ID.
#[instrument(skip(context))]
pub fn get_feed_with_context(
    context: &StorageContext,
    id: &str,
) -> Result<Option<FeedSubscription>> {
    let (connection, _) = open_connection(context)?;
    let feed = connection
        .query_row(
            r#"
            SELECT id, title, url, kind, collection, tags, interval_minutes,
                   enabled, last_fetched_at, etag, last_modified,
                   last_entry_id, last_entry_date, last_status, last_error,
                   created_at, updated_at
            FROM feeds WHERE id = ?1
            "#,
            params![id],
            |row| Ok(read_feed_row(row)),
        )
        .optional()
        .with_context(|| format!("failed to get feed '{id}'"))?;
    Ok(feed)
}

/// Update mutable fields of a feed.
#[instrument(skip(context))]
#[allow(clippy::too_many_arguments)]
pub fn update_feed_with_context(
    context: &StorageContext,
    id: &str,
    title: &str,
    kind: &str,
    collection: &str,
    tags: &str,
    interval_minutes: i64,
    enabled: bool,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let rows = connection.execute(
        "UPDATE feeds SET title = ?1, kind = ?2, collection = ?3, tags = ?4, interval_minutes = ?5, enabled = ?6, updated_at = ?7 WHERE id = ?8",
        params![title, kind, collection, tags, interval_minutes, enabled as i64, now, id],
    )?;
    Ok(rows > 0)
}

/// Enable or disable a feed.
#[instrument(skip(context))]
pub fn set_feed_enabled_with_context(
    context: &StorageContext,
    id: &str,
    enabled: bool,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let rows = connection.execute(
        "UPDATE feeds SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
        params![enabled as i64, now, id],
    )?;
    Ok(rows > 0)
}

/// Record the result of a poll: high-water marks, status, error, and the
/// conditional-request headers to send next time (ETag / Last-Modified).
#[instrument(skip(context))]
#[allow(clippy::too_many_arguments)]
pub fn update_feed_fetch_result_with_context(
    context: &StorageContext,
    id: &str,
    etag: &str,
    last_modified: &str,
    last_entry_id: &str,
    last_entry_date: &str,
    status: &str,
    error: &str,
) -> Result<bool> {
    let (connection, _) = open_connection(context)?;
    let now = Utc::now().to_rfc3339();
    let rows = connection.execute(
        "UPDATE feeds SET etag = ?1, last_modified = ?2, last_entry_id = ?3, last_entry_date = ?4, last_fetched_at = ?5, last_status = ?6, last_error = ?7, updated_at = ?8 WHERE id = ?9",
        params![etag, last_modified, last_entry_id, last_entry_date, now, status, error, now, id],
    )?;
    Ok(rows > 0)
}

/// Map a `rusqlite` row of the `feeds` table into [`FeedSubscription`].
fn read_feed_row(row: &rusqlite::Row<'_>) -> FeedSubscription {
    FeedSubscription {
        id: row.get(0).unwrap_or_default(),
        title: row.get(1).unwrap_or_default(),
        url: row.get(2).unwrap_or_default(),
        kind: row.get(3).unwrap_or_else(|_| "rss".to_string()),
        collection: row.get(4).unwrap_or_default(),
        tags: row.get(5).unwrap_or_default(),
        interval_minutes: row.get(6).unwrap_or(60),
        enabled: row.get::<_, i64>(7).unwrap_or(1) != 0,
        last_fetched_at: row.get(8).unwrap_or_default(),
        etag: row.get(9).unwrap_or_default(),
        last_modified: row.get(10).unwrap_or_default(),
        last_entry_id: row.get(11).unwrap_or_default(),
        last_entry_date: row.get(12).unwrap_or_default(),
        last_status: row.get(13).unwrap_or_default(),
        last_error: row.get(14).unwrap_or_default(),
        created_at: row.get(15).unwrap_or_default(),
        updated_at: row.get(16).unwrap_or_default(),
    }
}

// ────────────────────────────────────────────────────────
// OPML import / export (#3041)
//
// OPML (Outline Processor Markup Language) is the de-facto interchange
// format for feed readers. We support a minimal, well-formed subset:
//   <opml version="2.0">
//     <head><title>...</title></head>
//     <body>
//       <outline type="rss" text="Title" title="Title"
//                xmlUrl="https://.../feed" htmlUrl="https://..."/>
//       <outline text="Group"> ... nested outlines ... </outline>
//     </body>
//   </opml>
// Nested (folder) outlines are flattened — the folder name is not stored
// as a separate entity but its child feeds are imported. This keeps the
// model (a flat feed list) intact while still round-tripping OPML files
// produced by other readers.
// ────────────────────────────────────────────────────────

/// A single parsed OPML outline entry (a feed subscription).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpmlFeed {
    /// Feed title (from `text`/`title` attribute, or derived from xmlUrl).
    pub title: String,
    /// The feed URL (`xmlUrl` attribute).
    pub xml_url: String,
    /// Optional site URL (`htmlUrl` attribute).
    pub html_url: String,
    /// `rss` | `atom` | `json` when the `type` attribute is present.
    pub kind: String,
}

/// Parse OPML content into a list of feeds.
///
/// Tolerant of the common variations: `text` vs `title`, missing `type`,
/// and nested folder outlines. Returns an error only when the document is
/// not recognizable XML/OPML at all.
pub fn parse_opml(content: &str) -> Result<Vec<OpmlFeed>, String> {
    let mut feeds = Vec::new();
    // Naive tag scanner — sufficient for OPML which is a small, regular
    // vocabulary. We don't pull in a full XML parser to keep the dependency
    // surface minimal and the logic easy to unit-test.
    let bytes = content.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Find the next `<outline`
        if let Some(pos) = find_substring(bytes, b"<outline", i) {
            i = pos;
            // Find the matching `>` for this tag.
            let tag_end = match find_from(bytes, b'>', i) {
                Some(e) => e,
                None => break,
            };
            let tag = &bytes[i..=tag_end];
            let tag_str = String::from_utf8_lossy(tag);
            // Only consider outlines that carry an `xmlUrl` (real feeds);
            // folder outlines (no xmlUrl) are containers we descend into.
            if let Some(xml_url) = attr_value(&tag_str, "xmlUrl") {
                let text = attr_value(&tag_str, "text")
                    .or_else(|| attr_value(&tag_str, "title"))
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| xml_url.clone());
                let html_url = attr_value(&tag_str, "htmlUrl").unwrap_or_default();
                let kind = attr_value(&tag_str, "type")
                    .map(|t| t.to_lowercase())
                    .unwrap_or_else(|| infer_kind(&xml_url));
                feeds.push(OpmlFeed {
                    title: xml_unescape(&text),
                    xml_url: xml_unescape(&xml_url),
                    html_url: xml_unescape(&html_url),
                    kind,
                });
            }
            i = tag_end + 1;
        } else {
            break;
        }
    }
    if feeds.is_empty()
        && !content.trim().is_empty()
        && !content.contains("<opml")
        && !content.contains("<outline")
    {
        return Err("not a recognizable OPML document".to_string());
    }
    Ok(feeds)
}

/// Serialize a list of feeds to OPML 2.0.
pub fn export_opml(title: &str, feeds: &[FeedSubscription]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<opml version=\"2.0\">\n");
    out.push_str("  <head>\n");
    out.push_str(&format!("    <title>{}</title>\n", xml_escape(title)));
    out.push_str("  </head>\n");
    out.push_str("  <body>\n");
    for feed in feeds {
        let kind = if feed.kind.is_empty() {
            "rss"
        } else {
            feed.kind.as_str()
        };
        out.push_str(&format!(
            "    <outline type=\"{kind}\" text=\"{}\" title=\"{}\" xmlUrl=\"{}\" htmlUrl=\"{}\"/>\n",
            xml_escape(&feed.title),
            xml_escape(&feed.title),
            xml_escape(&feed.url),
            xml_escape(&feed.url),
        ));
    }
    out.push_str("  </body>\n");
    out.push_str("</opml>\n");
    out
}

/// Convert parsed OPML feeds into `FeedSubscription` rows. `interval_minutes`
/// and `collection` come from the caller (OPML has no notion of them); the
/// feed title/url/kind are taken from the OPML entry.
pub fn opml_feeds_to_subscriptions(
    feeds: &[OpmlFeed],
    collection: &str,
    tags: &str,
    interval_minutes: i64,
) -> Vec<(String, String, String, String, String, i64)> {
    feeds
        .iter()
        .map(|f| {
            (
                f.xml_url.clone(),
                f.title.clone(),
                if f.kind.is_empty() {
                    infer_kind(&f.xml_url)
                } else {
                    f.kind.clone()
                },
                collection.to_string(),
                tags.to_string(),
                interval_minutes,
            )
        })
        .collect()
}

fn infer_kind(url: &str) -> String {
    if url.contains("json") {
        "json".to_string()
    } else if url.contains("atom") {
        "atom".to_string()
    } else {
        "rss".to_string()
    }
}

fn find_substring(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    let tail = &haystack[from..];
    tail.windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

fn find_from(haystack: &[u8], needle: u8, from: usize) -> Option<usize> {
    haystack[from..]
        .iter()
        .position(|&b| b == needle)
        .map(|p| from + p)
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    // Match `name="..."` or `name='...'`. The attribute name must be preceded
    // by whitespace or a tag boundary (`<`), so that a name appearing *inside
    // another attribute's value* (e.g. `xmlUrl="...?htmlUrl=1"`) is not
    // mistaken for a real attribute. See issue #3072.
    let pat = format!("{name}=");
    let mut search_from = 0;
    while let Some(idx) = tag[search_from..].find(&pat) {
        let abs = search_from + idx;
        let preceded_by_boundary = abs == 0
            || tag[..abs]
                .chars()
                .last()
                .map(|c| c.is_whitespace() || c == '<')
                == Some(true);
        if preceded_by_boundary {
            let rest = &tag[abs + pat.len()..];
            let quote = rest.chars().next()?;
            if quote != '"' && quote != '\'' {
                return None;
            }
            let close = rest[1..].find(quote)?;
            return Some(rest[1..1 + close].to_string());
        }
        // Keep scanning past this false-positive match.
        search_from = abs + pat.len();
    }
    None
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}

// ────────────────────────────────────────────────────────
// Async wrappers (mirror subscription.rs pattern)
// ────────────────────────────────────────────────────────

#[instrument(skip(ctx))]
pub async fn create_feed_async(
    ctx: &StorageContext,
    url: String,
    title: String,
    kind: String,
    collection: String,
    tags: String,
    interval_minutes: i64,
) -> Result<FeedSubscription> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        create_feed_with_context(
            &ctx,
            &url,
            &title,
            &kind,
            &collection,
            &tags,
            interval_minutes,
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

#[instrument(skip(ctx))]
pub async fn delete_feed_async(ctx: &StorageContext, id: String) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || delete_feed_with_context(&ctx, &id))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

#[instrument(skip(ctx))]
pub async fn list_feeds_async(ctx: &StorageContext) -> Result<Vec<FeedSubscription>> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || list_feeds_with_context(&ctx))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

#[instrument(skip(ctx))]
pub async fn get_feed_async(ctx: &StorageContext, id: String) -> Result<Option<FeedSubscription>> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || get_feed_with_context(&ctx, &id))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

#[instrument(skip(ctx))]
#[allow(clippy::too_many_arguments)]
pub async fn update_feed_async(
    ctx: &StorageContext,
    id: String,
    title: String,
    kind: String,
    collection: String,
    tags: String,
    interval_minutes: i64,
    enabled: bool,
) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        update_feed_with_context(
            &ctx,
            &id,
            &title,
            &kind,
            &collection,
            &tags,
            interval_minutes,
            enabled,
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

#[instrument(skip(ctx))]
pub async fn set_feed_enabled_async(
    ctx: &StorageContext,
    id: String,
    enabled: bool,
) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || set_feed_enabled_with_context(&ctx, &id, enabled))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

#[instrument(skip(ctx))]
#[allow(clippy::too_many_arguments)]
pub async fn update_feed_fetch_result_async(
    ctx: &StorageContext,
    id: String,
    etag: String,
    last_modified: String,
    last_entry_id: String,
    last_entry_date: String,
    status: String,
    error: String,
) -> Result<bool> {
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || {
        update_feed_fetch_result_with_context(
            &ctx,
            &id,
            &etag,
            &last_modified,
            &last_entry_id,
            &last_entry_date,
            &status,
            &error,
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::initialize_storage_with_context;

    fn setup_temp_context() -> (std::path::PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-feeds-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_test(&temp);
        (temp, ctx)
    }

    #[test]
    fn test_create_and_list_feeds() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let feed = create_feed_with_context(
            &ctx,
            "https://example.com/feed.xml",
            "Example Blog",
            "rss",
            "News",
            "rss,news",
            30,
        )
        .unwrap();
        assert_eq!(feed.url, "https://example.com/feed.xml");
        assert!(!feed.id.is_empty());
        assert!(feed.enabled);
        assert_eq!(feed.interval_minutes, 30);

        // Default interval is clamped for non-positive values.
        let slow =
            create_feed_with_context(&ctx, "https://slow.example/feed", "Slow", "", "", "", 0)
                .unwrap();
        assert_eq!(slow.kind, "rss");
        assert_eq!(slow.interval_minutes, 60);

        let feeds = list_feeds_with_context(&ctx).unwrap();
        assert_eq!(feeds.len(), 2);
    }

    #[test]
    fn test_update_and_delete_feed() {
        let (_temp, ctx) = setup_temp_context();
        initialize_storage_with_context(&ctx).unwrap();

        let feed = create_feed_with_context(
            &ctx,
            "https://example.com/feed.xml",
            "Example",
            "atom",
            "News",
            "rss",
            60,
        )
        .unwrap();

        assert!(update_feed_with_context(
            &ctx, &feed.id, "Renamed", "rss", "Tech", "tech", 120, false,
        )
        .unwrap());

        let got = get_feed_with_context(&ctx, &feed.id).unwrap().unwrap();
        assert_eq!(got.title, "Renamed");
        assert_eq!(got.kind, "rss");
        assert_eq!(got.collection, "Tech");
        assert_eq!(got.interval_minutes, 120);
        assert!(!got.enabled);

        assert!(update_feed_fetch_result_with_context(
            &ctx,
            &feed.id,
            "etag-abc",
            "Wed, 02 Oct 2002 13:00:00 GMT",
            "entry-1",
            "2002-10-02T13:00:00Z",
            "success",
            ""
        )
        .unwrap());
        let got2 = get_feed_with_context(&ctx, &feed.id).unwrap().unwrap();
        assert_eq!(got2.etag, "etag-abc");
        assert_eq!(got2.last_entry_id, "entry-1");
        assert_eq!(got2.last_status, "success");

        assert!(delete_feed_with_context(&ctx, &feed.id).unwrap());
        assert!(get_feed_with_context(&ctx, &feed.id).unwrap().is_none());
    }

    #[test]
    fn test_opml_round_trip() {
        let opml = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head><title>My Feeds</title></head>
  <body>
    <outline text="Blog &amp; Co" title="Blog &amp; Co" type="rss" xmlUrl="https://blog.example/feed.xml" htmlUrl="https://blog.example"/>
    <outline text="News Folder">
      <outline type="atom" xmlUrl="https://news.example/atom" htmlUrl="https://news.example"/>
    </outline>
    <outline xmlUrl="https://json.example/feed.json"/>
  </body>
</opml>"#;

        let parsed = parse_opml(opml).expect("parse failed");
        // Folder outline has no xmlUrl → skipped; 3 real feeds remain.
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].title, "Blog & Co");
        assert_eq!(parsed[0].xml_url, "https://blog.example/feed.xml");
        assert_eq!(parsed[0].kind, "rss");
        assert_eq!(parsed[1].xml_url, "https://news.example/atom");
        assert_eq!(parsed[1].kind, "atom");
        // Third has no type → inferred from url (json).
        assert_eq!(parsed[2].xml_url, "https://json.example/feed.json");
        assert_eq!(parsed[2].kind, "json");

        // Export a couple of feeds and re-parse to confirm round-trip shape.
        let subs = vec![FeedSubscription {
            id: "1".into(),
            title: "Exported & <Test>".into(),
            url: "https://export.example/feed".into(),
            kind: "rss".into(),
            collection: String::new(),
            tags: String::new(),
            interval_minutes: 60,
            enabled: true,
            last_fetched_at: String::new(),
            etag: String::new(),
            last_modified: String::new(),
            last_entry_id: String::new(),
            last_entry_date: String::new(),
            last_status: String::new(),
            last_error: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }];
        let exported = export_opml("VaultPilot", &subs);
        assert!(exported.contains("<opml version=\"2.0\">"));
        assert!(exported.contains("xmlUrl=\"https://export.example/feed\""));
        // XML escaping must protect the title.
        assert!(exported.contains("Exported &amp; &lt;Test&gt;"));
        let re = parse_opml(&exported).expect("re-parse failed");
        assert_eq!(re.len(), 1);
        assert_eq!(re[0].title, "Exported & <Test>");
    }

    #[test]
    fn test_opml_rejects_garbage() {
        assert!(parse_opml("just some random text").is_err());
        assert!(parse_opml("").unwrap().is_empty());
    }

    #[test]
    fn test_opml_attr_value_ignores_substring_in_other_attr() {
        // Regression test for #3072: an attribute name appearing *inside*
        // another attribute's value (e.g. a query string containing
        // `?htmlUrl=1`) must not be mistaken for a real attribute.
        let tag = r#"<outline text="Blog" xmlUrl="https://real.example.com/feed?htmlUrl=1" htmlUrl="https://real.example.com"/>"#;

        // `htmlUrl` is a real attribute and must resolve correctly.
        let html = attr_value(tag, "htmlUrl");
        assert_eq!(html.as_deref(), Some("https://real.example.com"));

        // `xmlUrl` is also a real attribute and must not be confused.
        let xml = attr_value(tag, "xmlUrl");
        assert_eq!(
            xml.as_deref(),
            Some("https://real.example.com/feed?htmlUrl=1")
        );

        // A name that only appears inside a value (not as a real attr) is None.
        assert_eq!(attr_value(tag, "type"), None);
    }

    #[test]
    fn test_parse_opml_does_not_drop_attr_in_query_string() {
        // End-to-end: parse an OPML where one feed's xmlUrl contains another
        // attribute name in its query string. The htmlUrl must survive.
        let opml = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <body>
    <outline text="Blog" xmlUrl="https://real.example.com/feed?htmlUrl=1" htmlUrl="https://real.example.com"/>
  </body>
</opml>"#;
        let parsed = parse_opml(opml).expect("parse failed");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].html_url, "https://real.example.com");
        assert_eq!(parsed[0].xml_url, "https://real.example.com/feed?htmlUrl=1");
    }
}
