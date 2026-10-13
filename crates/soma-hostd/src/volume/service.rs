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
