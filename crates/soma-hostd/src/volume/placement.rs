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

/// Returns the record root for one configured value, or the default when it is absent or blank.
#[must_use]
pub fn record_dir(configured: Option<&str>) -> PathBuf {
    configured
        .filter(|value| !value.trim().is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_RECORD_DIR), PathBuf::from)
}

/// Whether a mount table puts `directory` on `tmpfs` or `ramfs`.
///
/// The longest mount point that is a prefix of the directory decides, which is how the kernel
/// resolves a mount, so a durable root that itself sits under a volatile parent is still seen as
/// volatile. Mount fields are not unescaped, so a mount point containing spaces is read as two
/// fields; no platform mount point contains one, and a wrong reading can only fail the way the
/// check already fails, by calling a durable directory volatile.
#[must_use]
pub fn on_volatile_mount(mounts: &str, directory: &Path) -> bool {
    let mut best: Option<(usize, &str)> = None;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(_device), Some(mount), Some(filesystem)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let mount_path = Path::new(mount);
        if directory.starts_with(mount_path) {
            let depth = mount_path.components().count();
            if best.is_none_or(|(current, _)| depth > current) {
                best = Some((depth, filesystem));
            }
        }
    }
    matches!(
        best.map(|(_, filesystem)| filesystem),
        Some("tmpfs" | "ramfs")
    )
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
    fn a_malformed_line_is_skipped_rather_than_fatal() {
        assert!(!on_volatile_mount("garbage\ntmpfs\n", Path::new("/x")));
    }
}
