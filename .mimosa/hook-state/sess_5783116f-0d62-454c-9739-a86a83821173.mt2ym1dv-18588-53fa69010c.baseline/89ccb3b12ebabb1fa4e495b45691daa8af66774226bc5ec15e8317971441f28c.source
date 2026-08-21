//! Vault location risk detection (issue #3928).
//!
//! Mirrors Anytype JS-9831 and Obsidian 1.13.5: a vault data directory
//! sitting inside a cloud-synced folder (OneDrive/Dropbox/iCloud/…) or on a
//! network file system (UNC share, NFS, CIFS/SMB, SSHFS, WebDAV) is a
//! high-risk configuration for a local-first note app:
//!
//! - cloud sync amplifies multi-device **concurrent-write conflicts** (the
//!   vault is designed to be synced across devices — see `storage/pool.rs`
//!   #2831 — so two devices writing at once on a synced folder can silently
//!   overwrite each other);
//! - network drives have **unreliable file locking and write latency**
//!   (the target of Obsidian 1.13.5's "vaults on network drives" fix), which
//!   hurts the agent's frequent index/note writes.
//!
//! The detection is intentionally advisory: callers print a one-shot warning
//! and never block usage. The knowledge-base index and the machine-bound API
//! key already live outside the vault (`LOCALAPPDATA`/XDG — see
//! `storage/pool.rs::cli_config_root`), so only the notes themselves are at
//! risk.
//!
//! Known limitations:
//! - Windows *mapped* drives (e.g. `Z:\` → `\\nas\share`) are not detected
//!   without the `windows` crate (`GetDriveTypeW`); direct UNC paths (`\\…`)
//!   are detected.
//! - macOS detection relies on path components (case-insensitive APFS) and
//!   the same env-var roots; a `mount`-table parser exists only for Linux
//!   (`/proc/self/mounts`), which is where the agent sidecar runs in CI.

use std::path::{Path, PathBuf};

/// Location risk classification for a vault data directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationRisk {
    /// No known cloud-sync or network-drive risk.
    Safe,
    /// Under a known cloud-sync folder (OneDrive/Dropbox/iCloud/…).
    CloudSync,
    /// On a network file system (UNC / NFS / CIFS / SSHFS / WebDAV).
    NetworkDrive,
    /// Cloud-synced **and** on a network file system.
    CloudSyncAndNetworkDrive,
}

impl LocationRisk {
    /// Whether the risk warrants a startup warning.
    pub fn is_risky(self) -> bool {
        !matches!(self, LocationRisk::Safe)
    }

    /// Short English label (for logs / structured output).
    pub fn label(self) -> &'static str {
        match self {
            LocationRisk::Safe => "safe",
            LocationRisk::CloudSync => "cloud-sync",
            LocationRisk::NetworkDrive => "network-drive",
            LocationRisk::CloudSyncAndNetworkDrive => "cloud-sync + network-drive",
        }
    }

    fn label_zh(self) -> &'static str {
        match self {
            LocationRisk::Safe => "安全位置",
            LocationRisk::CloudSync => "云同步目录",
            LocationRisk::NetworkDrive => "网络驱动器",
            LocationRisk::CloudSyncAndNetworkDrive => "云同步目录（且为网络驱动器）",
        }
    }
}

/// Known cloud-sync folder names, matched case-insensitively against each
/// ancestor component of the vault dir (Windows/macOS paths are
/// case-insensitive by default).
const KNOWN_CLOUD_DIR_NAMES: &[&str] = &[
    "onedrive",
    "dropbox",
    "icloud",
    "mobile documents", // macOS iCloud Drive: ~/Library/Mobile Documents
    "google drive",
    "google drive file stream",
    "mega",
    "megasync",
    "pcloud",
    "box sync",
    "nutstore",
    "坚果云",
    "baidunetdisk",
    "百度网盘",
    "nextcloud",
    "owncloud",
    "seafile",
    "yandex.disk",
    "syncthing",
];

/// Environment variables that may point at a cloud-sync root folder.
const CLOUD_ROOT_ENV_VARS: &[&str] = &[
    "OneDrive",
    "OneDriveConsumer",
    "OneDriveCommercial",
    "DROPBOX",
];

