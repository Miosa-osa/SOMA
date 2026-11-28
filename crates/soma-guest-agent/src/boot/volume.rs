//! The durable data volume, for a machine that declared one.
//!
//! Captures reach the repair point with an empty volume slot and no filesystem mounted from it.
//! The guest first reads the assigned device after it accepts fresh launch authority.
//! Every persistent path must be mounted before the guest can report readiness.

use std::{fs, io, os::fd::AsRawFd as _};

mod live;

pub use live::Workspace;

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

/// Verifies the volume, mounts it, and binds its paths into the composed root, and answers
/// whether this Instance has a volume at all.
///
/// A device whose superblock region is all zeros is the host's empty scratch disk for a launch
/// that named no volume: the persistent paths then stay on the private overlay. Anything else
/// that is not a valid ext4 filesystem still fails the boot, so a damaged volume is never
/// served as an empty one (SOMA#262).
///
/// # Errors
///
/// Returns the first failed step; the caller reports it and powers off.
pub(super) fn compose() -> Result<bool, BootFailure> {
    // Read only after the cache of the capture's empty placeholder is dropped, or a real volume
    // would read as blank.
    flush_device()?;
    if super::superblock::superblock_is_blank(VOLUME_DEVICE).map_err(|errno| BootFailure {
        step: BootStep::VolumeIdentity,
        errno,
    })? {
        crate::console::report("no volume for this instance; persistent paths stay on the overlay");
        return Ok(false);
    }
    mount_device()?;
    for path in PERSISTENCE_PATHS {
        bind_path(path)?;
    }
    Ok(true)
}

fn mount_device() -> Result<(), BootFailure> {
    flush_device()?;
    verify_superblock(BootStep::VolumeIdentity, VOLUME_DEVICE, ext4_superblock_ok)?;
    mount_ext4()
}

/// Drops what Linux buffered from early boot's empty device, now that the worker has assigned
/// this Instance's backing file.
fn flush_device() -> Result<(), BootFailure> {
    let file =
        fs::File::open(VOLUME_DEVICE).map_err(|error| failure(BootStep::VolumeIdentity, &error))?;
    discard_placeholder_cache(&file).map_err(|error| failure(BootStep::VolumeIdentity, &error))
}

fn mount_ext4() -> Result<(), BootFailure> {
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
    })
}

/// Binds one persistent path from the volume onto the composed root.
///
/// Both ends are created when they are absent rather than assumed, because an OCI image is not
/// required to carry `/workspace` or `/opt/user`; a target that cannot be created fails the boot
/// rather than silently leaving the path on the ephemeral root.
fn bind_path(path: &str) -> Result<(), BootFailure> {
    let step = BootStep::VolumeBind;
    let source = format!("{VOLUME_MOUNT}{path}");
    fs::create_dir_all(&source).map_err(|error| failure(step, &error))?;
    fs::create_dir_all(path).map_err(|error| failure(step, &error))?;
    mounts::bind(&source, path).map_err(|errno| BootFailure {
        step,
        errno: errno.0,
    })
}

#[allow(unsafe_code)]
fn discard_placeholder_cache(device: &fs::File) -> io::Result<()> {
    const BLKFLSBUF: libc::c_ulong = 0x1261;
    // SAFETY: BLKFLSBUF takes no pointer or other argument and the open file owns the fd.
    let result = unsafe { libc::ioctl(device.as_raw_fd(), crate::ioctl::request(BLKFLSBUF)) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
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
