//! Conformance proofs for the reflink profile.
//!
//! [`prove_isolation`] clones one template twice, writes different patterns through both
//! clones, forces allocation with `fdatasync`, and proves that the template and each peer are
//! byte-for-byte unchanged while the clones' written extents stopped being shared.
//! [`prove_no_space`] fills a clone until the filesystem reports `ENOSPC` and proves that the
//! template survived.

use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::FileExt;

use sha2::{Digest, Sha256};

use crate::clone::{self, CloneError, ClonedHead};
use crate::fiemap::{self, ExtentSummary};
use crate::head::HeadName;

/// Bytes written per region.
const REGION_BYTES: usize = 4096;

/// Result of a successful isolation proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsolationProof {
    /// Regions written through each clone.
    pub regions: usize,
    /// Extent summary of the first clone before any write.
    pub before: ExtentSummary,
    /// Extent summary of the first clone after its writes were flushed.
    pub after: ExtentSummary,
}

/// Result of a successful out-of-space proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoSpaceProof {
    /// Bytes written into the clone before the filesystem refused more.
    pub bytes_written: u64,
}

/// Why a conformance proof failed.
#[derive(Debug)]
pub enum ConformanceError {
    /// A clone could not be created.
    Clone(CloneError),
    /// A read, write, or sync failed.
    Io(io::Error),
    /// The template changed at this offset.
    TemplateMutated {
        /// Offset of the first differing region.
        offset: u64,
    },
    /// The two clones carried the same bytes at this offset.
    ClonesIdentical {
        /// Offset of the identical region.
        offset: u64,
    },
    /// The peer clone changed at this offset.
    PeerMutated {
        /// Offset of the first differing region.
        offset: u64,
    },
    /// No extent stopped being shared after the writes.
    CopyOnWriteNotObserved,
    /// The filesystem never reported `ENOSPC`.
    NoSpaceNotReached,
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clone(error) => write!(f, "clone failed: {error}"),
            Self::Io(error) => write!(f, "conformance I/O failed: {error}"),
            Self::TemplateMutated { offset } => write!(f, "template changed at {offset}"),
            Self::ClonesIdentical { offset } => write!(f, "clones are identical at {offset}"),
            Self::PeerMutated { offset } => write!(f, "peer clone changed at {offset}"),
            Self::CopyOnWriteNotObserved => f.write_str("no extent became private"),
            Self::NoSpaceNotReached => f.write_str("ENOSPC was never reported"),
        }
    }
}

impl std::error::Error for ConformanceError {}

