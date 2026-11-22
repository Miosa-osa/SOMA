//! Member encryption at rest under one Suspension's data key.
//!
//! A member is cut into [`CHUNK`]-byte plaintext chunks, each sealed with ChaCha20-Poly1305 (the
//! cipher the guest protocol and the volume keyring already use, so no crate is added). The nonce
//! is the member role and the chunk index, so no nonce repeats under one key, and the associated
//! data binds the Suspension, the role, the index, whether the chunk is the last, its kind and its
//! length: a chunk moved to another position, another member or another Suspension does not open,
//! and a member cut short or extended fails on its missing or extra final chunk. An empty member is
//! one empty final chunk.
//!
//! A chunk of zeros, which is most of guest memory and nearly all of a private head, is sealed as
//! an empty box whose associated data states its length: it costs neither encryption nor space,
//! is still authenticated in its place, and opens as a hole in the plaintext file.
//!
//! Opening also checks the plaintext's length and digest against the header's record, so what a
//! resume maps is exactly what the suspension wrote.
//!
//! The data key is 32 random bytes per Suspension, zeroized when dropped; sealing it under the
//! host key and destroying it with the Suspension is the store's job, as it is for volume keys.

use std::io::{self, Read, Seek, SeekFrom, Write};

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

/// What one sealed chunk holds: its plaintext, or only the statement that it is all zeros.
const DATA: u8 = 0;
const ZEROS: u8 = 1;

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

fn aad(id: SuspensionId, member: Member, index: u64, last: bool, kind: u8, len: usize) -> [u8; 31] {
    let mut aad = [0_u8; 31];
    aad[..16].copy_from_slice(id.as_bytes());
    aad[16] = member as u8;
    aad[17..25].copy_from_slice(&index.to_be_bytes());
    aad[25] = u8::from(last);
    aad[26] = kind;
    aad[27..31].copy_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_be_bytes());
    aad
}

/// The member digest covers each chunk's kind, position and length, and the plaintext of the
/// chunks that carry one; a chunk of zeros is bound by its kind and length alone.
fn digest_chunk(digest: &mut Sha256, kind: u8, index: u64, len: usize, data: &[u8]) {
    digest.update([kind]);
    digest.update(index.to_be_bytes());
    digest.update(u64::try_from(len).unwrap_or(u64::MAX).to_be_bytes());
    digest.update(data);
}

/// Whether every byte of `bytes` is zero.
///
/// Folded without an early exit so the compiler vectorizes it: an early-exit scan runs a byte at
/// a time, and this check runs over every byte of guest memory and the private head.
fn is_zero(bytes: &[u8]) -> bool {
    let (words, tail) = bytes.as_chunks::<8>();
    words
        .iter()
        .fold(0_u64, |acc, word| acc | u64::from_ne_bytes(*word))
        == 0
        && tail.iter().fold(0_u8, |acc, byte| acc | byte) == 0
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
        let (kind, data): (u8, &[u8]) = if is_zero(chunk) {
            (ZEROS, &[])
        } else {
            (DATA, chunk)
        };
        digest_chunk(&mut digest, kind, index, current_len, data);
        total += current_len as u64;
        let payload = Payload {
            msg: data,
            aad: &aad(id, member, index, last, kind, current_len),
        };
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce(member, index)), payload)
            .map_err(|_| SealError::Corrupt)?;
        sealed.write_all(&[kind])?;
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
/// A chunk of zeros is skipped over rather than written, so a file becomes sparse where the
/// member was zeros; the caller sets the file's length to `record.bytes`, which is what makes a
/// trailing run of zeros read back.
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
    plain: &mut (impl Write + Seek),
) -> Result<(), SealError> {
    let cipher = key.cipher();
    let mut digest = Sha256::new();
    let mut total: u64 = 0;
    let mut buffer = vec![0_u8; CHUNK + TAG];
    let mut index: u64 = 0;
    loop {
        let mut kind = [0_u8; 1];
        if fill(sealed, &mut kind)? != 1 {
            return Err(SealError::Truncated);
        }
        let kind = kind[0];
        let remaining = record.bytes.saturating_sub(total);
        let last = remaining <= CHUNK as u64;
        let len = usize::try_from(remaining.min(CHUNK as u64)).unwrap_or(CHUNK);
        let boxed = match kind {
            DATA => len + TAG,
            ZEROS => TAG,
            _ => return Err(SealError::Corrupt),
        };
        if fill(sealed, &mut buffer[..boxed])? != boxed {
            return Err(SealError::Truncated);
        }
        let payload = Payload {
            msg: &buffer[..boxed],
            aad: &aad(id, member, index, last, kind, len),
        };
        let chunk = cipher
            .decrypt(Nonce::from_slice(&nonce(member, index)), payload)
            .map_err(|_| SealError::Corrupt)?;
        digest_chunk(&mut digest, kind, index, len, &chunk);
        if kind == DATA {
            plain.write_all(&chunk)?;
        } else {
            plain.seek(SeekFrom::Current(i64::try_from(len).unwrap_or(i64::MAX)))?;
        }
        total += len as u64;
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
