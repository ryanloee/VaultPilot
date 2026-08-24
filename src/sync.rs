//! Real bidirectional LAN sync — device pairing + file-level vault sync.
//!
//! Builds on the discovery port (37421). The same axum server that answers
//! `GET /hello` also serves authenticated pairing and sync endpoints:
//!
//! * `POST /pair/accept?code=...` — completes a PIN-based pairing handshake.
//! * `GET  /sync/manifest`       — list the peer's vault files (sha256 + mtime).
//! * `GET  /sync/file?path=...`  — fetch one vault file (base64).
//! * `PUT  /sync/file?path=...`  — write one vault file.
//!
//! All `/sync/*` and `/pair/accept` requests are authorized against the local
//! set of paired devices. The vault is treated as the source of truth — a
//! Markdown folder — so sync moves files, and the receiver rebuilds its search
//! index afterwards (the existing `rebuild_index` command).

use anyhow::{anyhow, Context, Result};
use axum::response::IntoResponse;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Same well-known port the discovery server uses.
pub const SYNC_PORT: u16 = crate::sync_discovery::DISCOVERY_PORT;

/// Vault sub-directories that must never be synced (they hold local metadata,
/// the SQLite index and secrets — not user content).
const EXCLUDED_DIRS: &[&str] = &[".vaultpilot", ".git", "node_modules"];

/// Pairing request validity window.
const PAIR_TTL: Duration = Duration::from_secs(300);

// ── Public data types ──────────────────────────────────────────────────────

/// A device this instance has paired with.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerDevice {
    pub device_id: String,
    pub hostname: String,
    pub platform: String,
    /// Shared secret the peer uses to authenticate *to us*.
    pub token: String,
    /// Last known network address (set by the side that initiated pairing).
    pub ip: Option<String>,
    pub added_at: String,
    pub last_sync_at: Option<String>,
}

/// This instance's stable identity, persisted once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalIdentity {
    pub device_id: String,
    pub hostname: String,
    pub platform: String,
    pub token: String,
}

/// One entry in a vault manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    /// Vault-relative path (forward slashes).
    pub path: String,
    pub sha256: String,
    pub mtime_ms: i64,
}

/// Outcome of a sync run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub pulled: usize,
    pub pushed: usize,
    pub conflicts: usize,
    pub errors: Vec<String>,
}

// ── Process-wide sync state ─────────────────────────────────────────────────

struct PendingPair {
    expires_at: SystemTime,
}

struct SyncState {
    vault_dir: PathBuf,
    peers_path: PathBuf,
    identity_path: PathBuf,
    peers: Mutex<Vec<PeerDevice>>,
    pending: Mutex<HashMap<String, PendingPair>>,
}

static SYNC_STATE: OnceLock<SyncState> = OnceLock::new();

fn state() -> Option<&'static SyncState> {
    SYNC_STATE.get()
}

/// Initialize the global sync state. Must be called once at startup (before
/// the discovery/sync server is spawned).
pub fn init_sync_state(vault_dir: PathBuf, config_dir: PathBuf) {
    let peers_path = config_dir.join("sync_peers.json");
    let state = SyncState {
        vault_dir,
        peers_path: peers_path.clone(),
        identity_path: config_dir.join("sync_identity.json"),
        peers: Mutex::new(load_peers(&peers_path)),
        pending: Mutex::new(HashMap::new()),
    };
    let _ = SYNC_STATE.set(state);
}

// ── Identity ────────────────────────────────────────────────────────────────

/// Return this device's identity, generating and persisting it on first use.
pub fn get_identity() -> Result<LocalIdentity> {
    let st = state().ok_or_else(|| anyhow!("sync state not initialized"))?;
    if let Ok(s) = std::fs::read_to_string(&st.identity_path) {
        if let Ok(id) = serde_json::from_str::<LocalIdentity>(&s) {
            return Ok(id);
        }
    }
    let id = LocalIdentity {
        device_id: uuid::Uuid::new_v4().to_string(),
        hostname: hostname::get()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "unknown".into()),
        platform: std::env::consts::OS.to_string(),
        token: uuid::Uuid::new_v4().to_string(),
    };
    if let Some(parent) = st.identity_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&st.identity_path, serde_json::to_string_pretty(&id)?)
        .context("persist local identity")?;
    Ok(id)
}