impl From<io::Error> for ConformanceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Region offsets that touch the first block, an early metadata block, the middle, and the
/// last block of a file of `size` bytes.
fn regions(size: u64) -> Vec<u64> {
    let last = size.saturating_sub(REGION_BYTES as u64);
    let mut offsets = vec![0, REGION_BYTES as u64 * 4, size / 2, last];
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

fn read_region(file: &File, offset: u64) -> io::Result<[u8; REGION_BYTES]> {
    let mut buffer = [0u8; REGION_BYTES];
    file.read_exact_at(&mut buffer, offset)?;
    Ok(buffer)
}

fn create(
    template: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    name: &HeadName,
) -> Result<ClonedHead, ConformanceError> {
    clone::clone_head(template, dir, name, clone::Durability::Persisted)
        .map_err(ConformanceError::Clone)
}

/// Runs the two-clone isolation proof under `dir` with head names prefixed by `prefix`.
///
/// Both heads are unlinked before returning, whether the proof passed or failed.
///
/// # Errors
///
/// Returns the first violated property.
pub fn prove_isolation(
    template: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    prefix: &HeadName,
) -> Result<IsolationProof, ConformanceError> {
    let name_a =
        HeadName::new(format!("{prefix}-a")).map_err(|e| io::Error::other(e.to_string()))?;
    let name_b =
        HeadName::new(format!("{prefix}-b")).map_err(|e| io::Error::other(e.to_string()))?;
    let result = isolation(template, dir, &name_a, &name_b);
    for name in [&name_a, &name_b] {
        if let Ok(c_name) = CString::new(name.as_str()) {
            clone::unlink_quietly(dir, &c_name);
        }
    }
    let _ = clone::fsync(dir);
    result
}

fn isolation(
    template: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    name_a: &HeadName,
    name_b: &HeadName,
) -> Result<IsolationProof, ConformanceError> {
    let template_file = File::from(template.try_clone_to_owned()?);
    let size = template_file.metadata()?.len();
    let offsets = regions(size);
    let originals: Vec<[u8; REGION_BYTES]> = offsets
        .iter()
        .map(|offset| read_region(&template_file, *offset))
        .collect::<io::Result<_>>()?;

    let head_a = create(template, dir, name_a)?;
    let before = head_a.extents();
    let head_b = create(template, dir, name_b)?;
    let file_a = File::from(head_a.into_fd());
    let file_b = File::from(head_b.into_fd());

    for (index, offset) in offsets.iter().enumerate() {
        let salt = u8::try_from(index % 256).unwrap_or(0);
        let pattern_a = [0xa5u8 ^ salt; REGION_BYTES];
        let pattern_b = [0x5au8 ^ salt; REGION_BYTES];
        file_a.write_all_at(&pattern_a, *offset)?;
        file_b.write_all_at(&pattern_b, *offset)?;
    }
    file_a.sync_data()?;
    file_b.sync_data()?;

    for (index, offset) in offsets.iter().enumerate() {
        if read_region(&template_file, *offset)? != originals[index] {
            return Err(ConformanceError::TemplateMutated { offset: *offset });
        }
        let bytes_a = read_region(&file_a, *offset)?;
        let bytes_b = read_region(&file_b, *offset)?;
        if bytes_a == bytes_b {
            return Err(ConformanceError::ClonesIdentical { offset: *offset });
        }
        let salt = u8::try_from(index % 256).unwrap_or(0);
        if bytes_b != [0x5au8 ^ salt; REGION_BYTES] {
            return Err(ConformanceError::PeerMutated { offset: *offset });
        }
    }
    let after = fiemap::summarize(file_a.as_fd())?;
    if after.shared_extents >= after.extents {
        return Err(ConformanceError::CopyOnWriteNotObserved);
    }
    Ok(IsolationProof {
        regions: offsets.len(),
        before,
        after,
    })
}

/// Clones `template` under `dir` and writes until the filesystem reports `ENOSPC`, then proves
/// the template digest is unchanged.
///
/// The head is unlinked before returning.
///
/// # Errors
///
/// Returns [`ConformanceError::NoSpaceNotReached`] if `limit_bytes` were written without an
/// `ENOSPC`, or the first other failure.
pub fn prove_no_space(
    template: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    name: &HeadName,
    limit_bytes: u64,
) -> Result<NoSpaceProof, ConformanceError> {
    let result = no_space(template, dir, name, limit_bytes);
    if let Ok(c_name) = CString::new(name.as_str()) {
        clone::unlink_quietly(dir, &c_name);
    }
    let _ = clone::fsync(dir);
    result
}

fn no_space(
    template: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    name: &HeadName,
    limit_bytes: u64,
) -> Result<NoSpaceProof, ConformanceError> {
    let template_file = File::from(template.try_clone_to_owned()?);
    let before = digest(&template_file)?;
    let head = File::from(create(template, dir, name)?.into_fd());
    let size = head.metadata()?.len();
    let chunk = vec![0xc3u8; 1 << 20];
    let mut written = 0u64;
    let mut offset = 0u64;
    let reached = loop {
        if written >= limit_bytes || offset + chunk.len() as u64 > size {
            break false;
        }
        match head
            .write_all_at(&chunk, offset)
            .and_then(|()| head.sync_data())
        {
            Ok(()) => {
                written += chunk.len() as u64;
                offset += chunk.len() as u64;
            }
            Err(error) if error.raw_os_error() == Some(libc::ENOSPC) => break true,
            Err(error) => return Err(ConformanceError::Io(error)),
        }
    };
    if !reached {
        return Err(ConformanceError::NoSpaceNotReached);
    }
    if digest(&template_file)? != before {
        return Err(ConformanceError::TemplateMutated { offset: 0 });
    }
    Ok(NoSpaceProof {
        bytes_written: written,
    })
}

fn digest(file: &File) -> io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut offset = 0u64;
    loop {
        let read = file.read_at(&mut buffer, offset)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        offset += read as u64;
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_cover_first_middle_and_last_blocks_without_duplicates() {
        let offsets = regions(64 * 1024 * 1024);
        assert_eq!(
            offsets,
            vec![0, 16384, 32 * 1024 * 1024, 64 * 1024 * 1024 - 4096]
        );
        assert_eq!(regions(4096), vec![0, 2048, 16384]);
    }
}