/// File-system types from `/proc/mounts` that indicate a network-mounted
/// vault (NFS/CIFS/SMB/SSHFS/WebDAV). Local/overlay/loopback types are
/// intentionally absent.
const NETWORK_FS_TYPES: &[&str] = &[
    "nfs",
    "nfs4",
    "cifs",
    "smbfs",
    "smb3",
    "sshfs",
    "davfs",
    "davfs2",
    "fuse.sshfs",
    "fuse.davfs2",
    "fuse.smb",
    "fusesmb",
];

/// Case-aware component comparison. Windows and macOS user-facing paths are
/// case-insensitive; Linux paths are case-sensitive.
fn component_eq(a: &std::ffi::OsStr, b: &str) -> bool {
    let sa = a.to_string_lossy();
    if cfg!(any(windows, target_os = "macos")) {
        sa.eq_ignore_ascii_case(b)
    } else {
        sa == b
    }
}

/// Component-wise "is `vault` under `root`?" prefix check.
///
/// Uses `components()` instead of string prefixing so that `C:\Foo` and
/// `c:\foo` (or `/a/b` vs `/a/bc`) are handled structurally; comparison is
/// case-insensitive on Windows/macOS, exact on Linux.
fn is_path_under(vault: &Path, root: &Path) -> bool {
    let v: Vec<_> = vault.components().collect();
    let r: Vec<_> = root.components().collect();
    if r.len() > v.len() {
        return false;
    }
    v.iter()
        .zip(r.iter())
        .all(|(a, b)| component_eq(a.as_os_str(), &b.as_os_str().to_string_lossy()))
}

/// Whether `vault_dir` sits under any of the given cloud-sync roots, or
/// under an ancestor whose name matches a known cloud-sync folder, or under
/// a Syncthing-managed folder (`.stfolder` marker, best-effort).
pub fn is_under_cloud_sync_dir(vault_dir: &Path, env_roots: &[PathBuf]) -> bool {
    if env_roots.iter().any(|root| is_path_under(vault_dir, root)) {
        return true;
    }

    let mut current = Some(vault_dir);
    while let Some(dir) = current {
        if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_lowercase();
            if KNOWN_CLOUD_DIR_NAMES.iter().any(|k| lower == *k) {
                return true;
            }
        }
        // Syncthing marks managed folders with a `.stfolder` file.
        if dir.join(".stfolder").is_file() {
            return true;
        }
        current = dir.parent();
    }
    false
}

/// Detect cloud-sync placement using the live environment (OneDrive*/DROPBOX
/// env vars) plus path-component heuristics.
pub fn detect_cloud_sync(vault_dir: &Path) -> bool {
    let roots: Vec<PathBuf> = CLOUD_ROOT_ENV_VARS
        .iter()
        .filter_map(|name| std::env::var_os(name).map(PathBuf::from))
        .collect();
    is_under_cloud_sync_dir(vault_dir, &roots)
}

/// Parse `/proc/mounts`-style content and decide whether `vault_dir` lives
/// on a network file system. Pure function — tests feed synthetic mount
/// tables; the longest matching mount point wins (nested mounts).
pub fn network_fs_from_mounts(vault_dir: &Path, mounts_content: &str) -> bool {
    let vault = vault_dir
        .canonicalize()
        .unwrap_or_else(|_| vault_dir.to_path_buf());
    let vault_str = vault.to_string_lossy().to_string();

    let mut best_len: Option<usize> = None;
    let mut best_is_network = false;

    for line in mounts_content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        // /proc/mounts escapes spaces in mount points as \040.
        let mount_point = fields[1].replace("\\040", " ");
        let fs_type = fields[2];

        let is_prefix = vault_str == mount_point
            || vault_str.starts_with(&format!("{mount_point}/"))
            || (mount_point == "/" && vault_str.starts_with('/'));
        if !is_prefix {
            continue;
        }

        let len = mount_point.len();
        let is_network = NETWORK_FS_TYPES.contains(&fs_type);
        if best_len.is_none_or(|bl| len > bl) {
            best_len = Some(len);
            best_is_network = is_network;
        }
    }

    best_len.is_some() && best_is_network
}

/// Windows UNC share paths start with `\\server\share`.
pub fn is_unc_path(vault_dir: &Path) -> bool {
    vault_dir.to_string_lossy().starts_with("\\\\")
}

