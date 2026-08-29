//! The certified host inventory, profile, and Machine shape the daemon admits against.
//!
//! Every inventory dimension is an explicit operator input; the daemon never guesses a
//! measurement, and the per-VM overhead carries the label that says where it came from.

use soma_hostd::{
    CpuInventory, HostProfile, InstanceShape, MeasuredOverhead, MemoryClass, MemoryInventory,
    NetworkInventory, OperatorLimits, OvercommitPolicy, ProcessInventory, StorageInventory,
    WorkloadClass,
};

use super::Config;

/// Host descriptors one worker of this daemon's shape needs.
const SHAPE_DESCRIPTORS: u32 = 16;

/// Where the per-VM overhead of this daemon's profile comes from.
const OVERHEAD_EVIDENCE: &str =
    "operator supplied --host-overhead-bytes; not certified by this crate";

/// The certified host inventory the daemon admits capacity against.
pub(super) struct Host {
    pub(super) threads: Option<u32>,
    pub(super) reserved_threads: u32,
    pub(super) memory: Option<u64>,
    pub(super) reserved_memory: u64,
    pub(super) overhead: Option<u64>,
    pub(super) storage: Option<u64>,
    pub(super) network_units: u32,
    pub(super) processes: u32,
    pub(super) descriptors: u32,
    pub(super) residents: u32,
    pub(super) launches: u32,
    pub(super) runnable_vcpus: u32,
    pub(super) dirty_memory: u64,
    pub(super) cleanup_slots: u32,
}

pub(super) fn profile(host: &Host) -> Result<HostProfile, String> {
    HostProfile {
        cpu: CpuInventory {
            hardware_threads: host.threads.ok_or("--host-threads is required")?,
            reserved_threads: host.reserved_threads,
            numa_nodes: 1,
        },
        memory: MemoryInventory {
            total_bytes: host.memory.ok_or("--host-memory-bytes is required")?,
            reserved_bytes: host.reserved_memory,
            overhead: MeasuredOverhead {
                bytes_per_instance: host.overhead.ok_or("--host-overhead-bytes is required")?,
                evidence: OVERHEAD_EVIDENCE,
            },
            elastic_budget_bytes: 0,
        },
        storage: StorageInventory {
            private_budget_bytes: host.storage.ok_or("--host-storage-bytes is required")?,
        },
        network: NetworkInventory {
            units: host.network_units,
        },
        process: ProcessInventory {
            processes: host.processes,
            descriptors: host.descriptors,
        },
        limits: OperatorLimits {
            resident_instances: host.residents,
            concurrent_launches: host.launches,
            runnable_vcpus: host.runnable_vcpus,
            dirty_memory_bytes: host.dirty_memory,
            cleanup_slots: host.cleanup_slots,
        },
        overcommit: OvercommitPolicy::STRICT,
    }
    .validate()
    .map_err(|error| error.to_string())
}

pub(super) fn shape(config: &Config) -> Result<InstanceShape, String> {
    InstanceShape {
        vcpus: config.vcpus,
        guest_memory_bytes: config.memory,
        memory_class: MemoryClass::Guaranteed,
        private_storage_bytes: config.logical_bytes,
        workload: WorkloadClass::ApiWaiting,
        network_units: 1,
        descriptors: SHAPE_DESCRIPTORS,
    }
    .validate()
    .map_err(|error| error.to_string())
}
