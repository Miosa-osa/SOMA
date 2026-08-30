//! Installation of an already captured snapshot into the immutable Generation store.

use std::{io::Read, path::Path};

use super::{
    artifacts::{ArtifactDescriptor, ArtifactRole},
    contracts::{SNAPSHOT_CAPTURE_POINT_VERSION, SNAPSHOT_FORMAT_VERSION},
    error::{CompileError, CompileErrorKind, CompilePhase},
    manifest::SnapshotBinding,
};
use crate::{ImportPhase, store::Store};

const MAX_STATE_BYTES: u64 = 34 * 1024 * 1024;

/// One captured object paired with the descriptor computed while it was written.
pub struct SnapshotSource<'a, R: Read> {
    reader: &'a mut R,
    descriptor: ArtifactDescriptor,
}

impl<'a, R: Read> SnapshotSource<'a, R> {
    #[must_use]
    pub const fn new(reader: &'a mut R, descriptor: ArtifactDescriptor) -> Self {
        Self { reader, descriptor }
    }
}

/// Installs the three objects from one completed capture and returns their typed binding.
///
/// Each descriptor must come from the capture that produced its reader.
/// The store checks exact size and digest before publishing each immutable object.
/// Certification independently reopens and cross-checks all three objects afterward.
///
/// # Errors
///
/// Returns a typed failure for a wrong role, oversized state, missing store, or any content
/// mismatch.
pub fn install_snapshot(
    store: &Path,
    memory: SnapshotSource<'_, impl Read>,
    overlay: SnapshotSource<'_, impl Read>,
    state: SnapshotSource<'_, impl Read>,
) -> Result<SnapshotBinding, CompileError> {
    let SnapshotSource {
        reader: memory_reader,
        descriptor: memory_descriptor,
    } = memory;
    let SnapshotSource {
        reader: overlay_reader,
        descriptor: overlay_descriptor,
    } = overlay;
    let SnapshotSource {
        reader: state_reader,
        descriptor: state_descriptor,
    } = state;
    if memory_descriptor.role != ArtifactRole::MemorySnapshot
        || overlay_descriptor.role != ArtifactRole::OverlaySnapshot
        || state_descriptor.role != ArtifactRole::StateManifest
    {
        return Err(CompileError::new(
            CompilePhase::BootAndCapture,
            CompileErrorKind::InvalidInput,
        ));
    }
    if state_descriptor.size > MAX_STATE_BYTES {
        return Err(CompileError::new(
            CompilePhase::BootAndCapture,
            CompileErrorKind::LimitExceeded,
        ));
    }
    let store = Store::open(store)
        .map_err(|error| CompileError::from_import(CompilePhase::BootAndCapture, error))?;
    install(&store, memory_reader, memory_descriptor)?;
    install(&store, overlay_reader, overlay_descriptor)?;
    install(&store, state_reader, state_descriptor)?;
    Ok(SnapshotBinding::Captured {
        format_version: SNAPSHOT_FORMAT_VERSION,
        memory: memory_descriptor,
        overlay: overlay_descriptor,
        state: state_descriptor,
        capture_point_version: SNAPSHOT_CAPTURE_POINT_VERSION,
    })
}

fn install(
    store: &Store,
    source: &mut impl Read,
    descriptor: ArtifactDescriptor,
) -> Result<(), CompileError> {
    store
        .put_descriptor(
            source,
            &descriptor.to_store_descriptor(),
            descriptor.size,
            ImportPhase::Publish,
        )
        .map_err(|error| CompileError::from_import(CompilePhase::BootAndCapture, error))
}
