//! Member encryption at rest under one Suspension's data key.
//!
//! A member is cut into [`CHUNK`]-byte plaintext chunks, each sealed with ChaCha20-Poly1305 (the
//! cipher the guest protocol and the volume keyring already use, so no crate is added). The nonce
//! is the member role and the chunk index, so no nonce repeats under one key, and the associated
//! data binds the Suspension, the role, the index and whether the chunk is the last: a chunk moved
//! to another position, another member or another Suspension does not open, and a member cut short
//! or extended fails on its missing or extra final chunk. An empty member is one empty final chunk.
//!
//! Opening also checks the plaintext's length and SHA-256 against the header's record, so what a
//! resume maps is exactly what the suspension wrote.
//!
//! The data key is 32 random bytes per Suspension, zeroized when dropped; sealing it under the
//! host key and destroying it with the Suspension is the store's job, as it is for volume keys.

use std::io::{self, Read, Write};

use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead as _, KeyInit as _, Payload},
};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroize as _;

use super::{Member, MemberRecord};
use crate::ids::SuspensionId;

/// Plaintext bytes per sealed chunk.
pub const CHUNK: usize = 1 << 20;
const TAG: usize = 16;

/// One Suspension's data key.
pub struct DataKey([u8; 32]);

impl DataKey {
    /// A fresh random key.
    ///
    /// # Errors
    ///
    /// Returns the operating system's refusal to produce random bytes.
    pub fn generate() -> io::Result<Self> {
        let mut key = [0_u8; 32];
        getrandom::getrandom(&mut key).map_err(|error| io::Error::other(error.to_string()))?;
        Ok(Self(key))
    }

    /// A key from bytes the store unsealed.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new((&self.0).into())
    }
}

impl Drop for DataKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug)]
pub enum SealError {
    Io(io::Error),
    /// A chunk did not authenticate: wrong key, wrong position, or altered bytes.
    Corrupt,
    /// The member ended before its final chunk, or continued after it.
    Truncated,
    /// The plaintext's length or digest differs from the header's record.
    Mismatch,
}

impl From<io::Error> for SealError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for SealError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "suspension member i/o: {error}"),
            Self::Corrupt => formatter.write_str("suspension member chunk does not authenticate"),
            Self::Truncated => formatter.write_str("suspension member is cut short or extended"),
            Self::Mismatch => {
                formatter.write_str("suspension member differs from its header record")
            }
        }
    }
}

impl std::error::Error for SealError {}

fn nonce(member: Member, index: u64) -> [u8; 12] {
    let mut nonce = [0_u8; 12];
    nonce[0] = member as u8;
    nonce[4..].copy_from_slice(&index.to_be_bytes());
    nonce
}

fn aad(id: SuspensionId, member: Member, index: u64, last: bool) -> [u8; 26] {
    let mut aad = [0_u8; 26];
    aad[..16].copy_from_slice(id.as_bytes());
    aad[16] = member as u8;
    aad[17..25].copy_from_slice(&index.to_be_bytes());
    aad[25] = u8::from(last);
    aad
}

/// Fills `buffer` from `source` as far as it can, returning how much it read.
fn fill(source: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match source.read(&mut buffer[filled..])? {
            0 => break,
            count => filled += count,
        }
    }
    Ok(filled)
}

/// Seals one member from `plain` into `sealed`, and returns the header's record of it.
///
/// # Errors
///
/// Returns [`SealError::Io`] for a read or write failure; nothing else can fail.
pub fn seal_member(
    key: &DataKey,
    id: SuspensionId,
    member: Member,
    plain: &mut impl Read,
    sealed: &mut impl Write,
) -> Result<MemberRecord, SealError> {
    let cipher = key.cipher();
    let mut digest = Sha256::new();
    let mut total: u64 = 0;
    let mut current = vec![0_u8; CHUNK];
    let mut next = vec![0_u8; CHUNK];
    let mut current_len = fill(plain, &mut current)?;
    let mut index: u64 = 0;
    loop {
        // A chunk is the last when nothing follows it; reading ahead is how that is known.
        let next_len = if current_len == CHUNK {
            fill(plain, &mut next)?
        } else {
            0
        };
        let last = next_len == 0;
        let chunk = &current[..current_len];
        digest.update(chunk);
        total += current_len as u64;
        let payload = Payload {
            msg: chunk,
            aad: &aad(id, member, index, last),
        };
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce(member, index)), payload)
            .map_err(|_| SealError::Corrupt)?;
        sealed.write_all(&ciphertext)?;
        if last {
            break;
        }
        std::mem::swap(&mut current, &mut next);
        current_len = next_len;
        index += 1;
    }
    sealed.flush()?;
    Ok(MemberRecord {
        bytes: total,
        sha256: digest.finalize().into(),
    })
}

/// Opens one sealed member into `plain`, checking it against the header's `record`.
///
/// # Errors
///
/// Returns [`SealError::Corrupt`] for a chunk that does not authenticate, [`SealError::Truncated`]
/// for a member without its final chunk or with bytes after it, and [`SealError::Mismatch`] when
/// the plaintext's length or digest is not the recorded one. Bytes already written to `plain`
/// before a failure must be discarded by the caller.
pub fn open_member(
    key: &DataKey,
    id: SuspensionId,
    member: Member,
    record: &MemberRecord,
    sealed: &mut impl Read,
    plain: &mut impl Write,
) -> Result<(), SealError> {
    let cipher = key.cipher();
    let mut digest = Sha256::new();
    let mut total: u64 = 0;
    let mut buffer = vec![0_u8; CHUNK + TAG];
    let mut index: u64 = 0;
    loop {
        let read = fill(sealed, &mut buffer)?;
        if read < TAG {
            return Err(SealError::Truncated);
        }
        // A full sealed chunk is not the last unless the record says the member ends exactly here.
        let remaining = record.bytes.saturating_sub(total);
        let last = remaining <= CHUNK as u64;
        let payload = Payload {
            msg: &buffer[..read],
            aad: &aad(id, member, index, last),
        };
        let chunk = cipher
            .decrypt(Nonce::from_slice(&nonce(member, index)), payload)
            .map_err(|_| SealError::Corrupt)?;
        digest.update(&chunk);
        total += chunk.len() as u64;
        plain.write_all(&chunk)?;
        if last {
            let mut extra = [0_u8; 1];
            if fill(sealed, &mut extra)? != 0 {
                return Err(SealError::Truncated);
            }
            break;
        }
        index += 1;
    }
    plain.flush()?;
    let sha256: [u8; 32] = digest.finalize().into();
    if total != record.bytes || sha256 != record.sha256 {
        return Err(SealError::Mismatch);
    }
    Ok(())
}
