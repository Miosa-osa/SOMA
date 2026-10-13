//! Where the volume write-ahead records live, and the rule that keeps them off volatile disk.
//!
//! The Instance registry may stay on a `tmpfs` mount, because a running machine cannot survive a
//! reboot and a stopped volume is described by its sidecar rather than by the registry.
//! A write-ahead record is the opposite: it exists to be replayed after a crash or a reboot, so a
//! record on a mount a reboot erases cannot do the one thing it is for.
//! The rule is simple enough to state once: anything that must survive a reboot lives under the
//! durable record root, and anything that describes only a running machine may live on `tmpfs`.

use std::io;
use std::path::{Path, PathBuf};

/// The environment variable that overrides the durable record root.
pub const RECORD_DIR_ENV: &str = "SOMA_HOSTD_RECORD_DIR";
/// The default durable record root, on persistent disk and never under `/run`.
pub const DEFAULT_RECORD_DIR: &str = "/var/lib/soma/hostd/records";
/// The default root holding one directory per volume, on the host's encrypted data mount.
pub const DEFAULT_VOLUME_DIR: &str = "/srv/soma/volumes";
/// The environment variable that names this host's encrypted data mount.
pub const ENCRYPTED_MOUNT_ENV: &str = "SOMA_HOSTD_ENCRYPTED_MOUNT";
/// The encrypted data mount the reference Step A host carries, which is a LUKS volume.
pub const DEFAULT_ENCRYPTED_MOUNT: &str = "miosa-enc";

/// Returns the encrypted data mount name for one configured value, or the default when absent.
#[must_use]
pub fn encrypted_mount_name(configured: Option<&str>) -> String {
    configured
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_ENCRYPTED_MOUNT)
        .to_owned()
}

/// Returns the record root for one configured value, or the default when it is absent or blank.
#[must_use]
pub fn record_dir(configured: Option<&str>) -> PathBuf {
    configured
        .filter(|value| !value.trim().is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_RECORD_DIR), PathBuf::from)
}

/// One mount table row: the device, the mount point, and the filesystem.
type MountRow<'a> = (&'a str, &'a str, &'a str);

/// The mount the kernel resolves `directory` to, as its device, mount point, and filesystem.
///
/// The longest mount point that is a prefix of the directory decides, which is how the kernel
/// resolves a mount, so a durable root that itself sits under a volatile parent is still seen as
/// volatile. Mount fields are not unescaped, so a mount point containing spaces is read as two
/// fields; no platform mount point contains one, and a wrong reading can only fail the way the
/// checks already fail, by calling a directory something it is not.
fn resolved_mount<'a>(mounts: &'a str, directory: &Path) -> Option<MountRow<'a>> {
    let mut best: Option<(usize, MountRow<'a>)> = None;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(device), Some(mount), Some(filesystem)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let mount_path = Path::new(mount);
        if directory.starts_with(mount_path) {
            let depth = mount_path.components().count();
            if best.is_none_or(|(current, _)| depth > current) {
                best = Some((depth, (device, mount, filesystem)));
            }
        }
    }
    best.map(|(_, found)| found)
}

/// Whether a mount table puts `directory` on `tmpfs` or `ramfs`.
#[must_use]
pub fn on_volatile_mount(mounts: &str, directory: &Path) -> bool {
    matches!(
        resolved_mount(mounts, directory).map(|(_, _, filesystem)| filesystem),
        Some("tmpfs" | "ramfs")
    )
}

/// Whether a mount table puts `directory` on the named encrypted data mount.
///
/// A LUKS volume is presented by the kernel as a device-mapper target, so the mount this
/// directory resolves to must name `/dev/mapper/<name>`. What is checked is the mapping's name
/// and not its type: a host that maps something other than `dm-crypt` under that name passes,
/// which is why the name is this host's own configured mount rather than any mapper device.
#[must_use]
pub fn on_encrypted_mount(mounts: &str, directory: &Path, name: &str) -> bool {
    let mapper = format!("/dev/mapper/{name}");
    resolved_mount(mounts, directory).is_some_and(|(device, _, _)| device == mapper)
}

/// Refuses a record directory on `tmpfs` or `ramfs`, where a reboot would erase unshipped records.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] when the directory resolves to a volatile mount, and
/// the read error when the mount table cannot be read.
#[cfg(target_os = "linux")]
pub fn refuse_volatile(directory: &Path) -> io::Result<()> {
    let mounts = std::fs::read_to_string("/proc/self/mounts")?;
    let resolved = std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    if on_volatile_mount(&mounts, &resolved) {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the volume write-ahead records must be on persistent disk, not tmpfs",
        ))
    } else {
        Ok(())
    }
}

/// Whether this host's volume root is on its encrypted data mount.
///
/// # Errors
///
/// Returns the read error when the mount table cannot be read.
#[cfg(target_os = "linux")]
pub fn root_is_encrypted(directory: &Path, name: &str) -> io::Result<bool> {
    let mounts = std::fs::read_to_string("/proc/self/mounts")?;
    let resolved = std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    Ok(on_encrypted_mount(&mounts, &resolved, name))
}

