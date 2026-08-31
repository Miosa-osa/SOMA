use crate::{
    ExecutionReceipt, OperationId, RequestFingerprint, StateStoreFailureKind, TerminalStatus,
};

use super::{RunFailure, machine_state::ExecutionTombstone};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedStateError {
    MachineAlreadyExists,
    MachineNotFound,
    MachineStopped,
    OperationConflict,
    RecoveryRequired,
    ReplayCapacityReached,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayEvidence {
    operation_id: OperationId,
    request_fingerprint: RequestFingerprint,
    terminal_status: TerminalStatus,
    receipt_digest: RequestFingerprint,
    receipt: Option<Box<ExecutionReceipt>>,
}

impl ReplayEvidence {
    pub(super) fn from_tombstone(tombstone: &ExecutionTombstone) -> Self {
        Self {
            operation_id: tombstone.operation_id.clone(),
            request_fingerprint: tombstone.request_fingerprint.clone(),
            terminal_status: tombstone.terminal_status,
            receipt_digest: tombstone.receipt_digest.clone(),
            receipt: None,
        }
    }

    pub(super) fn from_receipt(receipt: ExecutionReceipt) -> Self {
        let encoded =
            serde_json::to_vec(&receipt).expect("a validated execution receipt always serializes");
        Self {
            operation_id: receipt.operation_id().clone(),
            request_fingerprint: receipt.request_fingerprint().clone(),
            terminal_status: *receipt.terminal_status(),
            receipt_digest: crate::fingerprint::digest_bytes(&encoded),
            receipt: Some(Box::new(receipt)),
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn request_fingerprint(&self) -> &RequestFingerprint {
        &self.request_fingerprint
    }

    #[must_use]
    pub const fn terminal_status(&self) -> &TerminalStatus {
        &self.terminal_status
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &RequestFingerprint {
        &self.receipt_digest
    }

    #[must_use]
    pub fn receipt(&self) -> Option<&ExecutionReceipt> {
        self.receipt.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagedFailure {
    State(ManagedStateError),
    StateStore(StateStoreFailureKind),
    Operation(Box<RunFailure>),
    ReplayUnavailable(ReplayEvidence),
    /// A backend refused an operation that mints no receipt, so there is no evidence to carry.
    ///
    /// Every other operation on this facade produces an [`crate::ExecutionReceipt`] whether it
    /// succeeded or failed, which is why they report through [`RunFailure`]. A filesystem
    /// operation produces none, so a receipt-carrying failure would have to invent one, and an
    /// invented receipt is worse than a bare kind.
    Backend(crate::BackendFailureKind),
}

impl ManagedFailure {
    pub(super) fn operation(failure: RunFailure) -> Self {
        Self::Operation(Box::new(failure))
    }
}
