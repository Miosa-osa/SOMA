//! The seam between the pool and the process that becomes one worker.
//!
//! The jail adapter that forks, namespaces, and execs the real VMM is built on another
//! branch; this crate defines the contract it must satisfy and ships an in-process launcher
//! for deterministic tests.

use std::{fmt, time::Duration};

use crate::{PoolKey, StepAck, TransferFault, TransferFrame, WorkerId};

/// Kernel-side identity of one worker process: the pidfd-verified process number and a
/// boot token such as the process start time or cgroup identity that defeats PID reuse.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WorkerIdentity {
    /// The process number.
    pub process: u64,
    /// The reuse-defeating token.
    pub token: [u8; 16],
}

impl WorkerIdentity {
    /// Encoded length.
    pub const LEN: usize = 24;

    /// Encodes the identity.
    #[must_use]
    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut out = [0; Self::LEN];
        out[..8].copy_from_slice(&self.process.to_be_bytes());
        out[8..].copy_from_slice(&self.token);
        out
    }

    /// Decodes an identity; the all-zero encoding means none.
    #[must_use]
    pub fn decode(bytes: &[u8; Self::LEN]) -> Option<Self> {
        if bytes.iter().all(|byte| *byte == 0) {
            return None;
        }
        let mut process = [0; 8];
        process.copy_from_slice(&bytes[..8]);
        let mut token = [0; 16];
        token.copy_from_slice(&bytes[8..]);
        Some(Self {
            process: u64::from_be_bytes(process),
            token,
        })
    }
}

/// What a probe of a recorded identity found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Liveness {
    /// The exact process is alive.
    Alive,
    /// The process is gone.
    Gone,
    /// The launcher cannot tell; reconciliation treats this as alive and terminates.
    Unknown,
}

/// Why a worker could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstructFault {
    /// The construction deadline passed.
    Timeout {
        /// The budget that was exceeded.
        budget: Duration,
    },
    /// The launcher refused the key or worker.
    Rejected(&'static str),
    /// A construction step failed.
    Failed(&'static str),
}

impl fmt::Display for ConstructFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { budget } => write!(formatter, "construction exceeded {budget:?}"),
            Self::Rejected(reason) => write!(formatter, "construction rejected: {reason}"),
            Self::Failed(reason) => write!(formatter, "construction failed: {reason}"),
        }
    }
}

impl std::error::Error for ConstructFault {}

/// The disposition of one destroyed resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Removal {
    /// Removed now.
    Removed,
    /// Already absent.
    AlreadyAbsent,
    /// Could not be verified.
    Unknown,
}

/// What destroying a worker did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DestroyOutcome {
    /// The process.
    pub process: Removal,
    /// The cgroup and namespaces.
    pub cgroup: Removal,
    /// Whether a final inspection found nothing left.
    pub complete: bool,
}

/// Why an assigned worker did not start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartFault {
    /// The worker refused to run.
    Rejected,
    /// The control channel is closed.
    Closed,
    /// The worker did not acknowledge in time.
    Timeout,
}

/// Builds and supervises worker processes for one host.
pub trait WorkerLauncher: Send + Sync + 'static {
    /// The handle on one live worker.
    type Handle: WorkerHandle;

    /// Constructs one sterile worker within `budget`.
    ///
    /// # Errors
    ///
    /// Returns the typed fault; the launcher must leave nothing behind on failure.
    fn construct(
        &self,
        key: &PoolKey,
        worker: WorkerId,
        budget: Duration,
    ) -> Result<Self::Handle, ConstructFault>;

    /// Probes a recorded identity after a restart.
    fn probe(&self, identity: WorkerIdentity) -> Liveness;

    /// Terminates a recorded identity after a restart when no handle exists.
    fn terminate(&self, identity: WorkerIdentity) -> DestroyOutcome;
}

/// One live worker the pool holds a handle on.
pub trait WorkerHandle: Send + 'static {
    /// The kernel identity.
    fn identity(&self) -> WorkerIdentity;

    /// Delivers one authority frame and waits for its acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns the typed fault; any error destroys the worker.
    fn deliver(&mut self, frame: TransferFrame) -> Result<StepAck, TransferFault>;

    /// Starts the assigned Instance.
    ///
    /// # Errors
    ///
    /// Returns the typed fault; any error destroys the worker.
    fn start(&mut self) -> Result<(), StartFault>;

    /// Destroys the worker and everything it holds.
    fn destroy(self) -> DestroyOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trips_and_zero_means_none() {
        let identity = WorkerIdentity {
            process: 4242,
            token: [9; 16],
        };
        assert_eq!(WorkerIdentity::decode(&identity.encode()), Some(identity));
        assert_eq!(WorkerIdentity::decode(&[0; WorkerIdentity::LEN]), None);
    }
}
