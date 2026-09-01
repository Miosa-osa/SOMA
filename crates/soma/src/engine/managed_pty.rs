//! One bounded terminal operation against an exact managed Instance.
//!
//! This path is the filesystem path's twin and is deliberately as light. A terminal operation
//! runs no program the facade has a terminal status for, and produces no evidence a later caller
//! could replay, so it mints no receipt, writes no tombstone, and moves the machine through no
//! phase. Admission is therefore a read: the machine must exist, must be Active, and must belong
//! to this Backend.
//!
//! The session itself is not state this engine holds. It belongs to the machine, which is held by
//! a process that outlives every call here, so a caller that opens a terminal in one process and
//! writes to it from another is addressing the same session without this engine remembering
//! anything between them.

use crate::{
    Backend, InstanceId, ManagedStateError, OperationId, PtyAnswer, PtyObservation, PtyOperation,
    StateStore,
};

use super::{Engine, ManagedFailure};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Performs one bounded terminal operation inside an exact managed Instance.
    ///
    /// # Errors
    ///
    /// Returns typed state or durable-store failures, and a backend failure when the operation
    /// could not be performed at all. A cause the guest reported is not an error here: it comes
    /// back as [`PtyAnswer::Refused`], because the guest was reached and declined.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the use-case boundary takes ownership of its immutable request"
    )]
    pub fn pty_machine(
        &mut self,
        request: PtyMachineRequest,
    ) -> Result<MachinePty, ManagedFailure> {
        self.admit_operation(&request.instance_id)?;
        let observation = self
            .backend
            .pty(crate::PtyRequest::new(
                &request.operation_id,
                &request.instance_id,
                &request.operation,
            ))
            .map_err(|failure| ManagedFailure::Backend(failure.kind()))?;
        let answer = accept(observation, &request.operation_id, &request.instance_id)
            .ok_or(ManagedFailure::State(ManagedStateError::OperationConflict))?;
        Ok(MachinePty {
            instance_id: request.instance_id,
            operation: request.operation,
            answer,
        })
    }
}

/// Takes the answer only when it names the operation and Instance that were asked about.
fn accept(
    observation: PtyObservation,
    operation_id: &OperationId,
    instance_id: &InstanceId,
) -> Option<PtyAnswer> {
    (observation.operation_id() == operation_id && observation.instance_id() == instance_id)
        .then(|| observation.into_answer())
}

/// One completed terminal operation and what it answered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachinePty {
    pub(super) instance_id: InstanceId,
    pub(super) operation: PtyOperation,
    pub(super) answer: PtyAnswer,
}

impl MachinePty {
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn operation(&self) -> &PtyOperation {
        &self.operation
    }

    #[must_use]
    pub const fn answer(&self) -> &PtyAnswer {
        &self.answer
    }
}

/// One terminal operation addressed to one managed Instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtyMachineRequest {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    pub(super) operation: PtyOperation,
}

impl PtyMachineRequest {
    /// The operation this request carries.
    #[must_use]
    pub const fn operation(&self) -> &PtyOperation {
        &self.operation
    }

    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        operation: PtyOperation,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            operation,
        }
    }
}
