//! The volume operations the Host Runtime performs, behind one seam.
//!
//! The daemon drives these through [`VolumeStore`], which is the same shape as the
//! `WorkerLauncher` and `ResourceBroker` seams: the Host owns the behavior, the seam makes every
//! policy testable without a kernel, and a second adapter can carry the same semantics without
//! speaking this socket.

use std::path::PathBuf;

use crate::{GenerationId, InstanceId, OperationId, RequestFingerprint, VolumeId};

use super::error::VolumeError;
use super::repair::RepairReceipt;
use super::sidecar::{VolumeSidecar, VolumeState};

/// The device a Step A volume is bound to, in the order the machine contract names its disks.
pub const VOLUME_DEVICE: &str = "vdc";

/// One attach request, which creates the volume on first attach and is idempotent thereafter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachRequest {
    /// The operation, which is the idempotency key.
    pub operation: OperationId,
    /// The owning Instance.
    pub instance: InstanceId,
    /// The volume.
    pub volume: VolumeId,
    /// The fingerprint of this request under the operation.
    pub fingerprint: RequestFingerprint,
    /// The requested disk class in GiB.
    pub size_class_gib: u32,
    /// The Generation the owning Instance was launched from.
    pub generation: GenerationId,
    /// The owning tenant label.
    pub tenant: String,
}

/// One detach request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DetachRequest {
    /// The operation, which is the idempotency key.
    pub operation: OperationId,
    /// The owning Instance.
    pub instance: InstanceId,
    /// The volume.
    pub volume: VolumeId,
    /// The fingerprint of this request under the operation.
    pub fingerprint: RequestFingerprint,
}

/// A broker-authorized copy of a saved volume into a distinct recovery Instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointVolumeRequest {
    /// The Instance that owns the captured workspace.
    pub source: InstanceId,
    /// The new Instance that will own the independent recovery workspace.
    pub destination: InstanceId,
    /// The checkpoint containing the source disk.
    pub checkpoint: OperationId,
    /// The certified Generation used by both Instances.
    pub generation: GenerationId,
    /// The block digest certified by the checkpoint manifest.
    pub digest: RequestFingerprint,
}

/// A broker-authorized replacement of the already-owned workspace of a stopped Instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceRestoreRequest {
    /// The checkpoint and its verified block digest.
    pub checkpoint: CheckpointVolumeRequest,
    /// The idempotency key of this restore, distinct from the checkpoint capture.
    pub operation: OperationId,
    /// The original workspace named by the restore authorization.
    pub volume: VolumeId,
}

/// A verified writable copy of the checkpoint disk, before assigning it to the guest.
pub struct CheckpointVolumeBinding {
    /// The recovery volume's identity.
    pub volume: VolumeId,
    /// The independent backing file on the encrypted data mount.
    pub path: PathBuf,
    /// The class in GiB.
    pub size_class_gib: u32,
}

/// The outcome of one attach.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachOutcome {
    /// The volume.
    pub volume: VolumeId,
    /// The device it is bound to.
    pub device: &'static str,
    /// The state after the attach.
    pub state: VolumeState,
    /// The repair receipt, when a repair ran.
    pub repair: Option<RepairReceipt>,
}

/// What a start needs to bind one Instance's volume to the machine that will serve it.
///
/// The path is the host's own, derived from the store root and the volume the Instance owns, so a
/// caller cannot name a file of its choosing. The repair is this start's receipt: the volume is
/// checked on the host, offline, under the volume lock, before any machine holds it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeBinding {
    /// The volume bound to the Instance.
    pub volume: VolumeId,
    /// The file the machine binds over its placeholder.
    pub path: PathBuf,
    /// The disk class the volume was created with.
    pub size_class_gib: u32,
    /// The repair this start ran on the detached file.
    pub repair: RepairReceipt,
}

/// The outcome of one detach.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DetachOutcome {
    /// The volume.
    pub volume: VolumeId,
    /// The state after the detach.
    pub state: VolumeState,
}

