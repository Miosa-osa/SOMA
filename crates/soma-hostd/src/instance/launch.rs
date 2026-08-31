//! The Launch transaction: one operation identity produces exactly one Instance.
//!
//! Every step that can fail runs before the binding exists, so a refused Launch leaves the
//! ownership table exactly as it was and the pool's own reverse cleanup owns the worker.

use crate::{
    AssignmentIntent, ClaimError, ClaimOutcome, InstanceError, InstanceView, Launched,
    RequestFingerprint, ResourceBroker, Runtime, WorkerLauncher,
};

impl<L: WorkerLauncher, R: ResourceBroker> Runtime<L, R> {
    /// Launches one Instance for `intent` and takes ownership of it.
    ///
    /// The Instance outlives this call and every client: it is addressable by
    /// [`Runtime::get`] until a terminal operation or proven worker death reclaims it.
    ///
    /// # Errors
    ///
    /// Returns the typed refusal; a replay of the same operation returns the same Instance,
    /// and a changed intent under that operation conflicts without any effect.
    pub fn launch(&self, intent: &AssignmentIntent) -> Result<Launched, InstanceError> {
        self.reclaim();
        let fingerprint = intent.fingerprint();
        if let Some(live) = self.locked().get(intent.instance) {
            return owned(live, intent, fingerprint);
        }
        let claim = self
            .pool()
            .claim(intent.operation, fingerprint)
            .map_err(InstanceError::Claim)?;
        match claim.grant {
            Some(grant) => {
                let evidence = self
                    .pool()
                    .transfer(grant, intent)
                    .map_err(InstanceError::Transfer)?;
                Ok(Launched::Live(self.bind(InstanceView {
                    instance: evidence.instance,
                    worker: evidence.worker,
                    lease_generation: evidence.lease_generation,
                    operation: evidence.operation,
                    fingerprint,
                    launch: evidence.launch,
                })))
            }
            None => self.rebind(intent, fingerprint, claim.outcome),
        }
    }

    /// Answers a replay whose Instance this Runtime does not currently own.
    ///
    /// The pool recorded the claim, so the operation is not free to create a second Instance;
    /// what the reply may say is decided by the worker the claim is bound to, exactly as the
    /// allocator answers a replayed claim.
    fn rebind(
        &self,
        intent: &AssignmentIntent,
        fingerprint: RequestFingerprint,
        outcome: ClaimOutcome,
    ) -> Result<Launched, InstanceError> {
        match self.pool().inspect(outcome.worker) {
            Some(view) if view.phase.is_terminal() => {
                Err(InstanceError::Terminated(intent.instance))
            }
            Some(view) => view.launch.map_or(
                Ok(Launched::Replayed {
                    worker: outcome.worker,
                    lease_generation: outcome.lease_generation,
                }),
                |launch| {
                    Ok(Launched::Live(self.bind(InstanceView {
                        instance: intent.instance,
                        worker: outcome.worker,
                        lease_generation: outcome.lease_generation,
                        operation: outcome.operation,
                        fingerprint,
                        launch,
                    })))
                },
            ),
            None => Err(InstanceError::Terminated(intent.instance)),
        }
    }
}

/// Decides what a Launch that names an Instance this Host already owns may receive.
///
/// A replay is only a replay when the whole request is the same one: the same operation
/// presenting a different intent is a changed intent and conflicts, and any other operation
/// is refused, because adopting a live Instance would give one Machine two owners.
fn owned(
    live: InstanceView,
    intent: &AssignmentIntent,
    fingerprint: RequestFingerprint,
) -> Result<Launched, InstanceError> {
    if live.operation != intent.operation {
        return Err(InstanceError::Occupied {
            instance: intent.instance,
            holder: live.operation,
            presented: intent.operation,
        });
    }
    if live.fingerprint != fingerprint {
        return Err(InstanceError::Claim(ClaimError::OperationConflict {
            operation: intent.operation,
            recorded: live.fingerprint,
            presented: fingerprint,
        }));
    }
    Ok(Launched::Live(live))
}
