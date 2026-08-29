//! The bounded idempotency registry behind [`Pool::claim`], backed by the durable ledger.
//!
//! The registry is a cache, not the record: [`Pool::open`] seeds it from `Ledger::claims`,
//! every completed claim adds its operation to the recorded set, and a miss on an operation
//! the ledger already recorded is answered from `Ledger::claim_of` instead of granting a
//! second worker.
//! An eviction and a restart are therefore both invisible to a replay.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::Instant,
};

use super::{ClaimClass, ClaimError, ClaimOutcome};
use crate::{
    ClaimRecord, LedgerError, OperationId, OverloadGate, Overloaded, Phase, Pool, RecordKind,
    RequestFingerprint, ResourceBroker, WorkerLauncher,
};

#[derive(Clone, Copy)]
enum Binding {
    InFlight {
        fingerprint: RequestFingerprint,
    },
    Done {
        fingerprint: RequestFingerprint,
        outcome: ClaimOutcome,
    },
}

/// The bounded idempotency registry.
#[derive(Default)]
pub(crate) struct Registry {
    bindings: BTreeMap<OperationId, Binding>,
    order: VecDeque<OperationId>,
    recorded: BTreeSet<OperationId>,
}

impl Registry {
    /// Seeds the registry from every claim the ledger holds, newest bindings first.
    pub(crate) fn seed(&mut self, claims: &[ClaimRecord], limit: usize) {
        for claim in claims {
            self.recorded.insert(claim.operation);
        }
        for claim in claims.iter().rev() {
            if self.bindings.len() >= limit {
                break;
            }
            if self.bindings.contains_key(&claim.operation) {
                continue;
            }
            if let Some(binding) = binding_of(claim) {
                self.bindings.insert(claim.operation, binding);
                self.order.push_front(claim.operation);
            }
        }
    }
}

fn binding_of(claim: &ClaimRecord) -> Option<Binding> {
    Some(Binding::Done {
        fingerprint: claim.fingerprint,
        outcome: outcome_of(claim)?,
    })
}

fn outcome_of(claim: &ClaimRecord) -> Option<ClaimOutcome> {
    Some(ClaimOutcome {
        worker: claim.worker,
        lease_generation: claim.lease_generation,
        operation: claim.operation,
        class: ClaimClass::from_code(claim.class)?,
    })
}

fn replay(
    claim: &ClaimRecord,
    fingerprint: RequestFingerprint,
) -> Result<Option<ClaimOutcome>, ClaimError> {
    if claim.fingerprint != fingerprint {
        return Err(ClaimError::OperationConflict {
            operation: claim.operation,
            recorded: claim.fingerprint,
            presented: fingerprint,
        });
    }
    outcome_of(claim)
        .map(Some)
        .ok_or(ClaimError::Ledger(LedgerError::Invariant {
            worker: claim.worker,
            kind: RecordKind::Claiming,
        }))
}

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// Returns `Some(outcome)` for a completed replay, `None` once the binding is reserved.
    pub(super) fn reserve_binding(
        &self,
        operation: OperationId,
        fingerprint: RequestFingerprint,
        started: Instant,
    ) -> Result<Option<ClaimOutcome>, ClaimError> {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match registry.bindings.get(&operation).copied() {
                Some(Binding::Done {
                    fingerprint: recorded,
                    outcome,
                }) => {
                    return if recorded == fingerprint {
                        Ok(Some(outcome))
                    } else {
                        Err(ClaimError::OperationConflict {
                            operation,
                            recorded,
                            presented: fingerprint,
                        })
                    };
                }
                Some(Binding::InFlight {
                    fingerprint: recorded,
                }) => {
                    if recorded != fingerprint {
                        return Err(ClaimError::OperationConflict {
                            operation,
                            recorded,
                            presented: fingerprint,
                        });
                    }
                    let waited = started.elapsed();
                    let Some(remaining) = self.limits().claim_deadline.checked_sub(waited) else {
                        return Err(ClaimError::Deadline { operation, waited });
                    };
                    registry = self
                        .registry_changed
                        .wait_timeout(registry, remaining)
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .0;
                }
                None => {
                    if registry.recorded.contains(&operation)
                        && let Some(claim) = self
                            .ledger()
                            .claim_of(operation)
                            .map_err(ClaimError::Ledger)?
                    {
                        return replay(&claim, fingerprint);
                    }
                    self.evict_bindings(&mut registry)?;
                    registry
                        .bindings
                        .insert(operation, Binding::InFlight { fingerprint });
                    registry.order.push_back(operation);
                    return Ok(None);
                }
            }
        }
    }

    /// Drops one completed binding to make room; the ledger can rebuild every one of them.
    fn evict_bindings(&self, registry: &mut Registry) -> Result<(), ClaimError> {
        let limit = self.limits().binding_limit;
        if registry.bindings.len() < limit {
            return Ok(());
        }
        let completed = |operation: &OperationId| {
            matches!(registry.bindings.get(operation), Some(Binding::Done { .. }))
        };
        let dead = |operation: &OperationId| match registry.bindings.get(operation) {
            Some(Binding::Done { outcome, .. }) => self
                .find_slot(outcome.worker)
                .is_none_or(|slot| slot.observe().phase == Phase::Dead),
            _ => false,
        };
        let position = registry
            .order
            .iter()
            .position(dead)
            .or_else(|| registry.order.iter().position(completed));
        match position {
            Some(position) => {
                if let Some(operation) = registry.order.remove(position) {
                    registry.bindings.remove(&operation);
                }
                Ok(())
            }
            None => Err(ClaimError::Overloaded(Overloaded {
                gate: OverloadGate::ClaimRegistry,
                current: registry.bindings.len(),
                limit,
            })),
        }
    }

    pub(super) fn clear_binding(&self, operation: OperationId) {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.bindings.remove(&operation);
        registry.order.retain(|candidate| *candidate != operation);
        self.registry_changed.notify_all();
    }

    pub(super) fn finish_binding(
        &self,
        operation: OperationId,
        fingerprint: RequestFingerprint,
        outcome: ClaimOutcome,
    ) {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.recorded.insert(operation);
        registry.bindings.insert(
            operation,
            Binding::Done {
                fingerprint,
                outcome,
            },
        );
        self.registry_changed.notify_all();
    }
}