/// The non-Linux build is not the host runtime and has no mount table, so it cannot certify one.
///
/// # Errors
///
/// This build never returns an error; the signature matches the Linux check.
#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the Linux check, which can fail to read the mount table"
)]
pub fn root_is_encrypted(directory: &Path, name: &str) -> io::Result<bool> {
    let _ = (directory, name);
    Ok(false)
}

/// The non-Linux build has no mount table to consult, so the check cannot refuse.
///
/// # Errors
///
/// This build never returns an error; the signature matches the Linux check.
#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the Linux check, which can refuse"
)]
pub fn refuse_volatile(directory: &Path) -> io::Result<()> {
    let _ = directory;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTS: &str = "\
sysfs /sys sysfs rw,nosuid 0 0
tmpfs /run tmpfs rw,nosuid,size=2097152k 0 0
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
/dev/nvme0n1p3 /var/lib/soma ext4 rw,relatime 0 0
tmpfs /var/lib/soma/hostd/scratch tmpfs rw,size=1024k 0 0
";

    #[test]
    fn the_default_record_root_is_not_under_run() {
        assert_eq!(DEFAULT_RECORD_DIR, "/var/lib/soma/hostd/records");
        assert!(!DEFAULT_RECORD_DIR.starts_with("/run"));
    }

    #[test]
    fn a_blank_or_absent_override_selects_the_default() {
        assert_eq!(record_dir(None), PathBuf::from(DEFAULT_RECORD_DIR));
        assert_eq!(record_dir(Some("   ")), PathBuf::from(DEFAULT_RECORD_DIR));
        assert_eq!(
            record_dir(Some("/srv/soma/wal")),
            PathBuf::from("/srv/soma/wal")
        );
    }

    #[test]
    fn a_durable_directory_is_not_volatile() {
        assert!(!on_volatile_mount(
            MOUNTS,
            Path::new("/var/lib/soma/hostd/records")
        ));
    }

    #[test]
    fn a_tmpfs_directory_is_volatile() {
        assert!(on_volatile_mount(
            MOUNTS,
            Path::new("/run/soma-hostd/ledger")
        ));
    }

    #[test]
    fn the_longest_matching_mount_point_decides() {
        // `/var/lib/soma` is ext4 and `/var/lib/soma/hostd/scratch` is tmpfs, so a path under the
        // scratch mount is volatile even though its grandparent mount is durable.
        assert!(on_volatile_mount(
            MOUNTS,
            Path::new("/var/lib/soma/hostd/scratch/volumes")
        ));
        assert!(!on_volatile_mount(
            MOUNTS,
            Path::new("/var/lib/soma/volumes")
        ));
    }

    #[test]
    fn only_the_named_data_mapper_is_an_encrypted_root() {
        const ENCRYPTED: &str = "\
sysfs /sys sysfs rw,nosuid 0 0
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
/dev/mapper/miosa-enc /srv/soma ext4 rw,relatime 0 0
";
        assert!(on_encrypted_mount(
            ENCRYPTED,
            Path::new("/srv/soma/volumes"),
            "miosa-enc"
        ));
        // A raw partition is refused, however it is formatted, and so is a mapper under another
        // name: the mount has to be this host's own encrypted data mount.
        assert!(!on_encrypted_mount(
            MOUNTS,
            Path::new("/var/lib/soma/volumes"),
            "miosa-enc"
        ));
        assert!(!on_encrypted_mount(
            ENCRYPTED,
            Path::new("/srv/soma/volumes"),
            "another-volume"
        ));
    }

    #[test]
    fn the_longest_mount_point_decides_for_encryption_too() {
        const NESTED: &str = "\
/dev/mapper/miosa-enc /srv/soma ext4 rw,relatime 0 0
/dev/nvme0n1p3 /srv/soma/plain ext4 rw,relatime 0 0
";
        assert!(on_encrypted_mount(
            NESTED,
            Path::new("/srv/soma/volumes"),
            "miosa-enc"
        ));
        assert!(!on_encrypted_mount(
            NESTED,
            Path::new("/srv/soma/plain/volumes"),
            "miosa-enc"
        ));
    }

    #[test]
    fn a_blank_or_absent_mount_name_selects_the_reference_host_mount() {
        assert_eq!(DEFAULT_ENCRYPTED_MOUNT, "miosa-enc");
        assert_eq!(encrypted_mount_name(None), "miosa-enc");
        assert_eq!(encrypted_mount_name(Some("  ")), "miosa-enc");
        assert_eq!(encrypted_mount_name(Some("data-crypt")), "data-crypt");
    }

    #[test]
    fn a_malformed_line_is_skipped_rather_than_fatal() {
        assert!(!on_volatile_mount("garbage\ntmpfs\n", Path::new("/x")));
    }
}
