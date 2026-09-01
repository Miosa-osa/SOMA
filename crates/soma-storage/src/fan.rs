//! Independent physical copies of one overlay template.
//!
//! `FICLONE` from a template updates the refcount btree record covering the template's extents,
//! and that update is one exclusive section per extent. A template that is one fully allocated
//! extent therefore hands out heads strictly one at a time: at concurrency one hundred, kernel
//! stacks showed ninety nine threads queued while one updated the allocation group, and the
//! clone phase cost 3060 us against one template and 96 us against four independent copies.
//!
//! Independent means physically independent. Giving every caller its own source inode made the
//! contention worse, because the copies still shared the one set of physical extents. A fan is
//! therefore made by copying bytes, never by reflinking them, and every replica is checked to
//! own its extents before it is published.
//!
//! It also means spread across allocation groups. The exclusive section a clone takes is the
//! allocation group's refcount btree, reached through that group's AGF buffer, so four copies
//! inside one group queue on one lock however many extents they own. XFS chooses a group by the
//! parent directory, so every replica is given a directory of its own: four copies written into
//! one directory landed in group 22 on eval-1, and four written into four directories landed in
//! groups 6, 7, 8, and 9.
//!
//! Warming a fan is a prepare-time operation: it writes one template's bytes per copy and reads
//! them back to prove them. The launch path only opens a replica that is already there, and
//! falls back to the template itself when the fan is absent, incomplete, or stale.

use std::fs::File;
use std::io::{self, Read as _, Seek as _, SeekFrom, Write as _};
use std::num::NonZeroUsize;
use std::os::fd::AsFd as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest as _, Sha256};

use crate::fiemap;

mod error;

pub use error::FanError;

#[cfg(test)]
#[path = "fan/tests.rs"]
mod tests;

/// Copies per template used when an operator names no other, measured as `t4`.
pub const DEFAULT_TEMPLATE_COPIES: usize = 4;

/// Highest copy count accepted; each copy costs one template's bytes on the reflink volume.
pub const MAX_TEMPLATE_COPIES: usize = 16;

/// Bytes moved per read and write while a replica is written.
const COPY_CHUNK: usize = 4 * 1024 * 1024;

/// What one warming pass did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanReport {
    /// The fan directory name this template's copies live under.
    pub key: String,
    /// Copies the fan is required to hold.
    pub copies: usize,
    /// Copies this pass wrote, the rest having already been present and proved.
    pub written: usize,
}

/// The directory name one template's copies live under.
///
/// The name is the template's identity on the host filesystem rather than its content digest,
/// so a snapshot's overlay and a store artifact are keyed the same way and a replaced template
/// keys somewhere else instead of being silently reused.
///
/// # Errors
///
/// Returns the failure to read the template's metadata.
pub fn fan_key(template: &File) -> io::Result<String> {
    let metadata = template.metadata()?;
    Ok(format!(
        "{:x}-{:x}-{:x}-{:x}",
        metadata.dev(),
        metadata.ino(),
        metadata.size(),
        metadata.mtime_nsec().max(0),
    ))
}

/// Materializes `copies` independent physical copies of `template` under `root`.
///
/// A copy that is already present, the right size, digests to the template, and owns its own
/// extents is kept. Anything else is written again. A copy is written to a temporary name and
/// renamed only once it has been proved, so a fan never publishes a replica a launch could
/// clone from and get wrong bytes.
///
/// # Errors
///
/// Returns the first copy that could not be written or proved.
pub fn warm(template: &File, root: &Path, copies: NonZeroUsize) -> Result<FanReport, FanError> {
    let copies = copies.get().min(MAX_TEMPLATE_COPIES);
    let key = fan_key(template).map_err(FanError::Io)?;
    let directory = root.join(&key);
    std::fs::create_dir_all(&directory).map_err(FanError::Io)?;
    let digest = digest_of(template)?;
    let size = template.metadata().map_err(FanError::Io)?.size();
    let mut written = 0;
    for index in 0..copies {
        let path = directory.join(replica_path(index));
        if proved(&path, size, digest).unwrap_or(false) {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(FanError::Io)?;
        }
        write_replica(template, &path, size, digest)?;
        written += 1;
    }
    Ok(FanReport {
        key,
        copies,
        written,
    })
}

