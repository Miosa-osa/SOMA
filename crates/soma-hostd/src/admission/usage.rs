//! Demand computation with checked arithmetic and the committed usage every gate is checked
//! against.

use crate::{CapacityRejection, Gate, HostProfile, InstanceShape, MemoryClass, NodeFree, NodeId};

use super::reserve::Reservation;

/// What one shape consumes, in the units of every gate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Demand {
    /// CPU milli-units after the class ratio.
    pub cpu_milli_units: u64,
    /// Guaranteed bytes including overhead.
    pub guaranteed_bytes: u64,
    /// Elastic bytes including overhead.
    pub elastic_bytes: u64,
    /// Private storage bytes.
    pub storage_bytes: u64,
    /// Network units.
    pub network_units: u64,
    /// Descriptors.
    pub descriptors: u64,
    /// vCPUs counted as runnable.
    pub runnable_vcpus: u64,
    /// Worst-case private dirty bytes.
    pub dirty_bytes: u64,
}

impl Demand {
    /// Computes the demand of `shape` on `profile`.
    ///
    /// # Errors
    ///
    /// Returns [`Gate::Arithmetic`] on overflow.
    pub fn of(profile: &HostProfile, shape: &InstanceShape) -> Result<Self, CapacityRejection> {
        let overflow = |requested| CapacityRejection {
            gate: Gate::Arithmetic,
            requested,
            committed: 0,
            limit: u64::MAX,
        };
        let ratio = profile.overcommit.ratio(shape.workload);
        let cpu_milli_units = u64::from(shape.vcpus)
            .checked_mul(1000)
            .and_then(|units| units.checked_mul(u64::from(ratio.threads)))
            .map(|units| units.div_ceil(u64::from(ratio.vcpus)))
            .ok_or_else(|| overflow(u64::from(shape.vcpus)))?;
        let overhead = profile.memory.overhead.bytes_per_instance;
        let with_overhead = |bytes: u64| bytes.checked_add(overhead).ok_or_else(|| overflow(bytes));
        let (guaranteed_bytes, elastic_bytes, dirty_bytes) = match shape.memory_class {
            MemoryClass::Guaranteed => (
                with_overhead(shape.guest_memory_bytes)?,
                0,
                shape.guest_memory_bytes,
            ),
            MemoryClass::Elastic {
                expected_resident_bytes,
            } => (
                0,
                with_overhead(expected_resident_bytes)?,
                expected_resident_bytes,
            ),
        };
        Ok(Self {
            cpu_milli_units,
            guaranteed_bytes,
            elastic_bytes,
            storage_bytes: shape.private_storage_bytes,
            network_units: u64::from(shape.network_units),
            descriptors: u64::from(shape.descriptors),
            runnable_vcpus: u64::from(shape.vcpus),
            dirty_bytes,
        })
    }
}

/// Committed usage across every gate.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    /// CPU milli-units.
    pub cpu_milli_units: u64,
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
    /// Per-node CPU milli-units.
    pub node_cpu: Vec<u64>,
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
        next.cpu_milli_units = gate(
            Gate::CpuUnits,
            self.cpu_milli_units,
            demand.cpu_milli_units,
            cpu_limit,
        )?;
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

    pub(super) fn subtract(&mut self, reservation: &Reservation) {
        let demand = &reservation.demand;
        self.cpu_milli_units = self.cpu_milli_units.saturating_sub(demand.cpu_milli_units);
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
        if let Some(cpu) = self.node_cpu.get_mut(node) {
            *cpu = cpu.saturating_sub(demand.cpu_milli_units);
        }
        if let Some(memory) = self.node_memory.get_mut(node) {
            *memory =
                memory.saturating_sub(demand.guaranteed_bytes.saturating_add(demand.elastic_bytes));
        }
    }
}