/// Detect network-drive placement: UNC prefix on any platform, plus the
/// Linux mount table (`/proc/self/mounts`). Mapped Windows drive letters
/// are not detectable without the `windows` crate (documented limitation).
pub fn detect_network_drive(vault_dir: &Path) -> bool {
    if is_unc_path(vault_dir) {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/self/mounts") {
            return network_fs_from_mounts(vault_dir, &content);
        }
        if let Ok(content) = std::fs::read_to_string("/proc/mounts") {
            return network_fs_from_mounts(vault_dir, &content);
        }
    }
    false
}

/// Combined risk classification for a vault directory.
pub fn detect(vault_dir: &Path) -> LocationRisk {
    let cloud = detect_cloud_sync(vault_dir);
    let network = detect_network_drive(vault_dir);
    match (cloud, network) {
        (true, true) => LocationRisk::CloudSyncAndNetworkDrive,
        (true, false) => LocationRisk::CloudSync,
        (false, true) => LocationRisk::NetworkDrive,
        (false, false) => LocationRisk::Safe,
    }
}

/// Human-readable (Chinese) advisory warning for a risky vault location.
/// Returns `None` when the location is safe.
pub fn warning_message(risk: LocationRisk, vault_dir: &Path) -> Option<String> {
    if !risk.is_risky() {
        return None;
    }
    Some(format!(
        "警告：vault 目录位于{}，存在多设备并发写冲突与文件锁不可靠的风险：\n  {}\n\
         建议将 vault 移出云同步/网络目录。知识库索引与 API 密钥已保存在本机应用数据目录 \
         （LOCALAPPDATA/XDG），不随 vault 同步，不受影响。",
        risk.label_zh(),
        vault_dir.display()
    ))
}