// ── Pairing (PIN handshake) ─────────────────────────────────────────────────

/// Generate a 6-character pairing code. The *acceptor* calls this, shows the
/// code to the human, and waits for the *initiator* to complete pairing with
/// it within [`PAIR_TTL`].
pub fn generate_pair_code() -> Result<String> {
    let code = {
        let u = uuid::Uuid::new_v4().simple().to_string();
        u[..6.min(u.len())].to_uppercase()
    };
    let mut pending = state()
        .ok_or_else(|| anyhow!("sync state not initialized"))?
        .pending
        .lock()
        .unwrap();
    pending.insert(
        code.clone(),
        PendingPair {
            expires_at: SystemTime::now() + PAIR_TTL,
        },
    );
    Ok(code)
}

/// Called by the *initiator* device. Contacts the remote, hands over its
/// identity, and on success records the remote as a paired peer.
pub async fn complete_pairing(remote_ip: &str, pair_code: &str) -> Result<PeerDevice> {
    let my_id = get_identity()?;
    let url = format!("http://{remote_ip}:{SYNC_PORT}/pair/accept?code={pair_code}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "deviceId": my_id.device_id,
            "hostname": my_id.hostname,
            "platform": my_id.platform,
            "token": my_id.token,
        }))
        .send()
        .await
        .map_err(|e| anyhow!("配对请求失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("配对被拒绝：配对码无效或已过期"));
    }
    let remote: PeerDevice = resp.json().await?;
    let peer = PeerDevice {
        device_id: remote.device_id,
        hostname: remote.hostname,
        platform: remote.platform,
        token: remote.token,
        ip: Some(remote_ip.to_string()),
        added_at: now_rfc3339(),
        last_sync_at: None,
    };
    add_peer(peer.clone())?;
    Ok(peer)
}

/// List paired devices.
pub fn list_peers() -> Vec<PeerDevice> {
    state()
        .map(|s| s.peers.lock().unwrap().clone())
        .unwrap_or_default()
}

/// Remove a paired device by id.
pub fn remove_peer(device_id: &str) -> Result<()> {
    let st = state().ok_or_else(|| anyhow!("sync state not initialized"))?;
    let mut peers = st.peers.lock().unwrap();
    peers.retain(|p| p.device_id != device_id);
    save_peers(&st.peers_path, &peers)
}

// ── Sync engine ─────────────────────────────────────────────────────────────

