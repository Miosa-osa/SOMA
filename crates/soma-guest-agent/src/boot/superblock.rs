//! Superblock identity checks for the two virtio block devices before they are mounted.

use std::fs;
use std::io::{Read, Seek, SeekFrom};

use super::{BootFailure, BootStep};

const SUPERBLOCK_OFFSET: u64 = 1024;
const SUPERBLOCK_SIZE: usize = 1024;
const EROFS_MAGIC: u32 = 0xE0F5_E1E2;
const EXT4_MAGIC: u16 = 0xEF53;
const EXT4_VALID_FS: u16 = 0x0001;
const EXT4_ERROR_FS: u16 = 0x0002;

pub(super) fn verify_superblock(
    step: BootStep,
    device: &str,
    accept: fn(&[u8]) -> bool,
) -> Result<(), BootFailure> {
    let superblock = read_superblock(device).map_err(|errno| BootFailure { step, errno })?;
    accept(&superblock)
        .then_some(())
        .ok_or(BootFailure { step, errno: 0 })
}

/// What one superblock read says about the device behind it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Identity {
    /// A filesystem this boot accepts.
    Filesystem,
    /// The device has no capacity to read: the placeholder a Generation that declares a volume is
    /// captured with, which carries nothing and therefore has nothing to mount.
    Placeholder,
    /// A device this boot refuses, with the errno to report: zero when the bytes were readable and
    /// only their content was wrong, otherwise the errno that stopped the read.
    Refused(i32),
}

/// Reads the superblock of one block device, or the errno that stopped the read.
pub(super) fn read_superblock(device: &str) -> Result<[u8; SUPERBLOCK_SIZE], i32> {
    let mut file = fs::File::open(device).map_err(errno_of)?;
    file.seek(SeekFrom::Start(SUPERBLOCK_OFFSET))
        .map_err(errno_of)?;
    let mut superblock = [0; SUPERBLOCK_SIZE];
    file.read_exact(&mut superblock).map_err(errno_of)?;
    Ok(superblock)
}

/// Classifies one superblock read.
///
/// A read that fails because the device has no capacity to read is the capture placeholder rather
/// than a fault: sector 0 of a device backed by nothing reads past its end, which is `EIO`. Every
/// other outcome is the caller's `accept` test or a refusal, so a real volume that lost its
/// superblock still fails its boot instead of booting a sandbox without its data.
pub(super) fn classify_identity(
    read: Result<[u8; SUPERBLOCK_SIZE], i32>,
    accept: fn(&[u8]) -> bool,
) -> Identity {
    match read {
        Ok(superblock) if accept(&superblock) => Identity::Filesystem,
        Ok(_) => Identity::Refused(0),
        Err(errno) if empty_device(errno) => Identity::Placeholder,
        Err(errno) => Identity::Refused(errno),
    }
}

/// Whether one errno means the device has no capacity to read.
///
/// A block device backed by nothing reads past its end, which is `EIO`; `ENXIO` is the same
/// condition reported by a device that was never attached. Both are the placeholder. A readable
/// but wrong superblock is not: it is a volume that lost its filesystem.
pub(super) const fn empty_device(errno: i32) -> bool {
    matches!(errno, libc::EIO | libc::ENXIO)
}

fn errno_of(error: std::io::Error) -> i32 {
    error.raw_os_error().unwrap_or(0)
}

/// Accepts a superblock whose first four bytes carry the EROFS magic.
pub(super) fn erofs_superblock_ok(superblock: &[u8]) -> bool {
    superblock
        .get(..4)
        .and_then(|bytes| bytes.try_into().ok())
        .is_some_and(|bytes| u32::from_le_bytes(bytes) == EROFS_MAGIC)
}

/// Accepts an ext4 superblock with the magic, a valid state, and no recorded error.
pub(super) fn ext4_superblock_ok(superblock: &[u8]) -> bool {
    let field = |offset: usize| -> Option<u16> {
        superblock
            .get(offset..offset + 2)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u16::from_le_bytes)
    };
    field(0x38) == Some(EXT4_MAGIC)
        && field(0x3A).is_some_and(|state| state & EXT4_VALID_FS != 0 && state & EXT4_ERROR_FS == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_with_no_capacity_is_the_capture_placeholder() {
        // Sector 0 of a device backed by nothing reads past its end, which is EIO; that is the
        // placeholder a capture declares, and the boot continues without a volume rather than
        // failing at VolumeIdentity.
        assert_eq!(
            classify_identity(Err(libc::EIO), ext4_superblock_ok),
            Identity::Placeholder
        );
        assert_eq!(
            classify_identity(Err(libc::ENXIO), ext4_superblock_ok),
            Identity::Placeholder
        );
    }

    #[test]
    fn a_readable_device_without_a_filesystem_is_refused() {
        // Sized but empty: a placeholder attached at the class capacity, or a volume whose
        // superblock is gone. Neither may be skipped, because skipping boots a sandbox whose
        // persistent paths are silently the ephemeral ones.
        assert_eq!(
            classify_identity(Ok([0; SUPERBLOCK_SIZE]), ext4_superblock_ok),
            Identity::Refused(0)
        );
    }

    #[test]
    fn a_device_that_fails_for_another_reason_is_refused_with_its_errno() {
        assert_eq!(
            classify_identity(Err(libc::EPERM), ext4_superblock_ok),
            Identity::Refused(libc::EPERM)
        );
    }

    #[test]
    fn a_valid_superblock_is_a_filesystem() {
        let mut superblock = [0; SUPERBLOCK_SIZE];
        superblock[0x38..0x3A].copy_from_slice(&EXT4_MAGIC.to_le_bytes());
        superblock[0x3A..0x3C].copy_from_slice(&EXT4_VALID_FS.to_le_bytes());
        assert_eq!(
            classify_identity(Ok(superblock), ext4_superblock_ok),
            Identity::Filesystem
        );
    }

    #[test]
    fn erofs_identity_requires_the_exact_magic() {
        let mut superblock = [0; SUPERBLOCK_SIZE];
        assert!(!erofs_superblock_ok(&superblock));
        superblock[..4].copy_from_slice(&EROFS_MAGIC.to_le_bytes());
        assert!(erofs_superblock_ok(&superblock));
        assert!(!erofs_superblock_ok(&superblock[..3]));
    }

    #[test]
    fn ext4_identity_requires_magic_and_a_clean_error_free_state() {
        let mut superblock = [0; SUPERBLOCK_SIZE];
        assert!(!ext4_superblock_ok(&superblock));
        superblock[0x38..0x3A].copy_from_slice(&EXT4_MAGIC.to_le_bytes());
        assert!(!ext4_superblock_ok(&superblock));
        superblock[0x3A..0x3C].copy_from_slice(&EXT4_VALID_FS.to_le_bytes());
        assert!(ext4_superblock_ok(&superblock));
        superblock[0x3A..0x3C].copy_from_slice(&(EXT4_VALID_FS | EXT4_ERROR_FS).to_le_bytes());
        assert!(!ext4_superblock_ok(&superblock));
    }
}
