//! Issue #4067: `/pair/accept` success response omitted fields required by
//! the initiator's `PeerDevice` (notably `addedAt`), so `complete_pairing`
//! always failed with "error decoding response body". Only the acceptor ever
//! recorded the pairing; every later authenticated sync call from the
//! initiator was rejected 403 — pairing looked successful on one side and
//! broken on the other.
//!
//! Contract: `pair_accept_success_json` must deserialize into `PeerDevice`
//! for every version of the client (serde skips unknown keys on older
//! clients, so extra fields are wire-compatible both ways).

#[cfg(test)]
mod tests {
    use crate::sync::{
        acceptor_peer_from_request, pair_accept_success_json, pair_request_body, LocalIdentity,
        PeerDevice,
    };

    fn sample_identity() -> LocalIdentity {
        LocalIdentity {
            device_id: "0f4c2a9e-1111-2222-3333-444455556666".into(),
            hostname: "desktop-pc".into(),
            platform: "windows".into(),
            token: "tok-1234".into(),
        }
    }

    #[test]
    fn pair_accept_response_decodes_as_peer_device() {
        let body = pair_accept_success_json(&sample_identity());
        let peer: PeerDevice =
            serde_json::from_value(body).expect("initiator must decode /pair/accept response");
        assert_eq!(peer.device_id, "0f4c2a9e-1111-2222-3333-444455556666");
        assert_eq!(peer.hostname, "desktop-pc");
        assert_eq!(peer.platform, "windows");
        assert_eq!(peer.token, "tok-1234");
        assert!(peer.ip.is_none());
        assert!(!peer.added_at.is_empty(), "addedAt must be present");
        assert!(peer.last_sync_at.is_none());
    }

    #[test]
    fn legacy_minimal_body_without_added_at_fails_decoding() {
        // Documents the original defect: the pre-fix response body could not
        // satisfy `PeerDevice`, which is what broke phone-side pairing.
        let legacy = serde_json::json!({
            "deviceId": "0f4c2a9e-1111-2222-3333-444455556666",
            "hostname": "desktop-pc",
            "platform": "windows",
            "token": "tok-1234",
        });
        let decoded: Result<PeerDevice, _> = serde_json::from_value(legacy);
        assert!(
            decoded.is_err(),
            "body without addedAt must NOT decode (root cause of #4067)"
        );
    }

    #[test]
    fn pair_request_body_is_accepted_by_both_naming_conventions() {
        // The initiator's body must satisfy BOTH the new camelCase
        // `PairAcceptBody` and the legacy snake_case one (pre-rename peers),
        // otherwise desktop-initiated pairing 422s against old phones.
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[allow(dead_code)]
        struct NewStyle {
            device_id: String,
            hostname: String,
            platform: String,
            token: String,
        }
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct LegacySnakeStyle {
            device_id: String,
            hostname: String,
            platform: String,
            token: String,
        }

        let body = pair_request_body(&sample_identity());
        let new_style: NewStyle =
            serde_json::from_value(body.clone()).expect("new-style peer must decode request");
        assert_eq!(new_style.device_id, sample_identity().device_id);
        let legacy: LegacySnakeStyle =
            serde_json::from_value(body).expect("legacy snake_case peer must decode request");
        assert_eq!(legacy.device_id, sample_identity().device_id);
    }

    #[test]
    fn acceptor_records_initiator_ip_without_port() {
        // Follow-up: the acceptor used to hardcode ip: None, so its「同步」
        // button always failed with 该设备无 IP 记录 even right after a
        // successful pairing. The TCP peer address must be recorded, sans port.
        let addr: std::net::SocketAddr = "192.168.1.50:51234".parse().unwrap();
        let peer = acceptor_peer_from_request("dev-1", "phone", "android", "tok", Some(addr));
        assert_eq!(peer.ip.as_deref(), Some("192.168.1.50"));
        assert_eq!(peer.device_id, "dev-1");
        assert_eq!(peer.hostname, "phone");

        // Unknown address still yields None (no crash / no empty string).
        let no_addr = acceptor_peer_from_request("dev-1", "phone", "android", "tok", None);
        assert!(no_addr.ip.is_none());
    }
}