/// Emit a one-shot stderr warning at startup when the vault lives on a
/// cloud-synced folder or a network drive (#3928). No-op for safe locations.
pub fn warn_if_risky_location(vault_dir: &Path) {
    let risk = detect(vault_dir);
    if let Some(message) = warning_message(risk, vault_dir) {
        eprintln!("{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_path_under_matches_exact_and_nested() {
        let vault = Path::new("/home/u/Documents/VaultPilotVault");
        assert!(is_path_under(vault, Path::new("/home/u")));
        assert!(is_path_under(vault, Path::new("/home/u/Documents")));
        assert!(is_path_under(vault, vault));
        assert!(!is_path_under(
            vault,
            Path::new("/home/u/Documents/VaultPilotVault/extra")
        ));
        assert!(!is_path_under(vault, Path::new("/home/u2")));
        assert!(is_path_under(vault, Path::new("/home")));
    }

    #[test]
    fn is_path_under_rejects_shallow_prefix_sibling() {
        // /a/b must not be considered under /a/bc
        let vault = Path::new("/a/bc/vault");
        assert!(!is_path_under(vault, Path::new("/a/b")));
        assert!(is_path_under(vault, Path::new("/a/bc")));
    }

    #[test]
    fn is_under_cloud_sync_dir_matches_env_root() {
        let vault = Path::new("/home/u/OneDrive/Docs/VaultPilotVault");
        let roots = vec![PathBuf::from("/home/u/OneDrive")];
        assert!(is_under_cloud_sync_dir(vault, &roots));
    }

    #[test]
    fn is_under_cloud_sync_dir_rejects_unrelated_env_root() {
        let vault = Path::new("/home/u/Documents/VaultPilotVault");
        let roots = vec![PathBuf::from("/home/u/OneDrive")];
        assert!(!is_under_cloud_sync_dir(vault, &roots));
    }

    #[test]
    fn is_under_cloud_sync_dir_matches_known_ancestor_name_without_env() {
        // Known folder name on an ancestor component, no env roots at all.
        let vault = Path::new("/home/u/OneDrive/Docs/Vault");
        assert!(is_under_cloud_sync_dir(vault, &[]));
        let dropbox = Path::new("/home/u/Dropbox/MyVault");
        assert!(is_under_cloud_sync_dir(dropbox, &[]));
    }

    #[test]
    fn is_under_cloud_sync_dir_matches_icloud_mobile_documents() {
        let vault = Path::new("/Users/u/Library/Mobile Documents/com~apple~CloudDocs/Vault");
        assert!(is_under_cloud_sync_dir(vault, &[]));
    }

    #[test]
    fn is_under_cloud_sync_dir_detects_stfolder_marker() {
        let dir = std::env::temp_dir().join(format!("vp-stfolder-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join(".stfolder"), b"").expect("write marker");
        let vault = dir.join("Vault");
        std::fs::create_dir_all(&vault).expect("create vault dir");
        // No env roots, no known name — only the Syncthing marker.
        assert!(is_under_cloud_sync_dir(&vault, &[]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn network_fs_from_mounts_detects_nfs_and_cifs() {
        let vault = Path::new("/mnt/nas/VaultPilotVault");
        let mounts = "\
/dev/sda1 / ext4 rw 0 0
10.0.0.5:/export /mnt/nas nfs rw 0 0
";
        assert!(network_fs_from_mounts(vault, mounts));

        let cifs = "\
10.0.0.5/share /mnt/smb cifs rw 0 0
";
        assert!(network_fs_from_mounts(Path::new("/mnt/smb/Vault"), cifs));
    }

    #[test]
    fn network_fs_from_mounts_ignores_local_fs() {
        let vault = Path::new("/home/u/VaultPilotVault");
        let mounts = "\
/dev/sda1 / ext4 rw 0 0
/dev/sda2 /home ext4 rw 0 0
overlay / overlay rw 0 0
";
        assert!(!network_fs_from_mounts(vault, mounts));
    }

    #[test]
    fn network_fs_from_mounts_longest_mount_wins() {
        // Vault under /mnt/nas/localcopy which is ext4, while /mnt/nas is nfs:
        // the longest (most specific) mount point decides.
        let vault = Path::new("/mnt/nas/localcopy/Vault");
        let mounts = "\
10.0.0.5:/export /mnt/nas nfs rw 0 0
/dev/sdb1 /mnt/nas/localcopy ext4 rw 0 0
";
        assert!(!network_fs_from_mounts(vault, mounts));
    }

    #[test]
    fn network_fs_from_mounts_handles_escaped_spaces_in_mount_point() {
        let vault = Path::new("/mnt/My NAS/Vault");
        let mounts = "/dev/sda1 /mnt/My\\040NAS ext4 rw 0 0\n";
        // Space-escaped mount point parses and matches, still local fs.
        assert!(!network_fs_from_mounts(vault, mounts));
    }

    #[test]
    fn is_unc_path_detects_windows_unc() {
        assert!(is_unc_path(Path::new(r"\\nas\share\VaultPilotVault")));
        assert!(!is_unc_path(Path::new("C:\\Users\\u\\Documents\\Vault")));
        assert!(!is_unc_path(Path::new("/mnt/nas/Vault")));
    }

    #[test]
    fn detect_combines_cloud_and_network() {
        // On Windows, `\\nas\share\OneDrive\Vault` parses as a UNC path whose
        // "OneDrive" component is cloud-sync; on Unix backslashes are ordinary
        // characters (single opaque component), so only the UNC half matches.
        let unc_cloud = Path::new(r"\\nas\share\OneDrive\Vault");
        #[cfg(windows)]
        assert_eq!(detect(unc_cloud), LocationRisk::CloudSyncAndNetworkDrive);
        #[cfg(not(windows))]
        assert_eq!(detect(unc_cloud), LocationRisk::NetworkDrive);

        let unc_only = Path::new(r"\\nas\share\Vault");
        assert_eq!(detect(unc_only), LocationRisk::NetworkDrive);
    }

    #[test]
    fn warning_message_none_for_safe() {
        assert!(warning_message(LocationRisk::Safe, Path::new("/tmp/vault")).is_none());
    }

    #[test]
    fn warning_message_some_for_risky() {
        let msg = warning_message(LocationRisk::CloudSync, Path::new("/x/OneDrive/Vault"))
            .expect("risky location must warn");
        assert!(msg.contains("云同步"));
        let net_msg = warning_message(LocationRisk::NetworkDrive, Path::new(r"\\nas\vault"))
            .expect("risky location must warn");
        assert!(net_msg.contains("网络驱动器"));
    }
}
