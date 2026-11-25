//! The rule that a host which declares its encrypted data mount serves volumes only from it.

use std::{io, path::Path};

use super::placement::on_encrypted_mount;

/// Whether a host that declared an encrypted data mount has its volume root somewhere else.
///
/// A host that names its mount (`SOMA_HOSTD_ENCRYPTED_MOUNT`) holds tenant volumes there, so a
/// root that resolves elsewhere is a mount that is missing: unmounted, or its device closed. A
/// host that declares none holds no tenant data and is never refused here; its attaches are.
#[must_use]
pub fn declared_mount_missing(mounts: &str, directory: &Path, declared: Option<&str>) -> bool {
    declared
        .filter(|name| !name.trim().is_empty())
        .is_some_and(|name| !on_encrypted_mount(mounts, directory, name))
}

/// Refuses to serve volumes when the encrypted data mount this host declares is not mounted.
///
/// # Errors
///
/// Returns [`io::ErrorKind::NotFound`] when the root is not on the declared mount, and the read
/// error when the mount table cannot be read.
#[cfg(target_os = "linux")]
pub fn refuse_missing_mount(directory: &Path, declared: Option<&str>) -> io::Result<()> {
    let mounts = std::fs::read_to_string("/proc/self/mounts")?;
    let resolved = std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    if declared_mount_missing(&mounts, &resolved, declared) {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the volume root is not on the encrypted data mount this host declares",
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
pub fn refuse_missing_mount(directory: &Path, declared: Option<&str>) -> io::Result<()> {
    let _ = (directory, declared);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_mount_that_is_not_mounted_is_missing_and_an_undeclared_one_never_is() {
        let mounted =
            "/dev/mapper/miosa_durable /srv/miosa-durable ext4 rw 0 0\n/dev/md0 / ext4 rw 0 0\n";
        let unmounted = "/dev/md0 / ext4 rw 0 0\n";
        let root = Path::new("/srv/miosa-durable/soma/volumes");
        assert!(!declared_mount_missing(
            mounted,
            root,
            Some("miosa_durable")
        ));
        assert!(declared_mount_missing(
            unmounted,
            root,
            Some("miosa_durable")
        ));
        assert!(!declared_mount_missing(unmounted, root, None));
        assert!(!declared_mount_missing(unmounted, root, Some(" ")));
    }
}
