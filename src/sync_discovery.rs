//! LAN Sync Discovery — read-only device probe.
//!
//! `discover_device` probes a user-supplied IP's well-known port (37421) to
//! find other VaultPilot instances on the LAN. The matching HTTP server (which
//! answers `GET /hello` and the authenticated `/pair/*` + `/sync/*` endpoints)
//! now lives in the [`crate::sync`] module's `start_sync_server`.
//!
//! Security: the discovery endpoint is read-only (no vault data exposed
//! beyond a note count and hostname). Data transfer requires pairing first.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Well-known port for VaultPilot LAN discovery (and the sync server).
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

/// Best-effort local IPv4 — the source address the OS would use for outbound
/// traffic (via a throwaway UDP connect that never sends a packet).
fn local_ipv4() -> Option<std::net::Ipv4Addr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    match sock.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) => Some(v4),
        _ => None,
    }
}

/// Scan the local /24 subnet for other VaultPilot instances.
///
/// Probes every host on the same /24 as the local interface (skipping self)
/// concurrently and returns those that answer `/hello`. This backs the
/// "leave the search box empty → browse the LAN" UX.
pub async fn scan_lan() -> Vec<(String, DeviceInfo)> {
    let Some(ip) = local_ipv4() else {
        return Vec::new();
    };
    let octets = ip.octets();
    // Loopback / odd subnets: nothing to scan.
    if octets[0] == 127 {
        return Vec::new();
    }
    let prefix = format!("{}.{}.{}.", octets[0], octets[1], octets[2]);
    let mut handles = Vec::new();
    for last in 1..=254u8 {
        if last == octets[3] {
            continue;
        }
        let target = format!("{prefix}{last}");
        handles.push(tokio::task::spawn(async move {
            match discover_device(&target).await {
                Ok(Some(info)) => Some((target, info)),
                _ => None,
            }
        }));
    }
    let mut out = Vec::new();
    for h in handles {
        if let Ok(Some(pair)) = h.await {
            out.push(pair);
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
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
