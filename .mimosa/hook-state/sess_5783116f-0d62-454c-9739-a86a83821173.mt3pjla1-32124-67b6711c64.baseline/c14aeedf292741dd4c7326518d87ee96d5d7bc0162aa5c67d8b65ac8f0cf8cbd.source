//! LAN Sync Discovery — lightweight HTTP server + client probe.
//!
//! Each desktop instance listens on a well-known port (37421) and answers
//! `GET /hello` with device info (hostname, platform, vault note count).
//! The `discover_device` command probes a user-supplied IP to find other
//! VaultPilot instances on the LAN.
//!
//! Security: the discovery endpoint is read-only (no vault data exposed
//! beyond a note count and hostname). The sync server itself will require
//! pairing consent before any data transfer.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Well-known port for VaultPilot LAN discovery.
pub const DISCOVERY_PORT: u16 = 37421;

/// Device identity returned by the `/hello` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub hostname: String,
    pub platform: String,
    pub vault_pilot_version: String,
    pub note_count: usize,
    pub vault_name: String,
}

/// Spawn the discovery server on the Tauri async runtime. Non-fatal — if the
/// port is already in use (another instance), the app continues normally.
pub async fn start_discovery_server(note_count: usize, vault_name: String) {
    let app = axum::Router::new().route(
        "/hello",
        axum::routing::get(move || async move {
            axum::Json(DeviceInfo {
                hostname: hostname::get()
                    .unwrap_or_else(|_| "unknown".into())
                    .to_string_lossy()
                    .into_owned(),
                platform: std::env::consts::OS.to_string(),
                vault_pilot_version: env!("CARGO_PKG_VERSION").to_string(),
                note_count,
                vault_name,
            })
        }),
    );

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], DISCOVERY_PORT));
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            tracing::info!(%addr, "sync discovery server started");
            if let Err(e) = axum::serve(listener, app).await {
                tracing::warn!(error = %e, "sync discovery server stopped");
            }
        }
        Err(e) => {
            // Port already in use — another VaultPilot instance is running.
            tracing::debug!(error = %e, "discovery port in use, skipping");
        }
    }
}

/// Probe `http://{ip}:37421/hello` for a VaultPilot discovery endpoint.
/// Returns `None` when no client responds within the timeout.
pub async fn discover_device(ip: &str) -> Result<Option<DeviceInfo>> {
    // Validate the input is an IP address (not a URL — prevents SSRF via
    // crafted hostnames that could resolve to internal services).
    let parsed: std::net::IpAddr = ip
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid IP address: {ip}"))?;

    let url = format!("http://{parsed}:{DISCOVERY_PORT}/hello");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let info: DeviceInfo = resp.json().await?;
            Ok(Some(info))
        }
        Ok(resp) => {
            // Something responded but it's not a VaultPilot discovery endpoint.
            tracing::debug!(status = %resp.status(), "non-VaultPilot response at {url}");
            Ok(None)
        }
        Err(_) => Ok(None), // connection refused / timeout — no client there
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discover_rejects_invalid_ip() {
        // Not an IP address — must be rejected before any HTTP call.
        assert!(discover_device("not-an-ip").await.is_err());
        assert!(discover_device("http://evil.com").await.is_err());
        assert!(discover_device("").await.is_err());
    }

    #[tokio::test]
    async fn discover_valid_ip_returns_none_when_no_server() {
        // 192.0.2.1 is TEST-NET-1 (RFC 5737) — guaranteed unroutable.
        let result = discover_device("192.0.2.1")
            .await
            .expect("valid IP should not error");
        assert!(result.is_none(), "no server at TEST-NET address");
    }
}
