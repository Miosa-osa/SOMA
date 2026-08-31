//! Bounded early-init sequence from the Generation compiler contract.
//!
//! The agent is `/init` of the deterministic initramfs, so it performs the sequence itself:
//! mount devtmpfs, procfs, and sysfs, read what the Generation declared, wait for exactly the
//! block devices that declaration implies, verify and mount the EROFS lower, compose a writable
//! root over it when there is a private head, and switch into whichever root resulted.
//! The guest holds no immutable authentication secret: the responder static secret is fresh
//! per Instance and arrives later through the launch page.
//! Every step is typed and every failure is reported with the step and errno before poweroff.
//!
//! A Generation that declared no writable storage has no second block device and no `OverlayFS`
//! at all: it verifies one superblock, performs one mount, and enters the immutable root. The
//! sterile-head checks are not weakened for it, they simply have nothing to run against, and
//! they still run in full for every machine that does have a head.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::mounts::{self, Errno};

use self::devices::wait_for_devices;
use self::superblock::{erofs_superblock_ok, verify_superblock};

mod declared;
mod devices;
mod superblock;
mod upper;

pub use self::declared::Declared;

/// First virtio-blk device: the immutable EROFS Generation root.
pub const ROOT_DEVICE: &str = "/dev/vda";
/// Second virtio-blk device: the Instance-private ext4 overlay head, when there is one.
pub const OVERLAY_DEVICE: &str = "/dev/vdb";
/// Upper bound for the complete early-init sequence.
pub const BOOT_BUDGET: Duration = Duration::from_secs(10);

const LOWER_MOUNT: &str = "/mnt/lower";
const ROOT_MOUNT: &str = "/mnt/root";
const NOSUID_NODEV_NOEXEC: libc::c_ulong = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;

/// One typed early-init step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootStep {
    /// Mount devtmpfs on `/dev`.
    Devtmpfs,
    /// Mount devpts on `/dev/pts`.
    Devpts,
    /// Mount procfs on `/proc`.
    Procfs,
    /// Mount sysfs on `/sys`.
    Sysfs,
    /// Wait for exactly the declared virtio block devices.
    Devices,
    /// Verify the EROFS superblock of the root device.
    LowerIdentity,
    /// Mount the EROFS lower read-only.
    LowerMount,
    /// Verify the ext4 superblock and clean state of the overlay device.
    UpperIdentity,
    /// Mount the private ext4 upper.
    UpperMount,
    /// Create the upper and work directories on a sterile head.
    UpperDirectories,
    /// Mount `OverlayFS` over the composed directories.
    Overlay,
    /// Move `/dev`, `/proc`, and `/sys` into the root.
    MoveMounts,
    /// Move the root over `/` and enter it.
    Pivot,
}

/// A typed early-init failure with the kernel errno that caused it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootFailure {
    /// The step that failed.
    pub step: BootStep,
    /// The kernel errno, or zero for a contract violation without a syscall error.
    pub errno: i32,
}

/// Performs the complete early-init sequence before the given absolute deadline.
///
/// Returns what the Generation declared, because the same declaration decides what the rest of
/// the agent must do: a machine with no network device has no interface to repair, and a
/// machine with no writable root cannot be asked to write to one.
///
/// # Errors
///
/// Returns the first failed step; the caller must report it and power off.
pub fn early_init(deadline: Instant) -> Result<Declared, BootFailure> {
    mount_pseudo(
        BootStep::Devtmpfs,
        "devtmpfs",
        "/dev",
        "devtmpfs",
        "mode=0755",
    )?;
    // Every pseudo-terminal slave lives on devpts, and `/dev/ptmx` cannot allocate a pair
    // without it, so a guest with no devpts has the terminal protocol and no terminal to serve
    // it with. It is mounted here, under `/dev`, so the move below carries it into the root
    // with the rest of the device tree.
    mount_pseudo(
        BootStep::Devpts,
        "devpts",
        "/dev/pts",
        "devpts",
        "mode=0620,ptmxmode=0666",
    )?;
    mount_pseudo(BootStep::Procfs, "proc", "/proc", "proc", "")?;
    mount_pseudo(BootStep::Sysfs, "sysfs", "/sys", "sysfs", "")?;
    // The declaration is read only after procfs exists and before any device is waited for, so
    // the wait knows how many devices to expect rather than assuming the maximum.
    let declared = Declared::from_proc();
    wait_for_devices(deadline, declared)?;
    verify_superblock(BootStep::LowerIdentity, ROOT_DEVICE, erofs_superblock_ok)?;
    fs::create_dir_all(LOWER_MOUNT).map_err(|error| failure(BootStep::LowerMount, &error))?;
    mounts::mount(
        ROOT_DEVICE,
        LOWER_MOUNT,
        "erofs",
        libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
        "",
    )
    .map_err(|errno| BootFailure {
        step: BootStep::LowerMount,
        errno: errno.0,
    })?;
    // With no private head the immutable root is the root: there is no second layer to compose
    // it with, so the mount that is already there is the one the guest enters.
    let root = if declared.overlay {
        upper::compose()?;
        ROOT_MOUNT
    } else {
        LOWER_MOUNT
    };
    move_mounts(root)?;
    switch_root(root)?;
    Ok(declared)
}

fn mount_pseudo(
    step: BootStep,
    source: &str,
    target: &str,
    fstype: &str,
    data: &str,
) -> Result<(), BootFailure> {
    fs::create_dir_all(target).map_err(|error| failure(step, &error))?;
    // A device filesystem must keep its device nodes usable, so neither devtmpfs nor devpts may
    // carry `MS_NODEV`; everything else this function mounts holds no device at all.
    let flags = if matches!(step, BootStep::Devtmpfs | BootStep::Devpts) {
        libc::MS_NOSUID | libc::MS_NOEXEC
    } else {
        NOSUID_NODEV_NOEXEC
    };
    match mounts::mount(source, target, fstype, flags, data) {
        Ok(()) | Err(Errno(libc::EBUSY)) => Ok(()),
        Err(errno) => Err(BootFailure {
            step,
            errno: errno.0,
        }),
    }
}

/// Moves the pseudo filesystems into the root the guest is about to enter.
///
/// The mount points must already exist on a read-only root, and every OCI image carries them,
/// so `create_dir_all` succeeding on a directory that is already there is the ordinary case
/// rather than a fallback.
fn move_mounts(root: &str) -> Result<(), BootFailure> {
    let step = BootStep::MoveMounts;
    for name in ["dev", "proc", "sys"] {
        let target = format!("{root}/{name}");
        fs::create_dir_all(&target).map_err(|error| failure(step, &error))?;
        mounts::move_mount(&format!("/{name}"), &target).map_err(|errno| BootFailure {
            step,
            errno: errno.0,
        })?;
    }
    Ok(())
}

fn switch_root(root: &str) -> Result<(), BootFailure> {
    let step = BootStep::Pivot;
    std::env::set_current_dir(root).map_err(|error| failure(step, &error))?;
    mounts::move_mount(".", "/").map_err(|errno| BootFailure {
        step,
        errno: errno.0,
    })?;
    mounts::chroot(".").map_err(|errno| BootFailure {
        step,
        errno: errno.0,
    })?;
    std::env::set_current_dir(Path::new("/")).map_err(|error| failure(step, &error))
}

pub(super) fn failure(step: BootStep, error: &std::io::Error) -> BootFailure {
    BootFailure {
        step,
        errno: error.raw_os_error().unwrap_or(0),
    }
}
