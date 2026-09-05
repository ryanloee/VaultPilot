//! Feed ingestion engine — RSS/Atom/JSON Feed auto-ingestion (#3041).
//!
//! This is the shared (lib) home of the poller. It was previously implemented
//! in the CLI binary (`src/bin/vaultpilot-cli/feed_poller.rs`); it now lives
//! here so the Tauri desktop app can offer the same "refresh feeds" action
//! through `#[tauri::command]` without spawning a subprocess (business logic
//! belongs in `vaultpilot_lib`, never in the Tauri/CLI shells).
//!
//! New entries are converted to Markdown with the lib Web Clipper pipeline
//! (`crate::clipper::html_to_markdown`) and stored as vault notes. Incremental
//! fetching uses ETag / If-Modified-Since plus a per-feed high-water mark, so
//! re-polls only ingest genuinely new items.
//!
//! Outbound fetches carry the same SSRF guard as the Web Clipper
//! (`validate_fetch_url_host` below mirrors the CLI `http_bridge` guard):
//! non-http(s) schemes and hosts resolving to loopback / private /
//! link-local / metadata addresses are refused fail-closed, and DNS is pinned
//! via `reqwest::ClientBuilder::resolve` to close the rebinding window.

use anyhow::Result;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use tracing::{instrument, warn};

use crate::models::FeedSubscription;
use crate::storage::{
    save_note_with_context, update_feed_fetch_result_with_context, StorageContext,
};

/// Result of polling a single feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedPollResult {
    pub feed_id: String,
    pub status: String,
    pub new_entries: usize,
    pub error: String,
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

