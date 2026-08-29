//! The bounded idempotency registry behind [`Pool::claim`].

use std::{
    collections::{BTreeMap, VecDeque},
    time::Instant,
};

use super::{ClaimError, ClaimOutcome};
use crate::{
    OperationId, OverloadGate, Overloaded, Phase, Pool, RequestFingerprint, ResourceBroker,
    WorkerLauncher,
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
                    self.evict_dead_bindings(&mut registry)?;
                    registry
                        .bindings
                        .insert(operation, Binding::InFlight { fingerprint });
                    registry.order.push_back(operation);
                    return Ok(None);
                }
            }
        }
    }

    fn evict_dead_bindings(&self, registry: &mut Registry) -> Result<(), ClaimError> {
        let limit = self.limits().binding_limit;
        if registry.bindings.len() < limit {
            return Ok(());
        }
        let position =
            registry
                .order
                .iter()
                .position(|operation| match registry.bindings.get(operation) {
                    Some(Binding::Done { outcome, .. }) => self
                        .find_slot(outcome.worker)
                        .is_none_or(|slot| slot.observe().phase == Phase::Dead),
                    _ => false,
                });
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