/// Bidirectionally sync the local vault with `peer` reachable at `remote_ip`.
///
/// `mode` is `"full"` (sync the entire vault) or `"selected"` (only paths that
/// start with one of the `includes` folder/file prefixes).
pub async fn sync_with_peer(
    remote_ip: &str,
    peer: &PeerDevice,
    mode: &str,
    includes: &[String],
) -> Result<SyncResult> {
    let my_id = get_identity()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let base = format!("http://{remote_ip}:{SYNC_PORT}");

    let remote_manifest: Vec<ManifestEntry> = client
        .get(format!("{base}/sync/manifest"))
        .query(&[("device", my_id.device_id.as_str()), ("token", my_id.token.as_str())])
        .send()
        .await
        .map_err(|e| anyhow!("获取对方清单失败：{e}"))?
        .error_for_status()
        .map_err(|e| anyhow!("获取对方清单被拒绝：{e}"))?
        .json()
        .await?;

    let st = state().ok_or_else(|| anyhow!("sync state not initialized"))?;
    let vault_dir = st.vault_dir.clone();
    let local = scan_manifest(&vault_dir);

    let local_map: HashMap<&str, &ManifestEntry> =
        local.iter().map(|e| (e.path.as_str(), e)).collect();
    let remote_map: HashMap<&str, &ManifestEntry> =
        remote_manifest.iter().map(|e| (e.path.as_str(), e)).collect();

    // Selective-sync filter: when `mode == "selected"` and `includes` is
    // non-empty, keep only paths equal to or nested under one of the prefixes.
    let keep = |p: &str| -> bool {
        if mode != "selected" || includes.is_empty() {
            return true;
        }
        includes
            .iter()
            .any(|inc| p == inc || p.starts_with(&format!("{inc}/")))
    };

    let mut result = SyncResult::default();

    // Pull from remote what we lack or that changed remotely.
    for r in &remote_manifest {
        if !keep(&r.path) {
            continue;
        }
        match local_map.get(r.path.as_str()) {
            None => {
                if let Ok(data) = get_remote_file(&client, &base, &my_id, &r.path).await {
                    let dest = match safe_join(&vault_dir, &r.path) {
                        Some(p) => p,
                        None => {
                            result.errors.push(format!("跳过非法路径：{}", r.path));
                            continue;
                        }
                    };
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if std::fs::write(&dest, &data).is_ok() {
                        result.pulled += 1;
                    } else {
                        result.errors.push(format!("写入失败：{}", r.path));
                    }
                } else {
                    result.errors.push(format!("拉取失败：{}", r.path));
                }
            }
            Some(l) if l.sha256 != r.sha256 => {
                // Conflict: both sides changed. Keep local; store remote as a
                // conflict copy, and push local to remote as a conflict copy so
                // neither side silently loses data.
                if let Ok(data) = get_remote_file(&client, &base, &my_id, &r.path).await {
                    let cp = conflict_path(&vault_dir, &r.path, &peer.hostname);
                    if let Some(parent) = cp.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if std::fs::write(&cp, &data).is_ok() {
                        // Read the local (unchanged) copy to also push it to the
                        // remote as a conflict copy. safe_join guards against any
                        // path-escape attempt even though the remote manifest is
                        // already validated server-side.
                        if let Some(local_src) = safe_join(&vault_dir, &r.path) {
                            if let Ok(local_data) = std::fs::read(&local_src) {
                                let _ = put_remote_file(
                                    &client,
                                    &base,
                                    &my_id,
                                    &conflict_rel(&r.path, &my_id.hostname),
                                    &local_data,
                                )
                                .await;
                            }
                        }
                        result.conflicts += 1;
                    }
                } else {
                    result.errors.push(format!("冲突拉取失败：{}", r.path));
                }
            }
            Some(_) => {}
        }
    }

    // Push to remote what we have but they don't.
    for l in &local {
        if !keep(&l.path) {
            continue;
        }
        if !remote_map.contains_key(l.path.as_str()) {
            if let Some(src) = safe_join(&vault_dir, &l.path) {
                if let Ok(data) = std::fs::read(&src) {
                    if put_remote_file(&client, &base, &my_id, &l.path, &data)
                        .await
                        .is_ok()
                    {
                        result.pushed += 1;
                    } else {
                        result.errors.push(format!("推送失败：{}", l.path));
                    }
                }
            }
        }
    }

    update_peer_last_sync(&peer.device_id);
    Ok(result)
}

// ── Vault scanning & hashing ────────────────────────────────────────────────