/// Opens one replica of `template` from the fan under `root`, or nothing.
///
/// This is the launch path, so it never writes, never hashes, and never fails: an absent or
/// incomplete fan means the caller clones the template itself and pays the serialized cost it
/// would have paid anyway.
#[must_use]
pub fn open_replica(template: &File, root: &Path, copies: NonZeroUsize) -> Option<File> {
    let copies = copies.get().min(MAX_TEMPLATE_COPIES);
    let size = template.metadata().ok()?.size();
    let directory = root.join(fan_key(template).ok()?);
    let start = next_index(copies);
    for offset in 0..copies {
        let index = (start + offset) % copies;
        let replica = File::open(directory.join(replica_path(index))).ok();
        if let Some(replica) = replica
            && replica.metadata().ok()?.size() == size
        {
            return Some(replica);
        }
    }
    None
}

/// The path of one copy relative to its fan directory.
///
/// Each copy has a directory of its own, because XFS picks an allocation group per directory and
/// copies that share a group also share the lock a clone takes.
#[must_use]
pub fn replica_path(index: usize) -> PathBuf {
    PathBuf::from(format!("copy-{index:02}")).join("template")
}

/// The next replica index for this process, spread the way head shards are.
fn next_index(copies: usize) -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let counter = NEXT.fetch_add(1, Ordering::Relaxed);
    crate::shard::process_offset().wrapping_add(counter) % copies
}

/// Writes one replica by copying bytes, proves it, and only then gives it its name.
fn write_replica(
    template: &File,
    path: &Path,
    size: u64,
    digest: [u8; 32],
) -> Result<(), FanError> {
    let temporary = temporary_path(path);
    let outcome = write_and_prove(template, &temporary, size, digest);
    if outcome.is_err() {
        let _ignored = std::fs::remove_file(&temporary);
        return outcome;
    }
    std::fs::rename(&temporary, path).map_err(FanError::Io)
}

fn write_and_prove(
    template: &File,
    temporary: &Path,
    size: u64,
    digest: [u8; 32],
) -> Result<(), FanError> {
    let mut source = template.try_clone().map_err(FanError::Io)?;
    source.seek(SeekFrom::Start(0)).map_err(FanError::Io)?;
    let mut destination = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary)
        .map_err(FanError::Io)?;
    // `std::io::copy` between two files reaches for `copy_file_range`, which XFS may serve by
    // sharing extents. That would produce a replica with its own inode and the template's
    // physical blocks, which is the shape measured to be worse than no fan at all, so the
    // bytes are moved through user space where nothing can share them.
    let mut buffer = vec![0_u8; COPY_CHUNK];
    loop {
        let read = source.read(&mut buffer).map_err(FanError::Io)?;
        if read == 0 {
            break;
        }
        destination
            .write_all(&buffer[..read])
            .map_err(FanError::Io)?;
    }
    destination.sync_all().map_err(FanError::Io)?;
    drop(destination);
    let replica = File::open(temporary).map_err(FanError::Io)?;
    prove(&replica, size, digest)
}

/// Whether a published replica is still the template, byte for byte and extent for extent.
fn proved(path: &Path, size: u64, digest: [u8; 32]) -> Result<bool, FanError> {
    match File::open(path) {
        Ok(replica) => prove(&replica, size, digest).map(|()| true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(FanError::Io(error)),
    }
}

fn prove(replica: &File, size: u64, digest: [u8; 32]) -> Result<(), FanError> {
    let actual = replica.metadata().map_err(FanError::Io)?.size();
    if actual != size {
        return Err(FanError::SizeMismatch {
            expected: size,
            actual,
        });
    }
    if digest_of(replica)? != digest {
        return Err(FanError::DigestMismatch);
    }
    let extents = fiemap::summarize(replica.as_fd()).map_err(FanError::Io)?;
    if extents.shared_extents != 0 {
        return Err(FanError::SharedExtents {
            shared: extents.shared_extents,
            extents: extents.extents,
        });
    }
    Ok(())
}

fn digest_of(file: &File) -> Result<[u8; 32], FanError> {
    let mut file = file.try_clone().map_err(FanError::Io)?;
    file.seek(SeekFrom::Start(0)).map_err(FanError::Io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_CHUNK];
    loop {
        let read = file.read(&mut buffer).map_err(FanError::Io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(hasher.finalize().as_ref());
    Ok(digest)
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".partial-{}", std::process::id()));
    PathBuf::from(name)
}
