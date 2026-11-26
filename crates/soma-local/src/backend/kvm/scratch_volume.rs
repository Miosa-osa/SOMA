//! The disk a volume-declaring Generation gets when its launch names no volume (SOMA#262).
//!
//! Every Generation declares the durable volume, and the platform always attaches one. A caller
//! that launches without one still gets a machine: the device stands on a sparse all-zero file
//! under the head root, which the guest reads as "no volume for this Instance" and serves the
//! persistent paths from its private overlay instead. A volume that is present but damaged is
//! never mistaken for this, because only a superblock that is all zeros reads as no volume.
//!
//! The file belongs to the machine: it is made at launch, made again if a stopped host left it,
//! and removed with the machine.

use std::path::PathBuf;

use soma::{BackendFailureKind, InstanceId, VolumeAttachment};

use super::boot::head::head_root;
use super::lifecycle::LaunchExtras;
use super::prepared::PreparedGeneration;

/// The volume a launch serves: the one it names, a scratch disk when the Generation declares a
/// volume and the launch names none, and nothing otherwise.
///
/// A restore and a resume bring their own disk state, so they are never given a scratch disk.
pub(super) fn resolve(
    extras: &LaunchExtras<'_>,
    instance: &InstanceId,
    prepared: &PreparedGeneration,
) -> Result<Option<VolumeAttachment>, BackendFailureKind> {
    let persistence = &prepared.manifest.persistence;
    let restoring = extras.checkpoint.is_some() || extras.resume.is_some();
    if extras.volume.is_some() || restoring || !persistence.enabled {
        return Ok(extras.volume.clone());
    }
    let path = path(instance);
    let unavailable = |_| BackendFailureKind::Unavailable;
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(unavailable)?;
    }
    // Emptied first, so a file a stopped host left is all zeros again before it is sized.
    let file = std::fs::File::create(&path).map_err(unavailable)?;
    file.set_len(u64::from(persistence.size_class_gib) << 30)
        .map_err(unavailable)?;
    let id = super::identity::instance_bytes(instance)?;
    Ok(Some(VolumeAttachment::new(id, path)))
}

/// Removes `instance`'s scratch disk, if it has one.
pub(super) fn remove(instance: &InstanceId) {
    let _ = std::fs::remove_file(path(instance));
}

fn path(instance: &InstanceId) -> PathBuf {
    head_root()
        .join("scratch-volumes")
        .join(format!("{}.img", instance.as_str()))
}
