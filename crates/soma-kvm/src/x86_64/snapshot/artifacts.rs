//! Publishing and opening the three snapshot objects.
//!
//! Every object is written to a private staging name, flushed to stable storage, hashed
//! through the handle that wrote it, and only then published under its certified name with a
//! link that fails when the name already exists. The state manifest is published last, so a
//! directory that contains it contains everything it names.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
};

use super::error::{Artifact, SnapshotError};
use crate::snapshot::{Digest, Hasher};

/// Bytes moved per read or write; large enough to amortise syscalls, small enough to bound
/// the host buffer regardless of the object size.
const CHUNK: usize = 1 << 20;

/// The certified names of one snapshot directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotPaths {
    directory: PathBuf,
}

impl SnapshotPaths {
    /// Names a snapshot directory; the directory itself is created by the capture.
    #[must_use]
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The guest-memory object: page-aligned and exactly the certified size.
    #[must_use]
    pub fn memory(&self) -> PathBuf {
        self.directory.join("memory.raw")
    }

    /// The quiesced private overlay head, which every restore clones as its sterile template.
    #[must_use]
    pub fn overlay(&self) -> PathBuf {
        self.directory.join("overlay.raw")
    }

    /// The canonical state manifest, published last.
    #[must_use]
    pub fn state(&self) -> PathBuf {
        self.directory.join("state.somasnap")
    }

    /// The private name an object is written under before it is published.
    ///
    /// The immutable root and the directory itself are never staged: the root is an input
    /// this snapshot only hashes, and the directory is the thing being published into.
    fn staging(&self, artifact: Artifact) -> PathBuf {
        self.directory.join(match artifact {
            Artifact::Memory => "memory.raw.staging",
            Artifact::Overlay => "overlay.raw.staging",
            Artifact::State | Artifact::Root | Artifact::Directory => "state.somasnap.staging",
        })
    }
}

/// One staging object being written, hashed, and published.
pub(super) struct Staging {
    artifact: Artifact,
    path: PathBuf,
    published: PathBuf,
    file: File,
    written: u64,
    running: Hasher,
    /// Whether this object is hashed. A Suspension's members are not: the store authenticates
    /// every byte of them under the Suspension's key, so a digest here would only cost the read.
    hashed: bool,
}

/// The digest an unhashed object states in place of one, so no record carries a zero digest.
pub(super) const UNHASHED: Digest = Digest::from_bytes(*b"soma.suspension.member.unhashed.");

