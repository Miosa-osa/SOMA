//! Superblock identity checks for the two virtio block devices before they are mounted.

use std::fs;
use std::io::{Read, Seek, SeekFrom};

use super::{BootFailure, BootStep, failure};

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
    let mut file = fs::File::open(device).map_err(|error| failure(step, &error))?;
    file.seek(SeekFrom::Start(SUPERBLOCK_OFFSET))
        .map_err(|error| failure(step, &error))?;
    let mut superblock = [0; SUPERBLOCK_SIZE];
    file.read_exact(&mut superblock)
        .map_err(|error| failure(step, &error))?;
    accept(&superblock)
        .then_some(())
        .ok_or(BootFailure { step, errno: 0 })
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
