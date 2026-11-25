//! Create a new volume: seal and record its data key, write its sidecar, then make its file.

use std::path::Path;

use super::super::{DirectoryVolumes, VolumeError, VolumeRecordKind, VolumeSidecar, VolumeState};
use crate::VolumeId;
use crate::volume::repair::CreateError;

/// Bytes in one GiB, the unit a volume's class is stated in.
const GIB: u64 = 1 << 30;

/// Free bytes the volume root keeps for the volumes it already holds.
pub(in crate::volume::store) const VOLUME_ROOT_RESERVE: u64 = GIB;

impl DirectoryVolumes {
    /// Creates a volume the store has never held, under its begin and end records.
    pub(super) fn create_volume(
        &self,
        volume: VolumeId,
        sidecar: &mut VolumeSidecar,
        file: &Path,
    ) -> Result<(), VolumeError> {
        self.require_room()?;
        let fresh = !self.directory(volume).exists();
        let created = self.create_steps(volume, sidecar, file);
        if created.is_err() && fresh {
            self.undo_create(volume);
        }
        created
    }

    /// Takes back what a create that failed had written: its sidecar and directory, its key and
    /// its records. Left there, the sidecar named a volume with no filesystem the next time the
    /// store opened. Only a directory this create made is removed, so a path that already held
    /// data (`Occupied`) is never touched.
    fn undo_create(&self, volume: VolumeId) {
        let _ = std::fs::remove_dir_all(self.directory(volume));
        let _ = self.keys.destroy(volume);
        self.forget(volume);
    }

    /// Refuses a new volume when the volume root has less free space than its reserve.
    ///
    /// Volumes are sparse, so a new one takes little at first, but its filesystem is written at
    /// once and every volume the root already holds grows into the same free space. A root
    /// below its reserve answers a capacity refusal the caller can place elsewhere, instead of
    /// failing part way through the create.
    fn require_room(&self) -> Result<(), VolumeError> {
        match free_bytes(&self.root) {
            Some(free) if free < self.reserve => Err(VolumeError::Capacity(
                "the volume store has no room for another volume",
            )),
            _ => Ok(()),
        }
    }

    fn create_steps(
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

/// The bytes an unprivileged writer may still use on the filesystem holding `path`.
#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "statvfs is the one way to read a filesystem's free space"
)]
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt as _;

    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `name` is a NUL-terminated path and `stats` is a buffer `statvfs` fills on success.
    if unsafe { libc::statvfs(name.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `statvfs` returned 0, so it initialized `stats`.
    let stats = unsafe { stats.assume_init() };
    stats.f_bavail.checked_mul(stats.f_frsize)
}

/// No host this store serves volumes on runs another system, so the reserve is not measured.
#[cfg(not(target_os = "linux"))]
const fn free_bytes(_path: &Path) -> Option<u64> {
    None
}
