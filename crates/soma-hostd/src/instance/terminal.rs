//! The idempotent terminal operation and the evidence that proves it.
//!
//! Destroy is answered from evidence rather than from absence: the first call releases the
//! worker the Instance owns, and every later call reads the same durable ledger the release
//! wrote, so a repeat returns the identical receipt instead of a refusal that would make a
//! retrying client believe its Machine is still running.

use crate::{
    InstanceError, InstanceId, LifecycleError, ResourceBroker, Runtime, WorkerId, WorkerLauncher,
};

/// What a terminal operation proved about one Instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalReceipt {
    /// The Instance.
    pub instance: InstanceId,
    /// The worker that served it.
    pub worker: WorkerId,
    /// Whether teardown and resource release both reported completion.
    ///
    /// A receipt that is not complete is not a failure to destroy; it states that ownership
    /// remains uncertain and is retained for reconciliation.
    pub complete: bool,
}

impl<L: WorkerLauncher, R: ResourceBroker> Runtime<L, R> {
    /// Destroys one Instance and returns its terminal receipt.
    ///
    /// The call is idempotent: destroying an Instance this Runtime has already reclaimed is
    /// answered from the durable ledger with the same receipt.
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::Unknown`] when no live Instance and no durable record carries
    /// the identity, or the pool's refusal when the worker cannot yet be released.
    pub fn destroy(&self, instance: InstanceId) -> Result<TerminalReceipt, InstanceError> {
        let Some(live) = self.locked().get(instance) else {
            return self.recorded_terminal(instance);
        };
        match self.pool().release(live.worker) {
            Ok(evidence) => {
                self.locked().remove(instance);
                Ok(TerminalReceipt {
                    instance,
                    worker: live.worker,
                    complete: evidence.destroyed.complete && evidence.released.complete,
                })
            }
            // The pool no longer owns the worker, so the Instance is already terminal and the
            // binding this Runtime still held was stale; the ledger states what happened.
            Err(LifecycleError::Unknown(_)) => {
                self.locked().remove(instance);
                self.recorded_terminal(instance)
            }
            // Any other refusal leaves the Instance owned, because nothing has been torn down
            // and a client that retries must still be able to address it.
            Err(error) => Err(InstanceError::Lifecycle(error)),
        }
    }

    /// Reads the terminal disposition of an Instance this Runtime no longer owns.
    fn recorded_terminal(&self, instance: InstanceId) -> Result<TerminalReceipt, InstanceError> {
        let entries = self
            .pool()
            .ledger()
            .entries()
            .map_err(InstanceError::Ledger)?;
        let entry = entries
            .values()
            .find(|entry| entry.instance == Some(instance))
            .ok_or(InstanceError::Unknown(instance))?;
        Ok(TerminalReceipt {
            instance,
            worker: entry.worker,
            complete: entry.phase.is_terminal(),
        })
    }
}
