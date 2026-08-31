//! The writable half of the root, for a machine that declared one.
//!
//! Everything here runs only when the Generation declared writable storage. A machine without
//! it never verifies an ext4 superblock, never mounts an upper, never creates a work directory,
//! and never composes `OverlayFS`: it switches straight into the immutable root it already
//! mounted read-only.

use std::collections::BTreeSet;
use std::fs;

use crate::mounts;

use super::superblock::{ext4_superblock_ok, verify_superblock};
use super::{BootFailure, BootStep, LOWER_MOUNT, OVERLAY_DEVICE, ROOT_MOUNT, failure};

const UPPER_MOUNT: &str = "/mnt/upper";
const UPPER_DIR: &str = "/mnt/upper/upper";
const WORK_DIR: &str = "/mnt/upper/work";

/// Verifies the private head, mounts it, and composes the writable root over the lower.
///
/// # Errors
///
/// Returns the first failed step; the caller reports it and powers off.
pub(super) fn compose() -> Result<(), BootFailure> {
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
    })
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
