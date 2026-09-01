//! One bounded filesystem operation against an exact managed Instance.
//!
//! This path is deliberately lighter than execute. A filesystem operation runs no program, has no
//! terminal status, and produces no evidence a later caller could replay, so it mints no receipt,
//! writes no tombstone, and moves the machine through no phase. Admission is therefore a read:
//! the machine must exist, must be Active, and must belong to this Backend. Recording a phase
//! transition for an operation that leaves nothing to recover would add a durable write to every
//! directory listing and a recovery case that could never fire.

use crate::{
    Backend, FileAnswer, FileObservation, FileOperation, InstanceId, ManagedStateError,
    OperationId, StateStore,
};

use super::{
    Engine, ManagedFailure,
    machine_state::{ActiveMachine, DurablePhase, VersionedMachine},
};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Performs one bounded filesystem operation inside an exact managed Instance.
    ///
    /// # Errors
    ///
    /// Returns typed state or durable-store failures, and a backend failure when the operation
    /// could not be performed at all. A cause the guest reported is not an error here: it comes
    /// back as [`FileAnswer::Refused`], because the guest was reached and declined.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the use-case boundary takes ownership of its immutable request"
    )]
    pub fn file_machine(
        &mut self,
        request: FileMachineRequest,
    ) -> Result<MachineFile, ManagedFailure> {
        self.admit_file(&request.instance_id)?;
        let observation = self
            .backend
            .file(crate::FileRequest::new(
                &request.operation_id,
                &request.instance_id,
                &request.operation,
            ))
            .map_err(|failure| ManagedFailure::Backend(failure.kind()))?;
        let answer = accept(observation, &request.operation_id, &request.instance_id)
            .ok_or(ManagedFailure::State(ManagedStateError::OperationConflict))?;
        Ok(MachineFile {
            instance_id: request.instance_id,
            operation: request.operation,
            answer,
        })
    }

    /// Reads the machine and refuses one that cannot serve a filesystem operation.
    fn admit_file(&mut self, instance_id: &InstanceId) -> Result<(), ManagedFailure> {
        let stored = self
            .load_machine(instance_id)?
            .ok_or(ManagedFailure::State(ManagedStateError::MachineNotFound))?;
        let VersionedMachine { machine, .. } = stored;
        match machine.phase {
            DurablePhase::Active { active } => self.ensure_backend(&active),
            // A machine mid-command, mid-launch, or mid-release is not one a second operation may
            // address: its own operation has not finished deciding what it is.
            DurablePhase::Executing { .. }
            | DurablePhase::Launching { .. }
            | DurablePhase::Terminating { .. } => {
                Err(ManagedFailure::State(ManagedStateError::RecoveryRequired))
            }
            DurablePhase::Terminal { .. } => {
                Err(ManagedFailure::State(ManagedStateError::MachineStopped))
            }
        }
    }

    fn ensure_backend(&self, active: &ActiveMachine) -> Result<(), ManagedFailure> {
        if active.backend() == self.backend.kind() {
            Ok(())
        } else {
            Err(ManagedFailure::StateStore(
                crate::StateStoreFailureKind::Corrupt,
            ))
        }
    }
}

/// Takes the answer only when it names the operation and Instance that were asked about.
///
/// A backend that answered about a different Instance would otherwise report one sandbox's
/// filesystem as another's, which is the one answer this surface may never give by mistake.
fn accept(
    observation: FileObservation,
    operation_id: &OperationId,
    instance_id: &InstanceId,
) -> Option<FileAnswer> {
    (observation.operation_id() == operation_id && observation.instance_id() == instance_id)
        .then(|| observation.into_answer())
}

/// One completed filesystem operation and what it answered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineFile {
    pub(super) instance_id: InstanceId,
    pub(super) operation: FileOperation,
    pub(super) answer: FileAnswer,
}

impl MachineFile {
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn operation(&self) -> &FileOperation {
        &self.operation
    }

    #[must_use]
    pub const fn answer(&self) -> &FileAnswer {
        &self.answer
    }
}

/// One filesystem operation addressed to one managed Instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileMachineRequest {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    pub(super) operation: FileOperation,
}

impl FileMachineRequest {
    /// The operation this request carries.
    ///
    /// Published so a caller that built the request can report what it asked for without keeping
    /// a second copy of it beside the request.
    #[must_use]
    pub const fn operation(&self) -> &FileOperation {
        &self.operation
    }

    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        operation: FileOperation,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            operation,
        }
    }
}
