use crate::{Backend, GenerationId, InstanceId, MachineName, OperationId, StateStore};

use crate::engine::{
    Engine, ManagedFailure, ManagedStateError,
    machine_state::{DurableMachine, DurablePhase, RecoverySource},
};

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Installs recovery authority for an absent Instance under a caller-held restore lease.
    ///
    /// The trusted caller must fence the source and verify publication of the exact checkpoint
    /// in the callback. Hold the same per-Instance lease until this method returns.
    ///
    /// # Errors
    ///
    /// Refuses a conflicting Instance or a failed verification. The same authority can retry a
    /// partially published install without making the Instance available to restore early.
    #[allow(
        clippy::too_many_arguments,
        reason = "the independent receipt binds every input"
    )]
    pub fn install_recovery_source<E>(
        &mut self,
        instance: &InstanceId,
        owner: &MachineName,
        checkpoint: &OperationId,
        generation: &GenerationId,
        receipt_digest: &GenerationId,
        publish: impl FnOnce() -> Result<(), E>,
    ) -> Result<Result<(), E>, ManagedFailure> {
        let source = RecoverySource {
            owner: owner.clone(),
            checkpoint: checkpoint.clone(),
            generation: generation.clone(),
            receipt_digest: receipt_digest.clone(),
            backend: self.backend.kind(),
        };
        let revision = match self.load_machine(instance)? {
            Some(stored) => match &stored.machine.phase {
                DurablePhase::Recovery {
                    source: existing,
                    installed,
                } if existing.as_ref() == &source => {
                    if *installed {
                        return Ok(publish());
                    }
                    stored.revision
                }
                _ => return Err(ManagedFailure::State(ManagedStateError::OperationConflict)),
            },
            None => self.create_machine(&DurableMachine::recovery(
                instance.clone(),
                source.clone(),
                false,
            ))?,
        };
        if let Err(error) = publish() {
            return Ok(Err(error));
        }
        self.replace_machine(
            revision,
            &DurableMachine::recovery(instance.clone(), source, true),
        )?;
        Ok(Ok(()))
    }
    /// Whether an installed recovery reservation binds the supplied checkpoint transfer.
    ///
    /// Callers must separately enforce ownership and hold the Instance lease.
    ///
    /// # Errors
    ///
    /// Propagates a durable state read failure.
    pub fn permits_checkpoint_transfer(
        &mut self,
        instance: &InstanceId,
        checkpoint: &OperationId,
        generation: &GenerationId,
        receipt_digest: &GenerationId,
    ) -> Result<bool, ManagedFailure> {
        let Some(stored) = self.load_machine(instance)? else {
            return Ok(false);
        };
        let basis = match &stored.machine.phase {
            DurablePhase::Launching {
                prior_stop: Some(prior),
                ..
            } => prior,
            _ => &stored.machine,
        };
        match &basis.phase {
            DurablePhase::Recovery { source, installed } => Ok(*installed
                && &source.checkpoint == checkpoint
                && &source.generation == generation
                && &source.receipt_digest == receipt_digest),
            _ => Ok(true),
        }
    }
}