/// The volume lifecycle seam the daemon drives.
pub trait VolumeStore {
    /// Validates and repairs the workspace this running Instance already owns.
    ///
    /// # Errors
    ///
    /// Refuses a missing workspace, wrong owner or unsuccessful offline repair.
    fn stage_live_attach(&self, _request: &AttachRequest) -> Result<VolumeBinding, VolumeError> {
        Err(VolumeError::Missing(
            "live workspace attachment is unavailable",
        ))
    }

    /// Records a successful mount after the worker holds the locked backing.
    ///
    /// # Errors
    ///
    /// Refuses an unlocked file or a workspace whose owner or intent differs.
    fn commit_live_attach(&self, _request: &AttachRequest) -> Result<AttachOutcome, VolumeError> {
        Err(VolumeError::Missing(
            "live workspace attachment is unavailable",
        ))
    }

    /// Checks the requested ownership and operation identity before guest unmount.
    ///
    /// # Errors
    ///
    /// Refuses an unknown workspace, a conflicting owner, or a changed operation.
    fn validate_live_detach(&self, _request: &DetachRequest) -> Result<VolumeState, VolumeError> {
        Err(VolumeError::Missing("live workspace detach is unavailable"))
    }

    /// Copies and binds a saved disk without modifying the disk it was captured from.
    ///
    /// # Errors
    ///
    /// Refuses an unsafe or unavailable encrypted store, changed intent, or a bad saved disk.
    fn clone_checkpoint(
        &self,
        _request: &CheckpointVolumeRequest,
    ) -> Result<CheckpointVolumeBinding, VolumeError> {
        Err(VolumeError::Missing(
            "checkpoint volume cloning is unavailable",
        ))
    }

    /// Creates the volume on first attach, repairs it offline, and binds it to the Instance.
    ///
    /// # Errors
    ///
    /// Returns [`VolumeError`] for a missing volume, one attached to another Instance, a repair
    /// that could not make the file consistent, a class below the volume's own, or a changed
    /// intent under a replayed operation.
    fn attach(&self, request: &AttachRequest) -> Result<AttachOutcome, VolumeError>;

    /// Flushes and detaches the volume from its Instance, leaving the file in place.
    ///
    /// # Errors
    ///
    /// Returns [`VolumeError`] for a missing volume or a changed intent under a replay.
    fn detach(&self, request: &DetachRequest) -> Result<DetachOutcome, VolumeError>;

    /// Opens the volume attached to one Instance, repairing it for the start that asked.
    ///
    /// It is the start's half of the volume lifecycle: the file is checked offline before any
    /// machine holds it, and a volume that is not attached to this Instance is refused rather
    /// than handed to a machine that would write it.
    ///
    /// # Errors
    ///
    /// Returns [`VolumeError::Missing`] when no attached volume names this Instance, and the
    /// repair refusals [`VolumeError`] already carries.
    fn binding(&self, instance: InstanceId) -> Result<VolumeBinding, VolumeError>;

    /// Reads the volume one Instance owns, without touching its state.
    ///
    /// A stop needs to know which volume it is detaching and must not attach one to find out.
    ///
    /// # Errors
    ///
    /// Returns [`VolumeError::Missing`] when no volume belongs to this Instance.
    fn owned(&self, instance: InstanceId) -> Result<VolumeSidecar, VolumeError>;

    /// Destroys the volume file and its data key.
    ///
    /// # Errors
    ///
    /// Returns [`VolumeError`] for a missing volume or a failed unlink.
    fn destroy(&self, volume: VolumeId) -> Result<(), VolumeError>;

    /// Returns the sidecar projection of one volume.
    ///
    /// # Errors
    ///
    /// Returns [`VolumeError::Missing`] when no sidecar names this volume.
    fn inspect(&self, volume: VolumeId) -> Result<VolumeSidecar, VolumeError>;
}
