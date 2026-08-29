//! The exactly-once transfer of fresh authority into one claimed worker.
//!
//! Eight ordered frames carry identity, deadline, entropy, launch page, disk, network,
//! control, and the commit; each must be acknowledged before the next is sent.
//! Any fault, timeout, or partial acknowledgement destroys the worker, which never returns
//! to the pool.

mod run;

use std::{fmt, time::Duration};

use soma_guest::LaunchNetwork;

pub use run::TransferFailure;

use crate::{
    Descriptor, DestroyOutcome, InstanceId, LaunchMaterialHandle, LeaseGeneration, LedgerError,
    OperationId, ResourceFault, ResourceRelease, StateRace, WorkerId,
};

/// One ordered transfer step.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[repr(u8)]
pub enum TransferStep {
    /// Instance, operation, worker, and lease generation.
    Identity = 1,
    /// The Instance deadline.
    Deadline = 2,
    /// Fresh entropy.
    Entropy = 3,
    /// Launch material and network identity.
    LaunchPage = 4,
    /// The private disk head.
    Disk = 5,
    /// The TAP descriptor.
    Network = 6,
    /// The vsock CID and control channel.
    Control = 7,
    /// The worker owns everything; the allocator leaves the data path.
    Commit = 8,
}

impl TransferStep {
    /// Every step in transfer order.
    pub const ALL: [Self; 8] = [
        Self::Identity,
        Self::Deadline,
        Self::Entropy,
        Self::LaunchPage,
        Self::Disk,
        Self::Network,
        Self::Control,
        Self::Commit,
    ];

    /// Returns the stable encoding.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Decodes one step.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Identity),
            2 => Some(Self::Deadline),
            3 => Some(Self::Entropy),
            4 => Some(Self::LaunchPage),
            5 => Some(Self::Disk),
            6 => Some(Self::Network),
            7 => Some(Self::Control),
            8 => Some(Self::Commit),
            _ => None,
        }
    }
}

/// One authority frame.
#[derive(Debug)]
pub enum TransferFrame {
    /// Identity.
    Identity {
        /// The worker.
        worker: WorkerId,
        /// The lease generation.
        lease_generation: LeaseGeneration,
        /// The Instance.
        instance: InstanceId,
        /// The operation.
        operation: OperationId,
    },
    /// Deadline.
    Deadline {
        /// Nanoseconds the Instance may live.
        deadline_nanos: u64,
    },
    /// Entropy.
    Entropy {
        /// Fresh bytes from the host.
        seed: [u8; 32],
    },
    /// Launch page inputs.
    LaunchPage {
        /// The sealed material.
        material: LaunchMaterialHandle,
        /// The exact network identity.
        network: LaunchNetwork,
    },
    /// The private disk head.
    Disk(Descriptor),
    /// The TAP.
    Network(Descriptor),
    /// Control.
    Control {
        /// The vsock CID.
        vsock_cid: u32,
        /// The worker end of the control channel.
        channel: Descriptor,
    },
    /// Commit.
    Commit,
}

impl TransferFrame {
    /// Returns the step the frame belongs to.
    #[must_use]
    pub const fn step(&self) -> TransferStep {
        match self {
            Self::Identity { .. } => TransferStep::Identity,
            Self::Deadline { .. } => TransferStep::Deadline,
            Self::Entropy { .. } => TransferStep::Entropy,
            Self::LaunchPage { .. } => TransferStep::LaunchPage,
            Self::Disk(_) => TransferStep::Disk,
            Self::Network(_) => TransferStep::Network,
            Self::Control { .. } => TransferStep::Control,
            Self::Commit => TransferStep::Commit,
        }
    }
}

/// A worker's acknowledgement of one frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepAck {
    /// The frame was applied.
    Accepted,
}

/// Why a transfer step failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferFault {
    /// The worker refused the frame.
    Rejected,
    /// The worker did not acknowledge in time.
    Timeout,
    /// The worker acknowledged a different step.
    PartialAck,
    /// The channel closed.
    Closed,
    /// The claim deadline passed before the transfer completed.
    ClaimDeadline,
    /// The host produced no fresh entropy.
    Entropy,
    /// A resource could not be assigned.
    Resource(ResourceFault),
    /// The ledger could not record the step.
    Ledger(LedgerError),
    /// The worker's state moved under the transfer.
    State(StateRace),
}

impl fmt::Display for TransferFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected => formatter.write_str("worker rejected the frame"),
            Self::Timeout => formatter.write_str("worker did not acknowledge in time"),
            Self::PartialAck => formatter.write_str("worker acknowledged a different step"),
            Self::Closed => formatter.write_str("worker channel closed"),
            Self::ClaimDeadline => formatter.write_str("claim deadline passed"),
            Self::Entropy => formatter.write_str("no fresh entropy"),
            Self::Resource(fault) => write!(formatter, "resource fault: {fault}"),
            Self::Ledger(error) => write!(formatter, "ledger fault: {error}"),
            Self::State(race) => write!(formatter, "state race: {race:?}"),
        }
    }
}

impl std::error::Error for TransferFault {}

/// What a completed transfer proved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferEvidence {
    /// The worker.
    pub worker: WorkerId,
    /// The lease generation.
    pub lease_generation: LeaseGeneration,
    /// The Instance.
    pub instance: InstanceId,
    /// The operation.
    pub operation: OperationId,
    /// The exact network identity delivered.
    pub launch: LaunchNetwork,
    /// Steps acknowledged.
    pub steps: u8,
    /// Wall-clock cost of the transfer.
    pub elapsed: Duration,
}

/// What destroying a worker after a failed transfer did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Disposition {
    /// The worker teardown.
    pub destroyed: DestroyOutcome,
    /// The resource release.
    pub released: ResourceRelease,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_ordered_and_round_trip() {
        for (index, step) in TransferStep::ALL.iter().enumerate() {
            assert_eq!(usize::from(step.code()), index + 1);
            assert_eq!(TransferStep::from_code(step.code()), Some(*step));
        }
        assert_eq!(TransferStep::from_code(0), None);
        assert!(TransferStep::Identity < TransferStep::Commit);
        assert_eq!(TransferFrame::Commit.step(), TransferStep::Commit);
    }
}
