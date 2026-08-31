//! Bounded early-init sequence from the Generation compiler contract.
//!
//! The agent is `/init` of the deterministic initramfs, so it performs the sequence itself:
//! mount devtmpfs, procfs, and sysfs, wait for exactly the two virtio block devices, verify and
//! mount the EROFS lower and the private ext4 upper, compose `OverlayFS`, and switch into the
//! composed root.
//! The guest holds no immutable authentication secret: the responder static secret is fresh
//! per Instance and arrives later through the launch page.
//! Every step is typed and every failure is reported with the step and errno before poweroff.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::mounts::{self, Errno};

use self::devices::wait_for_devices;
use self::superblock::{erofs_superblock_ok, ext4_superblock_ok, verify_superblock};

mod devices;
mod superblock;

/// First virtio-blk device: the immutable EROFS Generation root.
pub const ROOT_DEVICE: &str = "/dev/vda";
/// Second virtio-blk device: the Instance-private ext4 overlay head.
pub const OVERLAY_DEVICE: &str = "/dev/vdb";
/// Upper bound for the complete early-init sequence.
pub const BOOT_BUDGET: Duration = Duration::from_secs(10);

const LOWER_MOUNT: &str = "/mnt/lower";
const UPPER_MOUNT: &str = "/mnt/upper";
const UPPER_DIR: &str = "/mnt/upper/upper";
const WORK_DIR: &str = "/mnt/upper/work";
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
    /// Wait for exactly the expected virtio block devices.
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
    /// Move `/dev`, `/proc`, and `/sys` into the composed root.
    MoveMounts,
    /// Move the composed root over `/` and enter it.
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
/// # Errors
///
/// Returns the first failed step; the caller must report it and power off.
pub fn early_init(deadline: Instant) -> Result<(), BootFailure> {
    mount_pseudo(
        BootStep::Devtmpfs,
        "devtmpfs",
        "/dev",
        "devtmpfs",
        "mode=0755",
    )?;
    // Every pseudo-terminal slave lives on devpts, and `/dev/ptmx` cannot allocate a pair
    // without it, so a guest with no devpts has the terminal protocol and no terminal to serve
    // it with. It is mounted here, under `/dev`, so the move below carries it into the composed
    // root with the rest of the device tree.
    mount_pseudo(
        BootStep::Devpts,
        "devpts",
        "/dev/pts",
        "devpts",
        "mode=0620,ptmxmode=0666",
    )?;
    mount_pseudo(BootStep::Procfs, "proc", "/proc", "proc", "")?;
    mount_pseudo(BootStep::Sysfs, "sysfs", "/sys", "sysfs", "")?;
    wait_for_devices(deadline)?;
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
    verify_superblock(BootStep::UpperIdentity, OVERLAY_DEVICE, ext4_superblock_ok)?;
    fs::create_dir_all(UPPER_MOUNT).map_err(|error| failure(BootStep::UpperMount, &error))?;
    mounts::mount(
        OVERLAY_DEVICE,
        UPPER_MOUNT,
        "ext4",
        libc::MS_NOSUID | libc::MS_NODEV,
        "errors=remount-ro",
    )
    .map_err(|errno| BootFailure {
        step: BootStep::UpperMount,
        errno: errno.0,
    })?;
    prepare_upper_directories()?;
    fs::create_dir_all(ROOT_MOUNT).map_err(|error| failure(BootStep::Overlay, &error))?;
    mounts::mount(
        "overlay",
        ROOT_MOUNT,
        "overlay",
        libc::MS_NOSUID | libc::MS_NODEV,
        &format!("lowerdir={LOWER_MOUNT},upperdir={UPPER_DIR},workdir={WORK_DIR}"),
    )
    .map_err(|errno| BootFailure {
        step: BootStep::Overlay,
        errno: errno.0,
    })?;
    move_mounts()?;
    switch_root()
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

/// Creates or verifies the `upper` and `work` directories on the sterile head.
///
/// The Generation compiler publishes overlay templates that already carry both directories
/// empty, so a head may contain exactly `lost+found`, `upper`, and `work`; anything else, or
/// a non-empty or non-directory `upper` or `work`, is tenant state or tampering and fails.
fn prepare_upper_directories() -> Result<(), BootFailure> {
    let step = BootStep::UpperDirectories;
    let entries = fs::read_dir(UPPER_MOUNT)
        .map_err(|error| failure(step, &error))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| failure(step, &error))?;
    if !entries
        .iter()
        .all(|name| matches!(name.as_str(), "lost+found" | "upper" | "work"))
    {
        return Err(BootFailure {
            step,
            errno: libc::EEXIST,
        });
    }
    for directory in [UPPER_DIR, WORK_DIR] {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.is_dir() => {
                let mut children =
                    fs::read_dir(directory).map_err(|error| failure(step, &error))?;
                if children.next().is_some() {
                    return Err(BootFailure {
                        step,
                        errno: libc::ENOTEMPTY,
                    });
                }
            }
            Ok(_) => {
                return Err(BootFailure {
                    step,
                    errno: libc::ENOTDIR,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(directory).map_err(|error| failure(step, &error))?;
            }
            Err(error) => return Err(failure(step, &error)),
        }
    }
    Ok(())
}

fn move_mounts() -> Result<(), BootFailure> {
    let step = BootStep::MoveMounts;
    for name in ["dev", "proc", "sys"] {
        let target = format!("{ROOT_MOUNT}/{name}");
        fs::create_dir_all(&target).map_err(|error| failure(step, &error))?;
        mounts::move_mount(&format!("/{name}"), &target).map_err(|errno| BootFailure {
            step,
            errno: errno.0,
        })?;
    }
    Ok(())
}

fn switch_root() -> Result<(), BootFailure> {
    let step = BootStep::Pivot;
    std::env::set_current_dir(ROOT_MOUNT).map_err(|error| failure(step, &error))?;
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
