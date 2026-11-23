//! The durable record of one workspace restore: the fingerprint and the three inodes it moves.

use std::{io::Read as _, path::Path};

use super::{IO_FAILURE, VolumeError, file::private_file};

pub(super) const INTENT: &str = "restore-intent";
pub(super) const INTENT_BYTES: usize = 56;

#[derive(Clone, Copy)]
pub(super) struct Intent {
    pub(super) fingerprint: [u8; 32],
    pub(super) original: u64,
    pub(super) replacement: u64,
    pub(super) transition: u64,
}

impl Intent {
    pub(super) fn read(path: &Path, fingerprint: [u8; 32]) -> Result<Self, VolumeError> {
        let mut bytes = [0; INTENT_BYTES];
        let mut file = private_file(&path.join(INTENT), false)?;
        file.read_exact(&mut bytes).map_err(|_| IO_FAILURE)?;
        if file.metadata().map_err(|_| IO_FAILURE)?.len() != INTENT_BYTES as u64 {
            return Err(IO_FAILURE);
        }
        if bytes[..32] != fingerprint {
            return Err(VolumeError::ChangedIntent);
        }
        let word = |offset| {
            let mut bytes_at = [0; 8];
            bytes_at.copy_from_slice(&bytes[offset..offset + 8]);
            u64::from_be_bytes(bytes_at)
        };
        Ok(Self {
            fingerprint,
            original: word(32),
            replacement: word(40),
            transition: word(48),
        })
    }

    pub(super) fn encode(self) -> [u8; INTENT_BYTES] {
        let mut bytes = [0; INTENT_BYTES];
        bytes[..32].copy_from_slice(&self.fingerprint);
        bytes[32..40].copy_from_slice(&self.original.to_be_bytes());
        bytes[40..48].copy_from_slice(&self.replacement.to_be_bytes());
        bytes[48..].copy_from_slice(&self.transition.to_be_bytes());
        bytes
    }
}
