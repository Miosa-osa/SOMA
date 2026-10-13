//! Daemon reply frames.

mod volumes;

use super::{ProtocolError, array};
use crate::{InstanceId, LeaseGeneration, MAX_LISTED, VolumeId, WorkerId};

/// One daemon reply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reply {
    /// Fresh authority was transferred; the launch-page network values follow in order.
    Claimed {
        /// The worker.
        worker: WorkerId,
        /// The lease generation.
        lease_generation: LeaseGeneration,
        /// CID, generation, MAC, address, prefix, gateway, resolver, time sample.
        launch: [u8; 35],
    },
    /// The operation holds this worker but its launch page was never delivered; release and retry.
    Replayed {
        /// The worker.
        worker: WorkerId,
        /// The lease generation.
        lease_generation: LeaseGeneration,
    },
    /// Release completed.
    Released {
        /// Whether teardown and release both reported completion.
        complete: bool,
    },
    /// One worker's phase and generation.
    Inspected {
        /// The phase code.
        phase: u8,
        /// The lease generation.
        lease_generation: LeaseGeneration,
    },
    /// Reconciliation counts: suspects, terminated, released, and retained.
    Reconciled {
        /// Suspects.
        suspects: u32,
        /// Terminated.
        terminated: u32,
        /// Released.
        released: u32,
        /// Retained.
        retained: u32,
    },
    /// One Instance is launched and owned by the Runtime; launch-page values follow in order.
    Launched {
        /// The worker serving the Instance.
        worker: WorkerId,
        /// The lease generation.
        lease_generation: LeaseGeneration,
        /// CID, generation, MAC, address, prefix, gateway, resolver, time sample.
        launch: [u8; 35],
    },
    /// One live Instance, addressed by its own identity.
    Live {
        /// The worker serving it.
        worker: WorkerId,
        /// The lease generation.
        lease_generation: LeaseGeneration,
        /// The phase code of that worker.
        phase: u8,
    },
    /// One bounded page of live Instances.
    Listed {
        /// The Instances of this page, in identity order.
        instances: Vec<InstanceId>,
        /// Whether more live Instances follow the last one listed.
        more: bool,
    },
    /// One Instance is terminal; the reply repeats for every later Destroy of that identity.
    Destroyed {
        /// The worker that served it.
        worker: WorkerId,
        /// Whether teardown and release both reported completion.
        complete: bool,
    },
    /// A volume is bound to its Instance; `state` and `repair` are stable vocabulary codes.
    VolumeAttached {
        /// The volume.
        volume: VolumeId,
        /// The volume state code.
        state: u8,
        /// The repair outcome code.
        repair: u8,
        /// The repair exit code, carried as its own byte.
        exit_code: i8,
    },
    /// A volume is released from its Instance and keeps its file.
    VolumeDetached {
        /// The volume.
        volume: VolumeId,
        /// The volume state code.
        state: u8,
    },
    /// A volume is destroyed; its file and its data key are gone.
    VolumeDestroyed {
        /// The volume.
        volume: VolumeId,
        /// The terminal state code.
        state: u8,
    },
    /// One volume's sidecar projection.
    VolumeInspected {
        /// The volume.
        volume: VolumeId,
        /// The volume state code.
        state: u8,
        /// The disk class in GiB.
        size_class_gib: u32,
    },
    /// The volume bound to an Instance, with the file to bind and this start's repair.
    VolumeOpened {
        /// The volume.
        volume: VolumeId,
        /// The disk class in GiB.
        size_class_gib: u32,
        /// The repair outcome code.
        repair: u8,
        /// The repair exit code, carried as its own byte.
        exit_code: i8,
        /// The host's path to the volume file.
        path: String,
    },
    /// The volume one Instance owns, and its state, read without touching it.
    VolumeOwned {
        /// The volume.
        volume: VolumeId,
        /// The volume state code.
        state: u8,
    },
    /// A typed failure code from [`crate::failure_code`].
    Failed(u16),
}

/// Decodes one listing page, which is the only reply of variable length.
fn listed(bytes: &[u8]) -> Result<Reply, ProtocolError> {
    if bytes.len() < 3 || bytes[1] > 1 {
        return Err(ProtocolError("reply"));
    }
    let count = usize::from(bytes[2]);
    if count > MAX_LISTED || bytes.len() != 3 + count * 16 {
        return Err(ProtocolError("reply"));
    }
    let instances = bytes[3..]
        .as_chunks::<16>()
        .0
        .iter()
        .map(|chunk| InstanceId::new(*chunk).map_err(|_| ProtocolError("instance")))
        .collect::<Result<Vec<InstanceId>, ProtocolError>>()?;
    Ok(Reply::Listed {
        instances,
        more: bytes[1] == 1,
    })
}
mod codec;
