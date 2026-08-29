//! The committed usage every capacity gate is checked against, and the exact CPU census the
//! class overcommit ratio is applied to once rather than once per Instance.

use crate::{CapacityRejection, Demand, Gate, HostProfile, NodeFree, NodeId, WorkloadClass};

use super::reserve::Reservation;

/// The milli-units one class census costs under `ratio`, rounded up exactly once.
pub(super) fn class_milli_units(vcpus: u64, ratio: crate::Ratio) -> Option<u64> {
    let numerator = vcpus
        .checked_mul(1000)?
        .checked_mul(u64::from(ratio.threads))?;
    let denominator = u64::from(ratio.vcpus);
    numerator
        .checked_add(denominator.checked_sub(1)?)
        .map(|rounded| rounded / denominator)
}

/// The milli-units a whole per-class vCPU census costs under `profile`.
pub(super) fn census_milli_units(census: &[u64; 3], profile: &HostProfile) -> Option<u64> {
    let mut total = 0_u64;
    for class in WorkloadClass::ALL {
        let cost = class_milli_units(census[class.index()], profile.overcommit.ratio(class))?;
        total = total.checked_add(cost)?;
    }
    Some(total)
}

/// Committed usage across every gate.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    /// CPU milli-units under the class ratios; derived from `vcpus_by_class`.
    pub cpu_milli_units: u64,
    /// Committed raw vCPUs per workload class, in [`WorkloadClass::ALL`] order.
    pub vcpus_by_class: [u64; 3],
    /// Guaranteed bytes.
    pub guaranteed_bytes: u64,
    /// Elastic bytes.
    pub elastic_bytes: u64,
    /// Storage bytes.
    pub storage_bytes: u64,
    /// Network units.
    pub network_units: u64,
    /// Processes.
    pub processes: u64,
    /// Descriptors.
    pub descriptors: u64,
    /// Resident Instances.
    pub residents: u64,
    /// Launches in progress.
    pub launches: u64,
    /// Runnable vCPUs.
    pub runnable_vcpus: u64,
    /// Dirty bytes.
    pub dirty_bytes: u64,
    /// Cleanups in progress.
    pub cleanups: u64,
    /// Per-node CPU milli-units, derived from `node_vcpus`.
    pub node_cpu: Vec<u64>,
    /// Per-node raw vCPUs per workload class.
    pub node_vcpus: Vec<[u64; 3]>,
    /// Per-node memory bytes.
    pub node_memory: Vec<u64>,
}

pub(super) fn gate(
    gate: Gate,
    committed: u64,
    requested: u64,
    limit: u64,
) -> Result<u64, CapacityRejection> {
    let reject = |committed| CapacityRejection {
        gate,
        requested,
        committed,
        limit,
    };
    let total = committed
        .checked_add(requested)
        .ok_or_else(|| reject(committed))?;
    if total > limit {
        return Err(reject(committed));
    }
    Ok(total)
}