/// Recursively scan the vault, returning a manifest that excludes metadata
/// directories. Relative paths use forward slashes.
pub fn scan_manifest(vault_dir: &Path) -> Vec<ManifestEntry> {
    let mut out = Vec::new();
    scan_dir(vault_dir, vault_dir, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn scan_dir(root: &Path, dir: &Path, out: &mut Vec<ManifestEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            let name = entry.file_name();
            if EXCLUDED_DIRS.iter().any(|ex| name.to_string_lossy() == *ex) {
                continue;
            }
            scan_dir(root, &path, out);
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if rel.is_empty() {
                continue;
            }
            match std::fs::read(&path) {
                Ok(data) => out.push(ManifestEntry {
                    path: rel,
                    sha256: sha256_hex(&data),
                    mtime_ms: mtime_ms(&path),
                }),
                Err(_) => continue,
            }
        }
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let d = h.finalize();
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn mtime_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Join a vault-relative path, refusing anything that escapes the vault
/// (e.g. `../`, absolute paths, or embedded `..`).
fn safe_join(vault_dir: &Path, rel: &str) -> Option<PathBuf> {
    if rel.contains("..") || rel.starts_with('/') {
        return None;
    }
    let joined = vault_dir.join(rel);
    if joined.starts_with(vault_dir) {
        Some(joined)
    } else {
        None
    }
}

/// Local path for a conflict copy of `rel` suffixed by `suffix`.
fn conflict_path(vault_dir: &Path, rel: &str, suffix: &str) -> PathBuf {
    let p = vault_dir.join(rel);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = p.parent().unwrap_or(vault_dir);
    parent.join(format!("{stem}.conflict-{suffix}{ext}"))
}

/// Vault-relative path for a conflict copy (used when pushing to remote).
fn conflict_rel(rel: &str, suffix: &str) -> String {
    let p = Path::new(rel);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = p
        .parent()
        .map(|pp| pp.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = format!("{stem}.conflict-{suffix}{ext}");
    if parent.is_empty() {
        name
    } else {
        format!("{parent}/{name}")
    }
}

// ── Peer persistence ────────────────────────────────────────────────────────

fn load_peers(path: &Path) -> Vec<PeerDevice> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_peers(path: &Path, peers: &[PeerDevice]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(peers)?)?;
    Ok(())
}

fn add_peer(peer: PeerDevice) -> Result<()> {
    let st = state().ok_or_else(|| anyhow!("sync state not initialized"))?;
    let mut peers = st.peers.lock().unwrap();
    peers.retain(|p| p.device_id != peer.device_id);
    peers.push(peer);
    save_peers(&st.peers_path, &peers)
}

fn update_peer_last_sync(device_id: &str) {
    let st = match state() {
        Some(s) => s,
        None => return,
    };
    let mut peers = st.peers.lock().unwrap();
    if let Some(p) = peers.iter_mut().find(|p| p.device_id == device_id) {
        p.last_sync_at = Some(now_rfc3339());
    }
    let _ = save_peers(&st.peers_path, &peers);
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── Remote file transfer helpers ────────────────────────────────────────────

async fn get_remote_file(
    client: &reqwest::Client,
    base: &str,
    id: &LocalIdentity,
    rel: &str,
) -> Result<Vec<u8>> {
    let resp = client
        .get(format!("{base}/sync/file"))
        .query(&[
            ("device", id.device_id.as_str()),
            ("token", id.token.as_str()),
            ("path", rel),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("拉取文件失败：{rel}"));
    }
    let j: serde_json::Value = resp.json().await?;
    let c = j
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("远程文件无内容：{rel}"))?;
    Ok(base64::engine::general_purpose::STANDARD.decode(c)?)
}

async fn put_remote_file(
    client: &reqwest::Client,
    base: &str,
    id: &LocalIdentity,
    rel: &str,
    data: &[u8],
) -> Result<()> {
    let resp = client
        .put(format!("{base}/sync/file"))
        .query(&[
            ("device", id.device_id.as_str()),
            ("token", id.token.as_str()),
            ("path", rel),
        ])
        .body(data.to_vec())
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("推送文件失败：{rel}"));
    }
    Ok(())
}

// ── HTTP server (mounted on the discovery port) ─────────────────────────────

/// Start the combined discovery + sync server. Non-fatal if the port is taken.
pub async fn start_sync_server(note_count: usize, vault_name: String) {
    use axum::routing::{get, post};

    let app = axum::Router::new()
        .route(
            "/hello",
            get(move || async move {
                axum::Json(crate::sync_discovery::DeviceInfo {
                    hostname: hostname::get()
                        .map(|h| h.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| "unknown".into()),
                    platform: std::env::consts::OS.to_string(),
                    vault_pilot_version: env!("CARGO_PKG_VERSION").to_string(),
                    note_count,
                    vault_name,
                })
            }),
        )
        .route("/pair/accept", post(pair_accept_handler))
        .route("/sync/manifest", get(sync_manifest_handler))
        .route(
            "/sync/file",
            get(sync_file_get_handler).put(sync_file_put_handler),
        );

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], SYNC_PORT));
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            tracing::info!(%addr, "sync server started");
            if let Err(e) = axum::serve(listener, app).await {
                tracing::warn!(error = %e, "sync server stopped");
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "sync port in use, skipping");
        }
    }
}

#[derive(Debug, Deserialize)]
struct PairCodeQuery {
    code: String,
}

