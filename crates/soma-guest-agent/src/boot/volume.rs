//! The durable data volume, for a machine that declared one.
//!
//! Everything here runs only when the Generation declared a durable volume. The device is
//! verified and mounted before the root is entered, and the persistent paths are bind mounted
//! from it into the composed root, so a workload that writes them writes the volume and a stop
//! that keeps the disk keeps them.
//! The mount is deliberately not lazy: it happens here, inside the early-init sequence, before
//! the guest can report that it is running, so a start can never answer that a sandbox is up
//! before its own data is visible.

use std::fs;

use crate::mounts;

use super::superblock::{ext4_superblock_ok, verify_superblock};
use super::{BootFailure, BootStep, VOLUME_DEVICE, failure};

/// Where the durable volume is mounted for the composed root to bind from.
pub(super) const VOLUME_MOUNT: &str = "/mnt/volume";

/// The persistent paths a Step A volume carries, in the order ADR 0053 declares them.
///
/// `/var/lib/docker` is here so container images and containers survive a stop. Nothing else is
/// persistent in Step A: `/etc`, package state and `/usr` wait for ADR 0053 Step B.
pub const PERSISTENCE_PATHS: [&str; 4] = [
    "/home/sandbox",
    "/workspace",
    "/var/lib/docker",
    "/opt/user",
];

/// The mount flag that defers timestamp updates to memory; Linux 4.0 and later.
///
/// Spelled here rather than taken from `libc`, whose `MS_*` set stops short of it.
const MS_LAZYTIME: libc::c_ulong = 1 << 25;

/// Verifies the volume, mounts it, and binds its paths into the composed root.
///
/// # Errors
///
/// Returns the first failed step; the caller reports it and powers off.
pub(super) fn compose(root: &str) -> Result<(), BootFailure> {
    verify_superblock(BootStep::VolumeIdentity, VOLUME_DEVICE, ext4_superblock_ok)?;
    fs::create_dir_all(VOLUME_MOUNT).map_err(|error| failure(BootStep::VolumeMount, &error))?;
    mounts::mount(
        VOLUME_DEVICE,
        VOLUME_MOUNT,
        "ext4",
        libc::MS_NOSUID | libc::MS_NODEV | MS_LAZYTIME,
        "",
    )
    .map_err(|errno| BootFailure {
        step: BootStep::VolumeMount,
        errno: errno.0,
    })?;
    for path in PERSISTENCE_PATHS {
        bind_path(root, path)?;
    }
    Ok(())
}

/// Binds one persistent path from the volume onto the composed root.
///
/// Both ends are created when they are absent rather than assumed, because an OCI image is not
/// required to carry `/workspace` or `/opt/user`; a target that cannot be created fails the boot
/// rather than silently leaving the path on the ephemeral root.
fn bind_path(root: &str, path: &str) -> Result<(), BootFailure> {
    let step = BootStep::VolumeBind;
    let source = format!("{VOLUME_MOUNT}{path}");
    let target = format!("{root}{path}");
    fs::create_dir_all(&source).map_err(|error| failure(step, &error))?;
    fs::create_dir_all(&target).map_err(|error| failure(step, &error))?;
    mounts::bind(&source, &target).map_err(|errno| BootFailure {
        step,
        errno: errno.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_persistent_paths_are_the_four_adr_paths_in_order() {
        assert_eq!(
            PERSISTENCE_PATHS,
            [
                "/home/sandbox",
                "/workspace",
                "/var/lib/docker",
                "/opt/user"
            ]
        );
        assert_eq!(VOLUME_DEVICE, "/dev/vdc");
        // The device root is where the binds come from, so a path inside it must never be the
        // mount point itself.
        assert!(PERSISTENCE_PATHS.iter().all(|path| path.starts_with('/')));
        assert!(!PERSISTENCE_PATHS.contains(&VOLUME_MOUNT));
    }
}
