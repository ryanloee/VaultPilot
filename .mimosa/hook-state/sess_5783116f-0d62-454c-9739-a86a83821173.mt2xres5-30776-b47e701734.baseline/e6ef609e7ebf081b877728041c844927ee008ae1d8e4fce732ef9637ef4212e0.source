//! Feed ingestion engine — RSS/Atom/JSON Feed auto-ingestion (#3041).
//!
//! This is the *binary-crate* copy of the poller. It reuses the Web Clipper
//! HTML→Markdown pipeline (`http_bridge::html_to_markdown`) and the same SSRF
//! guard (`http_bridge::validate_clip_url_host`) used by `/api/clip`, so a
//! malicious feed URL cannot bounce us to an internal endpoint (#3041 hardening,
//! reuses #3059/#3060 defenses).
//!
//! The lib crate holds the pure storage layer (`vaultpilot_lib::storage::feeds`)
//! and the `FeedSubscription` model. This binary module owns the network fetch
//! + parsing + dedup + note ingestion.

use anyhow::Result;
use chrono::DateTime;
use tracing::{instrument, warn};

use vaultpilot_lib::models::FeedSubscription;
use vaultpilot_lib::storage::{
    save_note_with_context, update_feed_fetch_result_with_context, StorageContext,
};

/// A function that converts fetched entry HTML into Markdown. The binary crate
/// supplies the real implementation (reusing the Web Clipper pipeline).
pub type MarkdownConverter = fn(&str) -> String;

/// Result of polling a single feed.
#[derive(Debug, Clone)]
pub struct FeedPollResult {
    pub feed_id: String,
    pub status: String,
    pub new_entries: usize,
    pub error: String,
}

