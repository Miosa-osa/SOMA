//! Bounded early-init sequence from the Generation compiler contract.
//!
//! The agent is `/init` of the deterministic initramfs, so it performs the sequence itself:
//! mount devtmpfs, procfs, and sysfs, wait for exactly the two virtio block devices, verify and
//! mount the EROFS lower and the private ext4 upper, compose `OverlayFS`, take the Generation
//! responder key from the initramfs, and switch into the composed root.
//! Every step is typed and every failure is reported with the step and errno before poweroff.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use soma_guest::ResponderPrivateKey;
use zeroize::Zeroizing;

use crate::mounts::{self, Errno};

use self::superblock::{erofs_superblock_ok, ext4_superblock_ok, verify_superblock};

mod superblock;

/// First virtio-blk device: the immutable EROFS Generation root.
pub const ROOT_DEVICE: &str = "/dev/vda";
/// Second virtio-blk device: the Instance-private ext4 overlay head.
pub const OVERLAY_DEVICE: &str = "/dev/vdb";
/// Path of the Generation-scoped responder private key inside the initramfs.
pub const RESPONDER_KEY_PATH: &str = "/etc/soma/responder.key";
/// Upper bound for the complete early-init sequence.
pub const BOOT_BUDGET: Duration = Duration::from_secs(10);

const LOWER_MOUNT: &str = "/mnt/lower";
const UPPER_MOUNT: &str = "/mnt/upper";
const UPPER_DIR: &str = "/mnt/upper/upper";
const WORK_DIR: &str = "/mnt/upper/work";
const ROOT_MOUNT: &str = "/mnt/root";
const EXPECTED_BLOCK_DEVICES: [&str; 2] = ["vda", "vdb"];
const DEVICE_POLL: Duration = Duration::from_millis(5);
const NOSUID_NODEV_NOEXEC: libc::c_ulong = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;

/// One typed early-init step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootStep {
    /// Mount devtmpfs on `/dev`.
    Devtmpfs,
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
    /// Read and erase the Generation responder key from the initramfs.
    ResponderKey,
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

/// Material produced by a successful boot.
pub struct BootEvidence {
    /// The Generation-scoped responder private key.
    pub responder: ResponderPrivateKey,
}

/// Performs the complete early-init sequence before the given absolute deadline.
///
/// # Errors
///
/// Returns the first failed step; the caller must report it and power off.
pub fn early_init(deadline: Instant) -> Result<BootEvidence, BootFailure> {
    mount_pseudo(
        BootStep::Devtmpfs,
        "devtmpfs",
        "/dev",
        "devtmpfs",
        "mode=0755",
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
    let responder = take_responder_key()?;
    move_mounts()?;
    switch_root()?;
    Ok(BootEvidence { responder })
}

fn mount_pseudo(
    step: BootStep,
    source: &str,
    target: &str,
    fstype: &str,
    data: &str,
) -> Result<(), BootFailure> {
    fs::create_dir_all(target).map_err(|error| failure(step, &error))?;
    let flags = if step == BootStep::Devtmpfs {
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

fn wait_for_devices(deadline: Instant) -> Result<(), BootFailure> {
    loop {
        let names = block_device_names().unwrap_or_default();
        if names == expected_block_devices()
            && is_block_device(ROOT_DEVICE)
            && is_block_device(OVERLAY_DEVICE)
        {
            return Ok(());
        }
        if names.len() > EXPECTED_BLOCK_DEVICES.len() || Instant::now() >= deadline {
            return Err(BootFailure {
                step: BootStep::Devices,
                errno: libc::ETIMEDOUT,
            });
        }
        thread::sleep(DEVICE_POLL);
    }
}

fn block_device_names() -> std::io::Result<BTreeSet<String>> {
    fs::read_dir("/sys/block")?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect()
}

fn expected_block_devices() -> BTreeSet<String> {
    EXPECTED_BLOCK_DEVICES
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

fn is_block_device(path: &str) -> bool {
    use std::os::unix::fs::FileTypeExt;
    fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_block_device())
}

fn prepare_upper_directories() -> Result<(), BootFailure> {
    let entries = fs::read_dir(UPPER_MOUNT)
        .map_err(|error| failure(BootStep::UpperDirectories, &error))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| failure(BootStep::UpperDirectories, &error))?;
    if !entries.iter().all(|name| name == "lost+found") {
        return Err(BootFailure {
            step: BootStep::UpperDirectories,
            errno: libc::EEXIST,
        });
    }
    for directory in [UPPER_DIR, WORK_DIR] {
        fs::create_dir(directory).map_err(|error| failure(BootStep::UpperDirectories, &error))?;
    }
    Ok(())
}

fn take_responder_key() -> Result<ResponderPrivateKey, BootFailure> {
    let step = BootStep::ResponderKey;
    let bytes =
        Zeroizing::new(fs::read(RESPONDER_KEY_PATH).map_err(|error| failure(step, &error))?);
    let key: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| BootFailure { step, errno: 0 })?;
    let responder = ResponderPrivateKey::new(key).map_err(|_| BootFailure { step, errno: 0 })?;
    fs::write(RESPONDER_KEY_PATH, [0_u8; 32]).map_err(|error| failure(step, &error))?;
    fs::remove_file(RESPONDER_KEY_PATH).map_err(|error| failure(step, &error))?;
    Ok(responder)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_contract_is_exactly_two_virtio_block_devices() {
        assert_eq!(ROOT_DEVICE, "/dev/vda");
        assert_eq!(OVERLAY_DEVICE, "/dev/vdb");
        assert_eq!(expected_block_devices().len(), 2);
        assert!(!is_block_device("/proc/self/exe"));
    }
}
