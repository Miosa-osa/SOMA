//! Releasing the durable records of Instances that ended (ADR 0055).
//!
//! A terminal record is written only after the machine's cleanup completed, so it owes the host
//! nothing. What it still holds is the evidence a repeated operation replays, and that evidence
//! is bounded by a retention rule the caller applies, because only the caller knows how long a
//! repeated request can still arrive. This module supplies the two halves such a rule needs: a
//! census of which records are terminal, and the release of one of them.
//!
//! A release removes a record only when it is terminal at the revision that was read, so a live
//! record, or one that moved since it was judged, is never taken.

use crate::{Backend, InstanceId, OperationKind, StateStore};

use super::{
    Engine, ManagedFailure,
    machine_state::{DurablePhase, TerminalBasis},
    managed_list::is_unreadable_record,
    managed_state::store_failure,
};

/// What a release census found in the durable store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalCensus {
    records: usize,
    terminal: Vec<InstanceId>,
}

impl TerminalCensus {
    /// Builds a census, for a facade that keeps its records somewhere other than this engine.
    #[must_use]
    pub const fn new(records: usize, terminal: Vec<InstanceId>) -> Self {
        Self { records, terminal }
    }

    /// Every record the store held, terminal or not, including one that could not be decoded.
    #[must_use]
    pub const fn records(&self) -> usize {
        self.records
    }

    /// The Instances whose record reached a terminal phase, in identity order.
    #[must_use]
    pub fn terminal(&self) -> &[InstanceId] {
        &self.terminal
    }

    /// Takes the terminal Instances out of the census.
    #[must_use]
    pub fn into_terminal(self) -> Vec<InstanceId> {
        self.terminal
    }
}

/// What one release did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MachineRelease {
    /// The terminal record was removed, and the identity now reads as absent.
    Released,
    /// No record exists, so there was nothing to remove.
    Absent,
    /// The record was kept because the Instance may still own resources or be recoverable.
    Kept,
}

impl<B: Backend, S: StateStore> Engine<B, S> {
    /// Counts the store's records and names the ones that reached a terminal phase.
    ///
    /// A record that cannot be decoded is counted and never named: nothing proves it terminal, so
    /// it stays for an operator rather than being removed on the strength of a guess.
    ///
    /// # Errors
    ///
    /// Returns a typed durable-store failure when the store cannot be enumerated, or a record
    /// cannot be read at all.
    pub fn terminal_census(&mut self) -> Result<TerminalCensus, ManagedFailure> {
        let mut identities = self.state.list().map_err(store_failure)?;
        identities.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        let mut census = TerminalCensus::default();
        for instance_id in identities {
            let terminal = match self.load_machine(&instance_id) {
                Ok(Some(stored)) => matches!(stored.machine.phase, DurablePhase::Terminal { .. }),
                // Released between the enumeration and this read, so it is not held any more.
                Ok(None) => continue,
                Err(ManagedFailure::StateStore(kind)) if is_unreadable_record(kind) => false,
                Err(failure) => return Err(failure),
            };
            census.records = census.records.saturating_add(1);
            if terminal {
                census.terminal.push(instance_id);
            }
        }
        Ok(census)
    }

    /// Releases the durable record of one Instance that reached a terminal phase.
    ///
    /// After a release the identity reads as absent: an inspect, stop, or destroy answers
    /// `MachineNotFound`, and a repeated operation no longer replays its receipt.
    ///
    /// # Errors
    ///
    /// Returns a typed durable-store failure. `Conflict` means the record changed between the
    /// read and the removal, and nothing was removed.
    pub fn release_machine(
        &mut self,
        instance_id: &InstanceId,
    ) -> Result<MachineRelease, ManagedFailure> {
        let Some(stored) = self.load_machine(instance_id)? else {
            return Ok(MachineRelease::Absent);
        };
        if !matches!(stored.machine.phase, DurablePhase::Terminal { .. }) {
            return Ok(MachineRelease::Kept);
        }
        if let DurablePhase::Terminal {
            operation: OperationKind::Stop,
            basis,
            ..
        } = &stored.machine.phase
            && let TerminalBasis::Active { active } = basis.as_ref()
            && let Some(generation) = active
                .workload()
                .and_then(crate::WorkloadIdentity::generation_id)
            && active.backend() == self.backend.kind()
            && self
                .backend
                .retains_stopped_checkpoint(instance_id, generation)
                .map_err(|failure| ManagedFailure::Backend(failure.kind()))?
        {
            return Ok(MachineRelease::Kept);
        }
        self.state
            .release(instance_id, stored.revision)
            .map_err(store_failure)?;
        Ok(MachineRelease::Released)
    }
}