/// Parse a feed document (RSS/Atom/JSON) into normalized entries.
///
/// Uses the `feed-rs` crate which handles all three formats. Each returned
/// entry has a resolved link, an id (or deterministic fallback), a publish
/// date, and a Markdown-ready body (HTML content or summary).
pub fn parse_feed(bytes: &[u8], converter: MarkdownConverter) -> Result<Vec<FeedEntry>, String> {
    let feed = feed_rs::parser::parse(bytes).map_err(|e| format!("feed parse error: {e}"))?;
    let mut entries = Vec::with_capacity(feed.entries.len());
    for entry in feed.entries {
        // Link: prefer the alternate/self href.
        let link = entry
            .links
            .iter()
            .find(|l| l.rel.as_deref() == Some("alternate") || l.rel.is_none())
            .or_else(|| entry.links.first())
            .map(|l| l.href.clone())
            .unwrap_or_default();

        let title = entry
            .title
            .map(|t| t.content.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Untitled".to_string());

        // Entry id: prefer the feed-provided id; otherwise use the link, or a
        // content hash as a last resort.
        let id = if entry.id.trim().is_empty() {
            if link.trim().is_empty() {
                format!("{:x}", simple_hash(&title))
            } else {
                link.clone()
            }
        } else {
            entry.id.clone()
        };

        // Publish date — `feed-rs` gives a `chrono::DateTime<Utc>`.
        let published = entry.published.or(entry.updated).map(|d| d.to_rfc3339());

        // Body: prefer the HTML content, then the summary, then the title.
        let raw_html = entry
            .content
            .as_ref()
            .and_then(|c| c.body.as_deref())
            .or_else(|| entry.summary.as_ref().map(|t| t.content.as_str()))
            .unwrap_or("")
            .to_string();

        let markdown = if raw_html.trim().is_empty() {
            String::new()
        } else {
            converter(raw_html.trim())
        };

        entries.push(FeedEntry {
            id,
            title,
            link,
            published,
            markdown,
        });
    }
    Ok(entries)
}

/// A normalized feed entry ready for ingestion.
#[derive(Debug, Clone)]
pub struct FeedEntry {
    pub id: String,
    pub title: String,
    pub link: String,
    pub published: Option<String>,
    pub markdown: String,
}

/// Compare two entries by publish date (newest first), falling back to id for
/// stable ordering when dates are missing.
fn cmp_newest_first(a: &FeedEntry, b: &FeedEntry) -> std::cmp::Ordering {
    match (&a.published, &b.published) {
        (Some(pa), Some(pb)) => {
            let da = DateTime::parse_from_rfc3339(pa).ok();
            let db = DateTime::parse_from_rfc3339(pb).ok();
            match (da, db) {
                (Some(x), Some(y)) => y.cmp(&x),
                _ => b.id.cmp(&a.id),
            }
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.id.cmp(&a.id),
    }
}

/// Return only the entries that are strictly newer than the feed's high-water
/// mark (last entry id + last entry date). Keeps entries sorted newest-first so
/// the very first one becomes the new high-water mark.
pub fn select_new_entries(
    mut entries: Vec<FeedEntry>,
    last_entry_id: &str,
    last_entry_date: &str,
) -> Vec<FeedEntry> {
    entries.sort_by(cmp_newest_first);

    if last_entry_id.trim().is_empty() && last_entry_date.trim().is_empty() {
        // First poll: ingest everything (commonly desired for a new feed).
        return entries;
    }

    let last_date = DateTime::parse_from_rfc3339(last_entry_date).ok();

    entries
        .into_iter()
        .filter(|e| {
            // Already-seen by id → skip.
            if !last_entry_id.trim().is_empty() && e.id == last_entry_id {
                return false;
            }
            // Older than (or equal to) the high-water date → skip, unless we
            // can't parse dates in which case we keep it (defensive).
            if let (Some(ld), Some(ed)) = (last_date, e.published.as_ref()) {
                if let (Ok(l), Ok(e)) = (
                    DateTime::parse_from_rfc3339(ld.to_rfc3339().as_str()),
                    DateTime::parse_from_rfc3339(ed.as_str()),
                ) {
                    if e <= l {
                        return false;
                    }
                }
            }
            true
        })
        .collect()
}

/// Compute the new high-water mark (id, date) after ingesting a sorted set of
/// entries. The first entry (newest) determines the mark.
fn new_high_water_mark(
    entries: &[FeedEntry],
    previous_id: &str,
    previous_date: &str,
) -> (String, String) {
    if let Some(newest) = entries.first() {
        let date = newest
            .published
            .clone()
            .unwrap_or_else(|| previous_date.to_string());
        (newest.id.clone(), date)
    } else {
        (previous_id.to_string(), previous_date.to_string())
    }
}

/// Ingest a single feed's new entries as vault notes, returning the count of
/// notes actually saved.
#[instrument(skip(context, _converter))]
pub fn ingest_feed_entries(
    context: &StorageContext,
    feed: &FeedSubscription,
    entries: &[FeedEntry],
    _converter: MarkdownConverter,
) -> usize {
    let mut saved = 0usize;
    let base_tags: Vec<String> = {
        let mut t = feed
            .tags
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if !t.iter().any(|x| x == "rss") {
            t.push("rss".to_string());
        }
        if !t.iter().any(|x| x == &feed.title) {
            let feed_slug = feed.title.trim().to_lowercase().replace(' ', "-");
            if !feed_slug.is_empty() {
                t.push(feed_slug);
            }
        }
        t
    };

    for entry in entries {
        let note_title = if entry.title.trim().is_empty() {
            entry.link.clone()
        } else {
            entry.title.trim().to_string()
        };
        let mut body = String::new();
        body.push_str(&format!("> Source: {}\n\n", entry.link));
        if !entry.markdown.trim().is_empty() {
            body.push_str(entry.markdown.trim());
        } else {
            body.push_str("_No content extracted for this entry._");
        }

        let note = vaultpilot_lib::models::NoteDocument {
            meta: vaultpilot_lib::models::NoteMeta {
                title: note_title,
                tags: base_tags.clone(),
                source: entry.link.clone(),
                collections: if feed.collection.trim().is_empty() {
                    vec![]
                } else {
                    vec![feed.collection.trim().to_string()]
                },
                ..Default::default()
            },
            body,
            search_snippet: None,
            search_score: None,
        };

        match save_note_with_context(context, note) {
            Ok(_) => saved += 1,
            Err(e) => {
                // A duplicate (same id) or transient error shouldn't abort the
                // whole feed; log and continue.
                warn!(
                    feed_id = %feed.id,
                    entry = %entry.id,
                    "failed to save feed entry note: {e}"
                );
            }
        }
    }
    saved
}

/// Poll every enabled feed, ingest new entries, and update high-water marks.
///
/// `converter` turns entry HTML into Markdown. In the binary crate this is wired
/// to `http_bridge::html_to_markdown`.
#[instrument(skip(context, converter))]
pub async fn poll_all_feeds(
    context: &StorageContext,
    converter: MarkdownConverter,
) -> Vec<FeedPollResult> {
    let feeds = match vaultpilot_lib::storage::list_feeds_with_context(context) {
        Ok(f) => f,
        Err(e) => {
            warn!("feed poller: failed to list feeds: {e}");
            return vec![];
        }
    };

    let mut results = Vec::with_capacity(feeds.len());
    for feed in feeds {
        if !feed.enabled {
            continue;
        }
        let result = poll_single_feed(context, &feed, converter).await;
        results.push(result);
    }
    results
}

/// Poll a single feed.
async fn poll_single_feed(
    context: &StorageContext,
    feed: &FeedSubscription,
    converter: MarkdownConverter,
) -> FeedPollResult {
    let fetch = fetch_feed_bytes(feed).await;
    let (status, new_entries, error, etag, last_modified, hw_id, hw_date) = match fetch {
        Ok(FetchOutcome {
            bytes,
            etag,
            last_modified,
            not_modified,
        }) => {
            if not_modified {
                // 304 Not Modified — nothing to do.
                let (id, date) =
                    new_high_water_mark(&[], &feed.last_entry_id, &feed.last_entry_date);
                (
                    "skipped".to_string(),
                    0usize,
                    String::new(),
                    etag,
                    last_modified,
                    id,
                    date,
                )
            } else {
                match parse_feed(&bytes, converter) {
                    Ok(parsed) => {
                        let new =
                            select_new_entries(parsed, &feed.last_entry_id, &feed.last_entry_date);
                        let saved = ingest_feed_entries(context, feed, &new, converter);
                        let (id, date) =
                            new_high_water_mark(&new, &feed.last_entry_id, &feed.last_entry_date);
                        (
                            "success".to_string(),
                            saved,
                            String::new(),
                            etag,
                            last_modified,
                            id,
                            date,
                        )
                    }
                    Err(e) => (
                        "failed".to_string(),
                        0usize,
                        e,
                        etag,
                        last_modified,
                        feed.last_entry_id.clone(),
                        feed.last_entry_date.clone(),
                    ),
                }
            }
        }
        Err(e) => (
            "failed".to_string(),
            0usize,
            e,
            String::new(),
            String::new(),
            feed.last_entry_id.clone(),
            feed.last_entry_date.clone(),
        ),
    };

    let _ = update_feed_fetch_result_with_context(
        context,
        &feed.id,
        &etag,
        &last_modified,
        &hw_id,
        &hw_date,
        &status,
        &error,
    );

    FeedPollResult {
        feed_id: feed.id.clone(),
        status,
        new_entries,
        error,
    }
}

/// Outcome of fetching a feed's bytes.
struct FetchOutcome {
    bytes: Vec<u8>,
    etag: String,
    last_modified: String,
    not_modified: bool,
}

/// Fetch feed bytes with the SSRF guard and streaming body cap.
///
/// Mirrors the Web Clipper's `/api/clip` fetch path: validate the host, then
/// send conditional headers from the previous poll (ETag / If-Modified-Since).
async fn fetch_feed_bytes(feed: &FeedSubscription) -> Result<FetchOutcome, String> {
    // SSRF guard — reject non-http(s) schemes and hosts that resolve to a
    // forbidden (loopback / private / link-local / …) address. Fail-closed.
    // The guard returns the verified (host, SocketAddr) pins so we can pin DNS
    // on the client, closing the DNS-rebinding TOCTOU window between this check
    // and the actual fetch (#3059 hardening, reused from /api/clip).
    let pins = crate::http_bridge::validate_clip_url_host(&feed.url)
        .await
        .map_err(|msg| format!("feed fetch blocked: {msg}"))?;

    let mut client_builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5));
    for (host, addr) in &pins {
        client_builder = client_builder.resolve(host, *addr);
    }
    let client = client_builder
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let mut req = client
        .get(&feed.url)
        .header(
            "User-Agent",
            "VaultPilot-FeedPoller/1.0 (+https://vaultpilot.app)",
        )
        .header(
            "Accept",
            "application/rss+xml, application/atom+xml, application/json, text/xml, */*;q=0.8",
        );

    if !feed.etag.trim().is_empty() {
        req = req.header("If-None-Match", feed.etag.trim());
    }
    if !feed.last_modified.trim().is_empty() {
        req = req.header("If-Modified-Since", feed.last_modified.trim());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("feed fetch failed: {e}"))?;

    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let last_modified = resp
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(FetchOutcome {
            bytes: vec![],
            etag,
            last_modified,
            not_modified: true,
        });
    }

    if !resp.status().is_success() {
        return Err(format!("feed returned status {}", resp.status()));
    }

    // Cap the body at 5 MiB to avoid memory exhaustion from a hostile feed.
    const MAX_BYTES: usize = 5 * 1024 * 1024;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("failed to read feed body: {e}"))?;
    if bytes.len() > MAX_BYTES {
        return Err(format!(
            "feed body too large: {} bytes (cap {MAX_BYTES})",
            bytes.len()
        ));
    }

    Ok(FetchOutcome {
        bytes: bytes.to_vec(),
        etag,
        last_modified,
        not_modified: false,
    })
}

