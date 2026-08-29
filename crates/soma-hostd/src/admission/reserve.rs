//! Atomic multi-dimension reservation with rollback.
//!
//! A reservation computes every demand with checked arithmetic, applies it to a copy of the
//! committed usage gate by gate, places it on one NUMA node, and only then swaps the copy
//! in; a rejection at any gate leaves the committed usage untouched.

use std::sync::Mutex;

use super::usage::{Usage, census_milli_units, gate};
use crate::Demand;
use crate::{
    CapacityRejection, Gate, HostProfile, InstanceShape, NodeDemand, NodeFree, NodeId,
    NumaPlacement, NumaRejection,
};

/// One committed reservation; it must be returned to [`Admission::release`].
#[derive(Debug, Eq, PartialEq)]
pub struct Reservation {
    pub(super) id: u64,
    pub(super) node: NodeId,
    pub(super) demand: Demand,
    pub(super) launching: bool,
}

impl Reservation {
    /// The placed node.
    #[must_use]
    pub const fn node(&self) -> NodeId {
        self.node
    }

    /// The demand committed.
    #[must_use]
    pub const fn demand(&self) -> Demand {
        self.demand
    }
}

/// The host's admission state.
pub struct Admission {
    profile: HostProfile,
    placement: Box<dyn NumaPlacement>,
    state: Mutex<(Usage, u64)>,
}

impl Admission {
    /// Opens admission for `profile` with `placement`.
    #[must_use]
    pub fn new(profile: HostProfile, placement: impl NumaPlacement + 'static) -> Self {
        let nodes = profile.cpu.numa_nodes.max(1) as usize;
        let usage = Usage {
            node_cpu: vec![0; nodes],
            node_vcpus: vec![[0; 3]; nodes],
            node_memory: vec![0; nodes],
            ..Usage::default()
        };
        Self {
            profile,
            placement: Box::new(placement),
            state: Mutex::new((usage, 1)),
        }
    }

    /// The profile.
    #[must_use]
    pub const fn profile(&self) -> &HostProfile {
        &self.profile
    }

    /// A copy of the committed usage.
    #[must_use]
    pub fn usage(&self) -> Usage {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0
            .clone()
    }

    /// Reserves every dimension for `shape` atomically.
    ///
    /// # Errors
    ///
    /// Returns the first refusing gate; nothing is committed on error.
    pub fn reserve(&self, shape: &InstanceShape) -> Result<Reservation, CapacityRejection> {
        let demand = Demand::of(&self.profile, shape)?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut candidate = state.0.apply(&demand, &self.profile)?;
        let marginal = candidate
            .cpu_milli_units
            .saturating_sub(state.0.cpu_milli_units);
        let free = state.0.free_nodes(&self.profile);
        let memory = demand.guaranteed_bytes.saturating_add(demand.elastic_bytes);
        let node = self
            .placement
            .place(
                NodeDemand {
                    cpu_milli_units: marginal,
                    memory_bytes: memory,
                },
                &free,
            )
            .map_err(|rejection| numa_rejection(rejection, &free, marginal))?;
        let index = demand.workload.index();
        if let Some(census) = candidate.node_vcpus.get_mut(node.0 as usize) {
            census[index] = census[index].saturating_add(demand.vcpus);
            let recomputed = census_milli_units(census, &self.profile).unwrap_or(u64::MAX);
            if let Some(cpu) = candidate.node_cpu.get_mut(node.0 as usize) {
                *cpu = recomputed;
            }
        }
        if let Some(bytes) = candidate.node_memory.get_mut(node.0 as usize) {
            *bytes = bytes.saturating_add(memory);
        }
        let id = state.1;
        state.1 += 1;
        state.0 = candidate;
        Ok(Reservation {
            id,
            node,
            demand,
            launching: true,
        })
    }

    /// Marks the Launch finished so the concurrent-launch slot is free.
    pub fn launched(&self, reservation: &mut Reservation) {
        if reservation.launching {
            reservation.launching = false;
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.0.launches = state.0.launches.saturating_sub(1);
        }
    }

    /// Takes one bounded cleanup slot for the teardown of `reservation`.
    ///
    /// # Errors
    ///
    /// Returns [`Gate::CleanupSlots`] when every slot is busy; the reservation is unchanged.
    pub fn begin_cleanup(&self, reservation: &Reservation) -> Result<u64, CapacityRejection> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.0.cleanups = gate(
            Gate::CleanupSlots,
            state.0.cleanups,
            1,
            u64::from(self.profile.limits.cleanup_slots),
        )?;
        Ok(reservation.id)
    }

    /// Releases every dimension of `reservation` and its cleanup slot when one was taken.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "consuming the reservation makes a double release impossible"
    )]
    pub fn release(&self, reservation: Reservation, cleanup_slot: Option<u64>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.0.subtract(&reservation, &self.profile);
        if cleanup_slot.is_some() {
            state.0.cleanups = state.0.cleanups.saturating_sub(1);
        }
    }
}

fn numa_rejection(
    rejection: NumaRejection,
    free: &[NodeFree],
    requested: u64,
) -> CapacityRejection {
    let (committed, limit) = match rejection {
        NumaRejection::Fragmented { .. } => free
            .first()
            .map_or((0, 0), |node| (0, node.cpu_milli_units)),
        NumaRejection::NoNodes => (0, 0),
        NumaRejection::MultiNodeUnsupported { nodes } => (u64::from(nodes), 1),
    };
    CapacityRejection {
        gate: Gate::NumaFit,
        requested,
        committed,
        limit,
    }
}