#[derive(Debug, Deserialize)]
struct PairAcceptBody {
    device_id: String,
    hostname: String,
    platform: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct AuthQuery {
    device: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct FileQuery {
    device: String,
    token: String,
    path: String,
}

fn authorized(device: &str, token: &str) -> bool {
    match state() {
        Some(st) => st
            .peers
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.device_id == device && p.token == token),
        None => false,
    }
}

fn err_response(code: axum::http::StatusCode, msg: &str) -> axum::response::Response {
    (
        code,
        axum::Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

async fn pair_accept_handler(
    axum::extract::Query(q): axum::extract::Query<PairCodeQuery>,
    axum::extract::Json(body): axum::extract::Json<PairAcceptBody>,
) -> axum::response::Response {
    let st = match state() {
        Some(s) => s,
        None => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "sync not initialized",
            )
        }
    };
    {
        let pending = st.pending.lock().unwrap();
        match pending.get(&q.code) {
            Some(p) if p.expires_at > SystemTime::now() => {}
            _ => {
                return err_response(
                    axum::http::StatusCode::FORBIDDEN,
                    "invalid or expired pair code",
                )
            }
        }
    }
    // The acceptor records the initiator as a peer (it learns the initiator's
    // identity from the request body).
    if let Err(e) = add_peer(PeerDevice {
        device_id: body.device_id.clone(),
        hostname: body.hostname.clone(),
        platform: body.platform.clone(),
        token: body.token.clone(),
        ip: None,
        added_at: now_rfc3339(),
        last_sync_at: None,
    }) {
        return err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    // Return our own identity so the initiator can record us.
    match get_identity() {
        Ok(id) => (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({
                "deviceId": id.device_id,
                "hostname": id.hostname,
                "platform": id.platform,
                "token": id.token,
            })),
        )
            .into_response(),
        Err(e) => err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn sync_manifest_handler(
    axum::extract::Query(q): axum::extract::Query<AuthQuery>,
) -> axum::response::Response {
    if !authorized(&q.device, &q.token) {
        return err_response(axum::http::StatusCode::FORBIDDEN, "unauthorized");
    }
    let st = match state() {
        Some(s) => s,
        None => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "sync not initialized",
            )
        }
    };
    let manifest = scan_manifest(&st.vault_dir);
    (axum::http::StatusCode::OK, axum::Json(manifest)).into_response()
}

async fn sync_file_get_handler(
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
) -> axum::response::Response {
    if !authorized(&q.device, &q.token) {
        return err_response(axum::http::StatusCode::FORBIDDEN, "unauthorized");
    }
    let st = match state() {
        Some(s) => s,
        None => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "sync not initialized",
            )
        }
    };
    let Some(path) = safe_join(&st.vault_dir, &q.path) else {
        return err_response(axum::http::StatusCode::BAD_REQUEST, "invalid path");
    };
    match std::fs::read(&path) {
        Ok(data) => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
            (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({ "content": b64 })),
            )
                .into_response()
        }
        Err(e) => err_response(axum::http::StatusCode::NOT_FOUND, &e.to_string()),
    }
}

async fn sync_file_put_handler(
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    if !authorized(&q.device, &q.token) {
        return err_response(axum::http::StatusCode::FORBIDDEN, "unauthorized");
    }
    let st = match state() {
        Some(s) => s,
        None => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "sync not initialized",
            )
        }
    };
    let Some(path) = safe_join(&st.vault_dir, &q.path) else {
        return err_response(axum::http::StatusCode::BAD_REQUEST, "invalid path");
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, &body[..]) {
        Ok(()) => (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({ "ok": true })),
        )
            .into_response(),
        Err(e) => err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_is_stable() {
        let real = sha256_hex(b"vaultpilot");
        let expected = {
            let mut h = Sha256::new();
            h.update(b"vaultpilot");
            let d = h.finalize();
            d.iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        assert_eq!(real, expected);
        assert_ne!(real, String::new());
    }

    #[test]
    fn safe_join_blocks_escape() {
        let root = Path::new("/vault");
        assert!(safe_join(root, "notes/a.md").is_some());
        assert!(safe_join(root, "../etc/passwd").is_none());
        assert!(safe_join(root, "a/../../b").is_none());
        assert!(safe_join(root, "/abs").is_none());
    }

    #[test]
    fn conflict_rel_preserves_dir_and_ext() {
        assert_eq!(
            conflict_rel("notes/foo.md", "phone"),
            "notes/foo.conflict-phone.md"
        );
        assert_eq!(conflict_rel("bar.txt", "pc"), "bar.conflict-pc.txt");
    }
}
