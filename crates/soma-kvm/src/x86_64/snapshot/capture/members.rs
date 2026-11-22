//! Writing a capture's two large objects: guest memory and the private head.

use std::fs::File;

use super::super::artifacts::{SnapshotPaths, Staging};
use super::super::error::{Artifact, SnapshotError};
use crate::x86_64::memory::SharedRam;

/// Bytes copied out of guest RAM per pass.
const CHUNK: usize = 1 << 20;

/// Writes guest memory and, when there is one, the private head into staging objects.
///
/// A Suspension's members are authenticated by the store, so they are not hashed, and its head
/// is cloned rather than copied.
pub(super) fn write(
    paths: &SnapshotPaths,
    (memory, ram_bytes): (&SharedRam, u64),
    overlay: Option<&mut File>,
    suspension: bool,
) -> Result<(Staging, Option<Staging>), SnapshotError> {
    let staging = if suspension {
        Staging::create_unhashed
    } else {
        Staging::create
    };
    let mut object = staging(paths, Artifact::Memory, paths.memory())?;
    let mut buffer = vec![0_u8; CHUNK];
    let mut offset = 0_u64;
    while offset < ram_bytes {
        let remaining = usize::try_from(ram_bytes - offset).unwrap_or(CHUNK);
        let span = remaining.min(CHUNK);
        // The walk reads the memory object linearly across both ranges of a split machine, so
        // it names object offsets rather than guest addresses.
        if !memory.read_image(offset, &mut buffer[..span]) {
            return Err(SnapshotError::NotQuiescent(
                "guest RAM shrank during capture",
            ));
        }
        object.write(&buffer[..span])?;
        offset += u64::try_from(span).unwrap_or(0);
    }
    let overlay = overlay
        .map(|head| {
            let mut object = staging(paths, Artifact::Overlay, paths.overlay())?;
            if suspension {
                object.clone_file(head)?;
            } else {
                object.write_file(head)?;
            }
            Ok::<_, SnapshotError>(object)
        })
        .transpose()?;
    Ok((object, overlay))
}
