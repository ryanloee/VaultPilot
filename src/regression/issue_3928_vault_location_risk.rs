//! Regression test for #3928 — vault location risk detection.
//!
//! Mirrors Anytype JS-9831 / Obsidian 1.13.5: a vault data directory inside
//! a cloud-synced folder (OneDrive/Dropbox/iCloud/…) or on a network file
//! system (UNC share, NFS, CIFS/SMB, SSHFS, WebDAV) is a high-risk
//! configuration — cloud sync amplifies multi-device concurrent-write
//! conflicts, and network drives have unreliable file locking.
//!
//! The public API lives in `crate::vault_location`; private helpers
//! (`is_path_under`, `component_eq`) are covered by inline unit tests in the
//! module itself.

#[cfg(test)]
mod tests {
    use crate::vault_location::{
        detect, detect_cloud_sync, detect_network_drive, is_unc_path, is_under_cloud_sync_dir,
        network_fs_from_mounts, warning_message, LocationRisk,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn cloud_sync_detected_via_known_ancestor_component() {
        let vault = Path::new("/home/u/OneDrive/Documents/VaultPilotVault");
        // No env vars needed: the "OneDrive" ancestor component is enough.
        assert!(is_under_cloud_sync_dir(vault, &[]));
        assert!(detect_cloud_sync(vault));
        assert_eq!(detect(vault), LocationRisk::CloudSync);
    }

    #[test]
    fn cloud_sync_detected_via_explicit_env_root() {
        let vault = Path::new("/home/u/cloud-stuff/VaultPilotVault");
        let roots = vec![PathBuf::from("/home/u/cloud-stuff")];
        assert!(is_under_cloud_sync_dir(vault, &roots));
    }

    #[test]
    fn cloud_sync_not_detected_for_plain_documents_dir() {
        let vault = Path::new("/home/u/Documents/VaultPilotVault");
        assert!(!is_under_cloud_sync_dir(vault, &[]));
        assert!(!detect_cloud_sync(vault));
        assert_eq!(detect(vault), LocationRisk::Safe);
    }

    #[test]
    fn network_drive_detected_via_unc_prefix() {
        let unc = Path::new(r"\\nas\share\VaultPilotVault");
        assert!(is_unc_path(unc));
        assert!(detect_network_drive(unc));
        assert_eq!(detect(unc), LocationRisk::NetworkDrive);
    }

    #[test]
    fn network_drive_detected_via_mount_table() {
        let vault = Path::new("/mnt/nfs-export/VaultPilotVault");
        let mounts = "\
/dev/sda1 / ext4 rw 0 0
192.168.1.10:/srv/vaults /mnt/nfs-export nfs4 rw 0 0
";
        assert!(network_fs_from_mounts(vault, mounts));
        assert_eq!(detect(vault), LocationRisk::Safe); // mount table not consulted outside Linux
    }

    #[test]
    fn combined_cloud_and_network_classified_as_both() {
        // On Windows, `\\nas\share\OneDrive\Vault` is a UNC path with a
        // cloud-sync component → both risks. On Unix the backslashes are
        // ordinary characters, so only the UNC half is detected.
        let both = Path::new(r"\\nas\share\OneDrive\Vault");
        #[cfg(windows)]
        assert_eq!(detect(both), LocationRisk::CloudSyncAndNetworkDrive);
        #[cfg(not(windows))]
        assert_eq!(detect(both), LocationRisk::NetworkDrive);
    }

    #[test]
    fn warning_message_mentions_risk_and_advice() {
        let msg = warning_message(LocationRisk::CloudSync, Path::new("/x/OneDrive/V"))
            .expect("risky location yields a warning");
        assert!(msg.contains("云同步"));
        assert!(msg.contains("并发写冲突"));

        let net = warning_message(LocationRisk::NetworkDrive, Path::new(r"\\nas\v"))
            .expect("risky location yields a warning");
        assert!(net.contains("网络驱动器"));
        assert!(net.contains("LOCALAPPDATA"));
    }

    #[test]
    fn safe_location_yields_no_warning() {
        assert!(warning_message(LocationRisk::Safe, Path::new("/tmp/v")).is_none());
    }
}
