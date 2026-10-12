//! What the pool holds against what it was asked to hold, for the readback.

use super::{locked, pool};

/// What the pool holds and has been asked to hold, for one target.
pub(crate) struct TargetSnapshot {
    pub(crate) reference: String,
    pub(crate) generation_id: String,
    pub(crate) memory_mib: u64,
    pub(crate) target: usize,
    pub(crate) prepared: usize,
    pub(crate) building: bool,
    pub(crate) last_failure: Option<String>,
    pub(crate) claimed_primed: u64,
    pub(crate) claimed_miss: u64,
}

/// Launches that wanted a shape no target covers.
pub(crate) struct DemandSnapshot {
    pub(crate) reference: String,
    pub(crate) memory_mib: u64,
    /// The memory the Generation boots with: the only memory a target can name for it.
    pub(crate) boot_memory_mib: u64,
    pub(crate) claimed_primed: u64,
    pub(crate) claimed_miss: u64,
}

pub(crate) struct PoolSnapshot {
    pub(crate) targets: Vec<TargetSnapshot>,
    pub(crate) uncovered_demand: Vec<DemandSnapshot>,
    pub(crate) generic_target: usize,
    pub(crate) generic_prepared: usize,
}

/// Counts what the pool holds against what it was asked to hold.
pub(crate) fn snapshot() -> PoolSnapshot {
    let desired = locked(&pool().desired);
    let available = locked(&pool().available);
    let targets = desired
        .targets
        .iter()
        .map(|target| {
            let demand = desired
                .demand
                .get(&(target.key.reference.clone(), target.key.memory_mib))
                .copied()
                .unwrap_or_default();
            TargetSnapshot {
                reference: target.key.reference.clone(),
                generation_id: target.key.generation_id.as_str().to_owned(),
                memory_mib: target.key.memory_mib,
                target: target.count,
                prepared: available
                    .iter()
                    .filter(|host| host.primed.as_ref() == Some(&target.key))
                    .count(),
                building: target.building,
                last_failure: target.last_failure.clone(),
                claimed_primed: demand.primed,
                claimed_miss: demand.miss,
            }
        })
        .collect();
    let uncovered_demand = desired
        .demand
        .iter()
        .filter(|((reference, memory), _)| {
            !desired.targets.iter().any(|target| {
                &target.key.reference == reference && target.key.memory_mib == *memory
            })
        })
        .map(|((reference, memory), demand)| DemandSnapshot {
            reference: reference.clone(),
            memory_mib: *memory,
            boot_memory_mib: demand.boot_mib,
            claimed_primed: demand.primed,
            claimed_miss: demand.miss,
        })
        .collect();
    PoolSnapshot {
        targets,
        uncovered_demand,
        generic_target: desired.generic,
        generic_prepared: available
            .iter()
            .filter(|host| host.primed.is_none())
            .count(),
    }
}
