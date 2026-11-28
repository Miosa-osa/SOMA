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

/// Removes a newly created scratch disk if the launch fails before owning the machine.
pub(super) struct Guard<'a> {
    instance: &'a InstanceId,
    armed: bool,
}

impl Guard<'_> {
    /// Hands ownership of the scratch disk to the live machine; `cleanup_resident` will remove it.
    pub(super) const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if self.armed {
            remove(self.instance);
        }
    }
}

/// The volume a launch serves: the one it names, a scratch disk when the Generation declares a
/// volume and the launch names none, and nothing otherwise.
///
/// A restore and a resume bring their own disk state, so they are never given a scratch disk.
pub(super) fn resolve<'a>(
    extras: &LaunchExtras<'_>,
    instance: &'a InstanceId,
    prepared: &PreparedGeneration,
) -> Result<(Option<VolumeAttachment>, Guard<'a>), BackendFailureKind> {
    let persistence = &prepared.manifest.persistence;
    let restoring = extras.checkpoint.is_some() || extras.resume.is_some();
    if extras.volume.is_some() || restoring || !persistence.enabled {
        return Ok((
            extras.volume.clone(),
            Guard {
                instance,
                armed: false,
            },
        ));
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
    let guard = Guard {
        instance,
        armed: true,
    };
    let id = super::identity::instance_bytes(instance)?;
    Ok((Some(VolumeAttachment::new(id, path)), guard))
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

#[cfg(test)]
mod tests {
    use soma::InstanceId;

    use super::{Guard, path};

    #[test]
    fn guard_removes_scratch_disk_on_failed_launch_and_keeps_it_when_disarmed() {
        let id = InstanceId::new("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6").expect("id");
        let file = path(&id);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("dir");
        }
        std::fs::write(&file, b"scratch").expect("create");
        {
            let _guard = Guard {
                instance: &id,
                armed: true,
            };
        }
        assert!(
            !file.exists(),
            "dropping an armed guard must remove the scratch disk"
        );

        std::fs::write(&file, b"scratch").expect("recreate");
        {
            let mut guard = Guard {
                instance: &id,
                armed: true,
            };
            guard.disarm();
        }
        assert!(
            file.exists(),
            "a disarmed guard hands ownership to the live machine"
        );
        let _ = std::fs::remove_file(&file);
    }
}
