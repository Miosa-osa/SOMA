//! The capacity equation of the visual atlas: the safe count of one uniform shape is the
//! minimum over every independent limit, including the section 14 burst limits that
//! [`Admission::reserve`] enforces, so the estimate and the admitted count agree.

use crate::{CapacityRejection, CertifiedProfile, Demand, Gate, MemoryClass, ValidShape};

/// The per-dimension bounds and their minimum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityEstimate {
    /// CPU bound at strict 1:1.
    pub cpu_strict: u64,
    /// CPU bound under the class ratio.
    pub cpu_overcommitted: u64,
    /// Memory bound including measured overhead.
    pub memory: u64,
    /// Storage bound; unbounded when the shape reserves no storage.
    pub storage: u64,
    /// Network bound; unbounded when the shape needs no unit.
    pub network: u64,
    /// Process and descriptor bound.
    pub process: u64,
    /// Operator resident limit.
    pub operator: u64,
    /// Runnable vCPU burst bound.
    pub runnable: u64,
    /// Private dirty memory burst bound.
    pub dirty: u64,
    /// Concurrent Launch bound.
    pub launches: u64,
    /// The minimum.
    pub safe_count: u64,
    /// The first gate that produced the minimum.
    pub binding: Gate,
}

/// Estimates the safe count of `shape` on an empty `profile`.
///
/// # Errors
///
/// Returns [`Gate::Arithmetic`] on overflow.
pub fn estimate(
    profile: &CertifiedProfile,
    shape: &ValidShape,
) -> Result<CapacityEstimate, CapacityRejection> {
    let demand = Demand::of(profile, shape)?;
    let profile = profile.profile();
    let shape = shape.shape();
    let units = u64::from(profile.admissible_cpu_units());
    let ratio = profile.overcommit.ratio(shape.workload);
    let cpu_strict = divide(units, u64::from(shape.vcpus));
    let cpu_overcommitted = divide(
        units.saturating_mul(u64::from(ratio.vcpus)),
        u64::from(shape.vcpus).saturating_mul(u64::from(ratio.threads)),
    );
    let memory_cost = match shape.memory_class {
        MemoryClass::Guaranteed => demand.guaranteed_bytes,
        MemoryClass::Elastic { .. } => demand.elastic_bytes,
    };
    let memory_budget = match shape.memory_class {
        MemoryClass::Guaranteed => profile.admissible_memory_bytes(),
        MemoryClass::Elastic { .. } => profile.memory.elastic_budget_bytes,
    };
    let memory = divide(memory_budget, memory_cost);
    let storage = divide(profile.storage.private_budget_bytes, demand.storage_bytes);
    let network = divide(u64::from(profile.network.units), demand.network_units);
    let process = u64::from(profile.process.processes).min(divide(
        u64::from(profile.process.descriptors),
        demand.descriptors,
    ));
    let operator = u64::from(profile.limits.resident_instances);
    let runnable = divide(
        u64::from(profile.limits.runnable_vcpus),
        demand.runnable_vcpus,
    );
    let dirty = divide(profile.limits.dirty_memory_bytes, demand.dirty_bytes);
    let launches = u64::from(profile.limits.concurrent_launches);
    let bounds = [
        (Gate::CpuUnits, cpu_overcommitted),
        (Gate::GuaranteedMemory, memory),
        (Gate::PrivateStorage, storage),
        (Gate::NetworkInventory, network),
        (Gate::ProcessLimit, process),
        (Gate::OperatorSafetyLimit, operator),
        (Gate::RunnableVcpus, runnable),
        (Gate::DirtyMemory, dirty),
        (Gate::ConcurrentLaunches, launches),
    ];
    let (binding, safe_count) = bounds
        .iter()
        .copied()
        .min_by_key(|(_, count)| *count)
        .unwrap_or((Gate::Arithmetic, 0));
    Ok(CapacityEstimate {
        cpu_strict,
        cpu_overcommitted,
        memory,
        storage,
        network,
        process,
        operator,
        runnable,
        dirty,
        launches,
        safe_count,
        binding: if matches!(shape.memory_class, MemoryClass::Elastic { .. })
            && binding == Gate::GuaranteedMemory
        {
            Gate::ElasticMemory
        } else {
            binding
        },
    })
}

const fn divide(budget: u64, per_instance: u64) -> u64 {
    match budget.checked_div(per_instance) {
        Some(count) => count,
        None => u64::MAX,
    }
}
