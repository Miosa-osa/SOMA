//! The objects of a Suspension (ADR 0034), which are not hashed: the store authenticates them.

use std::fs::File;
use std::io::{Seek as _, SeekFrom};
use std::path::PathBuf;

use super::{Artifact, SnapshotError, SnapshotPaths, Staging};

impl Staging {
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
