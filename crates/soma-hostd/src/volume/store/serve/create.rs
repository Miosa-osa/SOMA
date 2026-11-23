//! Create a new volume: seal and record its data key, write its sidecar, then make its file.

use std::path::Path;

use super::super::{DirectoryVolumes, VolumeError, VolumeRecordKind, VolumeSidecar, VolumeState};
use crate::VolumeId;
use crate::volume::repair::CreateError;

/// Bytes in one GiB, the unit a volume's class is stated in.
const GIB: u64 = 1 << 30;

impl DirectoryVolumes {
    /// Creates a volume the store has never held, under its begin and end records.
    pub(super) fn create_volume(
        &self,
        volume: VolumeId,
        sidecar: &mut VolumeSidecar,
        file: &Path,
    ) -> Result<(), VolumeError> {
        self.transition(VolumeRecordKind::CreateBegin, volume)?;
        // The data key is sealed and recorded before the sidecar names it, so a volume never
        // exists without the key whose destruction is what proves its destruction.
        let sealed = self
            .keys
            .seal(volume)
            .map_err(|_| VolumeError::Encryption("the volume's data key could not be sealed"))?;
        self.keys
            .record(volume, &sealed)
            .map_err(|_| VolumeError::Encryption("the volume's data key could not be recorded"))?;
        sidecar.dek_id = sealed.dek_id;
        self.write_sidecar(&VolumeSidecar {
            state: VolumeState::Detached,
            ..sidecar.clone()
        })?;
        // The file and its filesystem are created here, so the repair below runs on a real
        // ext4 filesystem rather than on a path that does not exist yet.
        self.make(volume, file, sidecar.size_class_gib)?;
        self.transition(VolumeRecordKind::CreateEnd, volume)?;
        Ok(())
    }

    /// Creates the volume's file and filesystem at the class it was admitted with.
    ///
    /// A refusal here is a volume that does not exist: the file is left as it was found, so a
    /// later attach retries rather than inheriting a half-created volume.
    fn make(&self, volume: VolumeId, file: &Path, class_gib: u32) -> Result<(), VolumeError> {
        let bytes = u64::from(class_gib) * GIB;
        self.repairs
            .create(volume, file, bytes)
            .map_err(|error| match error {
                CreateError::Occupied => VolumeError::Busy("the volume path already holds data"),
                CreateError::Size(_) => VolumeError::Geometry("the volume class has no size"),
                CreateError::Tool(_) => {
                    VolumeError::FsckFailed("the volume filesystem could not be created")
                }
            })
    }
}