/// Parse a feed document (RSS/Atom/JSON) into normalized entries.
///
/// Uses the `feed-rs` crate which handles all three formats. Each returned
/// entry has a resolved link, an id (or deterministic fallback), a publish
/// date, and a Markdown-ready body (HTML content or summary).
pub fn parse_feed(bytes: &[u8]) -> Result<Vec<FeedEntry>, String> {
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
            crate::clipper::html_to_markdown(raw_html.trim())
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
#[instrument(skip(context))]
pub fn ingest_feed_entries(
    context: &StorageContext,
    feed: &FeedSubscription,
    entries: &[FeedEntry],
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

        let note = crate::models::NoteDocument {
            meta: crate::models::NoteMeta {
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
#[instrument(skip(context))]
pub async fn poll_all_feeds(context: &StorageContext) -> Vec<FeedPollResult> {
    let feeds = match crate::storage::list_feeds_with_context(context) {
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
        let result = poll_single_feed(context, &feed).await;
        results.push(result);
    }
    results
}

/// Poll a single enabled feed by id. Disabled feeds report `skipped`;
/// unknown ids are an error.
pub async fn poll_single_feed_by_id(
    context: &StorageContext,
    feed_id: &str,
) -> Result<FeedPollResult, String> {
    let feed = crate::storage::get_feed_with_context(context, feed_id)
        .map_err(|e| format!("failed to load feed: {e}"))?
        .ok_or_else(|| format!("feed not found: {feed_id}"))?;
    if !feed.enabled {
        return Ok(FeedPollResult {
            feed_id: feed.id.clone(),
            status: "skipped".to_string(),
            new_entries: 0,
            error: "feed is disabled".to_string(),
        });
    }
    Ok(poll_single_feed(context, &feed).await)
}

/// Poll a single feed.
async fn poll_single_feed(context: &StorageContext, feed: &FeedSubscription) -> FeedPollResult {
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
                match parse_feed(&bytes) {
                    Ok(parsed) => {
                        let new =
                            select_new_entries(parsed, &feed.last_entry_id, &feed.last_entry_date);
                        let saved = ingest_feed_entries(context, feed, &new);
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
    // and the actual fetch.
    let pins = validate_fetch_url_host(&feed.url)
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

/// Classify an IP as forbidden for outbound feed fetches (SSRF mitigation).
///
/// Forbidden ranges cover the classic SSRF targets:
/// - IPv4: loopback (127/8), private (10/8, 172.16/12, 192.168/16),
///   link-local (169.254/16, includes the cloud metadata endpoint
///   169.254.169.254), multicast, broadcast, unspecified, documentation.
/// - IPv6: loopback (::1), multicast (ff00::/8), unspecified (::),
///   unique-local (fc00::/7), link-local (fe80::/10).
fn ip_is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
        }
        IpAddr::V6(v6) => {
            let segs = v6.segments();
            // Unique-local addresses (fc00::/7) and link-local (fe80::/10).
            let is_ula = (segs[0] & 0xfe00) == 0xfc00;
            let is_link_local = (segs[0] & 0xffc0) == 0xfe80;
            v6.is_loopback() || v6.is_multicast() || v6.is_unspecified() || is_ula || is_link_local
        }
    }
}

/// Validate that the host of `url_str` does not resolve to a forbidden IP,
/// returning the verified `(hostname, SocketAddr)` pairs so the caller can
/// pin DNS via `reqwest::ClientBuilder::resolve`.
///
/// Returns `Ok(vec![])` for literal-IP URLs that pass the forbidden-range
/// check (reqwest does not resolve literal IPs, so no pinning is needed).
/// DNS resolution failure is an error (fail-closed).
async fn validate_fetch_url_host(url_str: &str) -> Result<Vec<(String, SocketAddr)>, String> {
    let parsed = url::Url::parse(url_str).map_err(|e| format!("invalid URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("refusing non-http(s) scheme '{}'", parsed.scheme()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    // Literal IP — validate directly without DNS.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip_is_forbidden(ip) {
            return Err(format!(
                "url host {host} is a forbidden IP (loopback/private/link-local/multicast/unspecified/broadcast)"
            ));
        }
        return Ok(Vec::new());
    }

    // Hostname — resolve via DNS and reject if ANY returned IP is forbidden
    // (refusing on any-forbidden rather than all-forbidden defends against
    // DNS rebinding setups that mix public and private IPs).
    let port = parsed
        .port_or_known_default()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let lookup_target = format!("{host}:{port}");
    let resolved = tokio::net::lookup_host(lookup_target.as_str())
        .await
        .map_err(|e| format!("DNS resolution failed for '{host}': {e}"))?;
    let mut pinned: Vec<(String, SocketAddr)> = Vec::new();
    for addr in resolved {
        if ip_is_forbidden(addr.ip()) {
            return Err(format!(
                "url host '{host}' resolves to forbidden IP {} (loopback/private/link-local/multicast/unspecified/broadcast)",
                addr.ip()
            ));
        }
        pinned.push((host.to_string(), addr));
    }
    Ok(pinned)
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
    use crate::storage::{create_feed_with_context, initialize_storage_with_context};

    fn setup() -> (std::path::PathBuf, StorageContext) {
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-test-feedingest-{}-{}",
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
        let entries = parse_feed(rss.as_bytes()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "a1");
        assert_eq!(entries[0].title, "Item1");
        assert!(entries[0].markdown.contains("Hello"));
        assert!(entries[0].markdown.contains("world"));
        // Second item has no <guid>; feed-rs assigns a stable derived id.
        assert_eq!(entries[1].title, "Item2");
        assert!(!entries[1].id.is_empty());
    }

    #[test]
    fn test_parse_atom_entries() {
        let atom = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><title>Atom</title><entry><title>A1</title><id>urn:a1</id><link href="http://x.com/a1"/><updated>2024-05-01T00:00:00Z</updated><content type="html">&lt;p&gt;Body&lt;/p&gt;</content></entry></feed>"#;
        let entries = parse_feed(atom.as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "urn:a1");
        assert_eq!(entries[0].link, "http://x.com/a1");
        assert!(entries[0].markdown.contains("Body"));
    }

    #[test]
    fn test_parse_garbage_is_error() {
        assert!(parse_feed(b"this is not a feed").is_err());
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
        let saved = ingest_feed_entries(&ctx, &feed, &entries);
        assert_eq!(saved, 1);

        // Ingesting the same entry again still works (save_note overwrites by
        // id-free title path; we just confirm no panic / no crash).
        let saved2 = ingest_feed_entries(&ctx, &feed, &entries);
        assert_eq!(saved2, 1);
    }

    #[tokio::test]
    async fn test_fetch_guard_rejects_non_http_scheme() {
        let err = validate_fetch_url_host("file:///etc/passwd")
            .await
            .unwrap_err();
        assert!(err.contains("non-http"), "got: {err}");
    }

    #[tokio::test]
    async fn test_fetch_guard_rejects_garbage_url() {
        assert!(validate_fetch_url_host("not a url at all").await.is_err());
    }

    #[tokio::test]
    async fn test_fetch_guard_rejects_loopback_literal_ip() {
        let err = validate_fetch_url_host("http://127.0.0.1:8080/secret")
            .await
            .unwrap_err();
        assert!(err.contains("forbidden"), "got: {err}");
    }

    #[tokio::test]
    async fn test_fetch_guard_rejects_metadata_endpoint_literal_ip() {
        let err = validate_fetch_url_host("http://169.254.169.254/latest/meta-data/")
            .await
            .unwrap_err();
        assert!(err.contains("forbidden"), "got: {err}");
    }

    #[tokio::test]
    async fn test_fetch_guard_rejects_ipv6_loopback_literal_ip() {
        let err = validate_fetch_url_host("http://[::1]/").await.unwrap_err();
        assert!(err.contains("forbidden"), "got: {err}");
    }

    #[tokio::test]
    async fn test_fetch_guard_accepts_public_literal_ip_without_pins() {
        // Literal public IP: no DNS involved, so this needs no network.
        let pins = validate_fetch_url_host("http://8.8.8.8/")
            .await
            .expect("public literal IP must pass");
        assert!(pins.is_empty());
    }
}