impl Usage {
    pub(super) fn apply(
        &self,
        demand: &Demand,
        profile: &HostProfile,
    ) -> Result<Self, CapacityRejection> {
        let cpu_limit = u64::from(profile.admissible_cpu_units()) * 1000;
        let memory_limit = profile.admissible_memory_bytes();
        let limits = profile.limits;
        let mut next = self.clone();
        let overflow = || CapacityRejection {
            gate: Gate::Arithmetic,
            requested: demand.vcpus,
            committed: self.cpu_milli_units,
            limit: cpu_limit,
        };
        let mut census = self.vcpus_by_class;
        let index = demand.workload.index();
        census[index] = census[index]
            .checked_add(demand.vcpus)
            .ok_or_else(overflow)?;
        let committed = census_milli_units(&census, profile).ok_or_else(overflow)?;
        if committed > cpu_limit {
            return Err(CapacityRejection {
                gate: Gate::CpuUnits,
                requested: demand.cpu_milli_units,
                committed: self.cpu_milli_units,
                limit: cpu_limit,
            });
        }
        next.vcpus_by_class = census;
        next.cpu_milli_units = committed;
        let memory_committed = self.guaranteed_bytes.saturating_add(self.elastic_bytes);
        let memory_requested = demand.guaranteed_bytes.saturating_add(demand.elastic_bytes);
        gate(
            Gate::GuaranteedMemory,
            memory_committed,
            memory_requested,
            memory_limit,
        )?;
        next.guaranteed_bytes = self
            .guaranteed_bytes
            .saturating_add(demand.guaranteed_bytes);
        next.elastic_bytes = gate(
            Gate::ElasticMemory,
            self.elastic_bytes,
            demand.elastic_bytes,
            profile.memory.elastic_budget_bytes,
        )?;
        next.storage_bytes = gate(
            Gate::PrivateStorage,
            self.storage_bytes,
            demand.storage_bytes,
            profile.storage.private_budget_bytes,
        )?;
        next.network_units = gate(
            Gate::NetworkInventory,
            self.network_units,
            demand.network_units,
            u64::from(profile.network.units),
        )?;
        next.processes = gate(
            Gate::ProcessLimit,
            self.processes,
            1,
            u64::from(profile.process.processes),
        )?;
        next.descriptors = gate(
            Gate::DescriptorLimit,
            self.descriptors,
            demand.descriptors,
            u64::from(profile.process.descriptors),
        )?;
        next.residents = gate(
            Gate::OperatorSafetyLimit,
            self.residents,
            1,
            u64::from(limits.resident_instances),
        )?;
        next.launches = gate(
            Gate::ConcurrentLaunches,
            self.launches,
            1,
            u64::from(limits.concurrent_launches),
        )?;
        next.runnable_vcpus = gate(
            Gate::RunnableVcpus,
            self.runnable_vcpus,
            demand.runnable_vcpus,
            u64::from(limits.runnable_vcpus),
        )?;
        next.dirty_bytes = gate(
            Gate::DirtyMemory,
            self.dirty_bytes,
            demand.dirty_bytes,
            limits.dirty_memory_bytes,
        )?;
        gate(
            Gate::CleanupSlots,
            self.cleanups,
            0,
            u64::from(limits.cleanup_slots),
        )?;
        Ok(next)
    }

    pub(super) fn free_nodes(&self, profile: &HostProfile) -> Vec<NodeFree> {
        let nodes = u64::from(profile.cpu.numa_nodes.max(1));
        let cpu_per_node = u64::from(profile.admissible_cpu_units()) * 1000 / nodes;
        let memory_per_node = profile.admissible_memory_bytes() / nodes;
        (0..profile.cpu.numa_nodes.max(1))
            .map(|node| NodeFree {
                node: NodeId(node),
                cpu_milli_units: cpu_per_node
                    .saturating_sub(self.node_cpu.get(node as usize).copied().unwrap_or(0)),
                memory_bytes: memory_per_node
                    .saturating_sub(self.node_memory.get(node as usize).copied().unwrap_or(0)),
            })
            .collect()
    }

    pub(super) fn subtract(&mut self, reservation: &Reservation, profile: &HostProfile) {
        let demand = &reservation.demand;
        let index = demand.workload.index();
        let census = &mut self.vcpus_by_class[index];
        *census = census.saturating_sub(demand.vcpus);
        self.cpu_milli_units = census_milli_units(&self.vcpus_by_class, profile).unwrap_or(0);
        self.guaranteed_bytes = self
            .guaranteed_bytes
            .saturating_sub(demand.guaranteed_bytes);
        self.elastic_bytes = self.elastic_bytes.saturating_sub(demand.elastic_bytes);
        self.storage_bytes = self.storage_bytes.saturating_sub(demand.storage_bytes);
        self.network_units = self.network_units.saturating_sub(demand.network_units);
        self.processes = self.processes.saturating_sub(1);
        self.descriptors = self.descriptors.saturating_sub(demand.descriptors);
        self.residents = self.residents.saturating_sub(1);
        if reservation.launching {
            self.launches = self.launches.saturating_sub(1);
        }
        self.runnable_vcpus = self.runnable_vcpus.saturating_sub(demand.runnable_vcpus);
        self.dirty_bytes = self.dirty_bytes.saturating_sub(demand.dirty_bytes);
        let node = reservation.node.0 as usize;
        if let Some(census) = self.node_vcpus.get_mut(node) {
            census[index] = census[index].saturating_sub(demand.vcpus);
            let recomputed = census_milli_units(census, profile).unwrap_or(0);
            if let Some(cpu) = self.node_cpu.get_mut(node) {
                *cpu = recomputed;
            }
        }
        if let Some(memory) = self.node_memory.get_mut(node) {
            *memory =
                memory.saturating_sub(demand.guaranteed_bytes.saturating_add(demand.elastic_bytes));
        }
    }
}