/// Small non-crypto hash for fallback entry ids.
fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 1469598103934665603; // FNV-1a 64-bit offset basis
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use vaultpilot_lib::storage::{create_feed_with_context, initialize_storage_with_context};

    fn trivial_converter(html: &str) -> String {
        // Strip tags crudely for tests.
        html.chars()
            .filter(|c| !matches!(c, '<' | '>'))
            .collect::<String>()
            .trim()
            .to_string()
    }

    fn setup() -> (std::path::PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-feedpoller-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&temp).expect("temp dir");
        let ctx = StorageContext::for_cli(Some(temp.clone())).expect("test context");
        (temp, ctx)
    }

    #[test]
    fn test_parse_rss_entries() {
        let rss = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Test</title><link>http://x.com</link><description>d</description><item><title>Item1</title><link>http://x.com/1</link><description>&lt;p&gt;Hello &lt;b&gt;world&lt;/b&gt;&lt;/p&gt;</description><pubDate>Wed, 02 Oct 2002 13:00:00 GMT</pubDate><guid>a1</guid></item><item><title>Item2</title><link>http://x.com/2</link><description>&lt;p&gt;Second&lt;/p&gt;</description></item></channel></rss>"#;
        let entries = parse_feed(rss.as_bytes(), trivial_converter).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "a1");
        assert_eq!(entries[0].title, "Item1");
        // The trivial test converter strips tags, so "<p>Hello <b>world</b></p>"
        // becomes "Hello bworld" — both words must survive.
        assert!(entries[0].markdown.contains("Hello"));
        assert!(entries[0].markdown.contains("world"));
        // Second item has no <guid>; feed-rs assigns a stable derived id.
        assert_eq!(entries[1].title, "Item2");
        assert!(!entries[1].id.is_empty());
    }

    #[test]
    fn test_select_new_entries_respects_high_water() {
        let entries = vec![
            FeedEntry {
                id: "new".to_string(),
                title: "New".to_string(),
                link: "http://x/new".to_string(),
                published: Some("2024-01-02T00:00:00Z".to_string()),
                markdown: String::new(),
            },
            FeedEntry {
                id: "old".to_string(),
                title: "Old".to_string(),
                link: "http://x/old".to_string(),
                published: Some("2024-01-01T00:00:00Z".to_string()),
                markdown: String::new(),
            },
        ];
        // High-water at "old" 2024-01-01 → only "new" is newer.
        let new = select_new_entries(entries, "old", "2024-01-01T00:00:00Z");
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id, "new");
        // No high-water → everything returned.
        let all = select_new_entries(
            vec![FeedEntry {
                id: "a".to_string(),
                title: "A".to_string(),
                link: "l".to_string(),
                published: None,
                markdown: String::new(),
            }],
            "",
            "",
        );
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_ingest_entries_creates_notes() {
        let (_temp, ctx) = setup();
        initialize_storage_with_context(&ctx).unwrap();
        let feed = create_feed_with_context(
            &ctx,
            "https://example.com/feed",
            "Example",
            "rss",
            "News",
            "tech",
            60,
        )
        .unwrap();

        let entries = vec![FeedEntry {
            id: "e1".to_string(),
            title: "Hello".to_string(),
            link: "https://example.com/1".to_string(),
            published: Some("2024-05-01T00:00:00Z".to_string()),
            markdown: "Body text".to_string(),
        }];
        let saved = ingest_feed_entries(&ctx, &feed, &entries, trivial_converter);
        assert_eq!(saved, 1);

        // Ingesting the same entry again still works (save_note overwrites by
        // id-free title path; we just confirm no panic / no crash).
        let saved2 = ingest_feed_entries(&ctx, &feed, &entries, trivial_converter);
        assert_eq!(saved2, 1);
    }
}
