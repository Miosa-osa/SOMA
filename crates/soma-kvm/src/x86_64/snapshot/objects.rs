//! The published snapshot as open handles rather than as a directory to walk.
//!
//! A restore reads exactly three published objects: the state manifest, the memory image, and,
//! where the Generation declared writable storage, the sterile overlay template. Naming them by
//! path works only for a process that can still resolve paths. The jailed machine cannot: its
//! root is empty, procfs is invisible, and `open` is a seccomp kill once the filter narrows.
//!
//! So the objects are opened by whoever still has a filesystem and handed over as descriptors.
//! [`SnapshotObjects::open`] is that side; [`SnapshotObjects::adopt`] is the jailed side, which
//! receives the same three handles out of its sealed descriptor table and never learns where
//! they came from.

use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};

use super::{
    artifacts::SnapshotPaths,
    error::{Artifact, SnapshotError},
};

/// Enough for the bounded schema-v2 header and 32 maximum-sized sections.
const MAX_STATE_BYTES: u64 = 34 * 1024 * 1024;

/// The open handles one restore reads its snapshot through.
#[derive(Debug)]
pub struct SnapshotObjects {
    state: File,
    memory: File,
    /// The sterile overlay template, present only for a Generation that declared writable
    /// storage, and read only when the caller asks for artifact verification.
    overlay_template: Option<File>,
}

impl SnapshotObjects {
    /// Opens the published objects of one snapshot directory.
    ///
    /// The overlay template is optional because a Generation with no writable storage publishes
    /// none; its absence is not a failure here and is checked against the state manifest by the
    /// restore that reads it.
    ///
    /// # Errors
    ///
    /// Returns the open failure of the state manifest or the memory image.
    pub fn open(paths: &SnapshotPaths) -> Result<Self, SnapshotError> {
        let state = File::open(paths.state())
            .map_err(|error| SnapshotError::io(Artifact::State, "open", &error))?;
        let memory = File::open(paths.memory())
            .map_err(|error| SnapshotError::io(Artifact::Memory, "open", &error))?;
        Ok(Self {
            state,
            memory,
            overlay_template: File::open(paths.overlay()).ok(),
        })
    }

    /// Adopts handles a broker already opened, which is how a jailed machine receives them.
    #[must_use]
    pub const fn adopt(state: File, memory: File, overlay_template: Option<File>) -> Self {
        Self {
            state,
            memory,
            overlay_template,
        }
    }

    /// The memory image, mapped privately by the restore.
    #[must_use]
    pub const fn memory(&self) -> &File {
        &self.memory
    }

    /// The sterile overlay template, when one was published.
    pub(super) const fn overlay_template(&mut self) -> Option<&mut File> {
        self.overlay_template.as_mut()
    }

    pub(super) const fn memory_handle(&mut self) -> &mut File {
        &mut self.memory
    }

    /// Reads the whole state manifest through the retained handle.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::ArtifactTooLarge`] when the object exceeds the bound every
    /// schema-v2 manifest fits inside, or the seek or read failure.
    pub(super) fn state_bytes(&mut self) -> Result<Vec<u8>, SnapshotError> {
        let size = self
            .state
            .metadata()
            .map_err(|error| SnapshotError::io(Artifact::State, "stat", &error))?
            .len();
        if size > MAX_STATE_BYTES {
            return Err(SnapshotError::ArtifactTooLarge {
                artifact: Artifact::State,
                size,
                maximum: MAX_STATE_BYTES,
            });
        }
        self.state
            .seek(SeekFrom::Start(0))
            .map_err(|error| SnapshotError::io(Artifact::State, "rewind", &error))?;
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        self.state
            .read_to_end(&mut bytes)
            .map_err(|error| SnapshotError::io(Artifact::State, "read", &error))?;
        Ok(bytes)
    }
}