impl Staging {
    /// Creates the staging object, replacing any leftover from an abandoned capture.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::AlreadyPublished`] when the certified name already exists,
    /// or the staging-create failure.
    pub(super) fn create(
        paths: &SnapshotPaths,
        artifact: Artifact,
        published: PathBuf,
    ) -> Result<Self, SnapshotError> {
        if published.exists() {
            return Err(SnapshotError::AlreadyPublished(artifact));
        }
        let path = paths.staging(artifact);
        let _ignored = fs::remove_file(&path);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| SnapshotError::io(artifact, "create staging object", &error))?;
        Ok(Self {
            artifact,
            path,
            published,
            file,
            written: 0,
            running: Hasher::new(),
            hashed: true,
        })
    }

    /// Creates a staging object that is not hashed (see [`UNHASHED`]).
    ///
    /// # Errors
    ///
    /// As [`Staging::create`].
    pub(super) fn create_unhashed(
        paths: &SnapshotPaths,
        artifact: Artifact,
        published: PathBuf,
    ) -> Result<Self, SnapshotError> {
        let mut staging = Self::create(paths, artifact, published)?;
        staging.hashed = false;
        Ok(staging)
    }

    /// Makes this object a copy of all of `source` by sharing its extents, which costs no read
    /// and no write on a filesystem that clones; elsewhere it copies.
    ///
    /// # Errors
    ///
    /// Returns the read, write, or stat failure.
    pub(super) fn clone_file(&mut self, source: &mut File) -> Result<(), SnapshotError> {
        if !self.hashed && self.written == 0 && clone_extents(&self.file, source) {
            let len = source
                .metadata()
                .map_err(|error| SnapshotError::io(self.artifact, "stat source", &error))?
                .len();
            self.file
                .seek(SeekFrom::Start(len))
                .map_err(|error| SnapshotError::io(self.artifact, "seek", &error))?;
            self.written = len;
            return Ok(());
        }
        self.write_file(source)
    }

    /// Appends bytes to the staging object.
    ///
    /// # Errors
    ///
    /// Returns the write failure.
    pub(super) fn write(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        // A run of zeros is left as a hole rather than written: guest memory and a private head
        // are mostly zeros, and writing them is most of what a capture costs. The length is set
        // at the end, so a trailing hole still reads back as the zeros it stands for.
        if bytes.iter().all(|byte| *byte == 0) {
            self.file
                .seek(SeekFrom::Current(
                    i64::try_from(bytes.len()).unwrap_or(i64::MAX),
                ))
                .map_err(|error| SnapshotError::io(self.artifact, "seek", &error))?;
        } else {
            self.file
                .write_all(bytes)
                .map_err(|error| SnapshotError::io(self.artifact, "write", &error))?;
        }
        if self.hashed {
            self.running.update(bytes);
        }
        self.written = self
            .written
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(0));
        Ok(())
    }

    /// Copies the whole of `source` from its start.
    ///
    /// # Errors
    ///
    /// Returns the seek, read, or write failure.
    pub(super) fn write_file(&mut self, source: &mut File) -> Result<(), SnapshotError> {
        source
            .seek(SeekFrom::Start(0))
            .map_err(|error| SnapshotError::io(self.artifact, "rewind source", &error))?;
        let mut buffer = vec![0_u8; CHUNK];
        loop {
            let count = source
                .read(&mut buffer)
                .map_err(|error| SnapshotError::io(self.artifact, "read source", &error))?;
            if count == 0 {
                return Ok(());
            }
            self.write(&buffer[..count])?;
        }
    }

    /// Bytes written so far.
    pub(super) const fn written(&self) -> u64 {
        self.written
    }

    /// The digest of the bytes handed to [`Staging::write`] so far.
    pub(super) fn running_digest(&self) -> Digest {
        if self.hashed {
            self.running.clone().finish()
        } else {
            UNHASHED
        }
    }

    /// Flushes to stable storage and re-hashes through the handle that did the writing.
    ///
    /// The re-read digest must equal the digest accumulated while writing; a difference
    /// means the bytes on the object are not the bytes the manifest describes.
    ///
    /// # Errors
    ///
    /// Returns the flush or read failure, or [`SnapshotError::StagingDigestMismatch`].
    pub(super) fn seal(&mut self) -> Result<Digest, SnapshotError> {
        self.file
            .set_len(self.written)
            .map_err(|error| SnapshotError::io(self.artifact, "set length", &error))?;
        self.file
            .sync_all()
            .map_err(|error| SnapshotError::io(self.artifact, "sync", &error))?;
        if !self.hashed {
            return Ok(UNHASHED);
        }
        let digest = hash(self.artifact, &mut self.file)?;
        if digest == self.running_digest() {
            Ok(digest)
        } else {
            Err(SnapshotError::StagingDigestMismatch(self.artifact))
        }
    }

    /// Publishes the certified name and removes the staging name.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::AlreadyPublished`] when the name exists, or the link failure.
    pub(super) fn link(self) -> Result<(), SnapshotError> {
        // A link fails when the certified name exists, so publication can never overwrite an
        // object another snapshot already names.
        let outcome = fs::hard_link(&self.path, &self.published).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                SnapshotError::AlreadyPublished(self.artifact)
            } else {
                SnapshotError::io(self.artifact, "publish", &error)
            }
        });
        let _ignored = fs::remove_file(&self.path);
        outcome
    }
}

/// Hashes a whole file from its start through the given handle.
///
/// # Errors
///
/// Returns the seek or read failure.
pub(super) fn hash(artifact: Artifact, file: &mut File) -> Result<Digest, SnapshotError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| SnapshotError::io(artifact, "rewind", &error))?;
    let mut hasher = Hasher::new();
    let mut buffer = vec![0_u8; CHUNK];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| SnapshotError::io(artifact, "read", &error))?;
        if count == 0 {
            return Ok(hasher.finish());
        }
        hasher.update(&buffer[..count]);
    }
}

/// Flushes the directory entries so a published name survives a crash.
///
/// # Errors
///
/// Returns the open or sync failure.
pub(super) fn sync_directory(directory: &Path) -> Result<(), SnapshotError> {
    let handle = File::open(directory)
        .map_err(|error| SnapshotError::io(Artifact::Directory, "open", &error))?;
    handle
        .sync_all()
        .map_err(|error| SnapshotError::io(Artifact::Directory, "sync", &error))
}

/// Shares all of `source`'s extents into `target` with `FICLONE`; `false` when the filesystem
/// cannot, in which case nothing was changed.
#[allow(unsafe_code)]
fn clone_extents(target: &File, source: &File) -> bool {
    use std::os::fd::AsRawFd as _;
    // SAFETY: both descriptors are open files this process owns for the length of the call, and
    // FICLONE reads nothing from user memory beyond the source descriptor passed by value.
    let rc = unsafe { libc::ioctl(target.as_raw_fd(), libc::FICLONE, source.as_raw_fd()) };
    rc == 0
}
