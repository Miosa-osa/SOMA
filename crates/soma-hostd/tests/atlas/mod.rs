//! The explanatory host profiles and Machine shape of the visual atlas capacity ladder.

#![allow(dead_code)]

use soma_hostd::{
    CpuInventory, HostProfile, InstanceShape, MeasuredOverhead, MemoryClass, MemoryInventory,
    NetworkInventory, OperatorLimits, OvercommitPolicy, ProcessInventory, Ratio, StorageInventory,
    WorkloadClass,
};

pub const GIB: u64 = 1 << 30;
pub const MIB: u64 = 1 << 20;

pub fn atlas_profile(
    threads: u32,
    reserved: u32,
    total_gib: u64,
    reserved_gib: u64,
) -> HostProfile {
    HostProfile {
        cpu: CpuInventory {
            hardware_threads: threads,
            reserved_threads: reserved,
            numa_nodes: 1,
        },
        memory: MemoryInventory {
            total_bytes: total_gib * GIB,
            reserved_bytes: reserved_gib * GIB,
            overhead: MeasuredOverhead::ATLAS_PLACEHOLDER,
            elastic_budget_bytes: (total_gib - reserved_gib) * GIB / 4,
        },
        storage: StorageInventory {
            private_budget_bytes: 1800 * GIB,
        },
        network: NetworkInventory { units: 4096 },
        process: ProcessInventory {
            processes: 4096,
            descriptors: 1 << 20,
        },
        limits: OperatorLimits {
            resident_instances: 100_000,
            concurrent_launches: 100_000,
            runnable_vcpus: 100_000,
            dirty_memory_bytes: u64::MAX / 2,
            cleanup_slots: 64,
        },
        overcommit: OvercommitPolicy {
            api_waiting: Ratio {
                vcpus: 4,
                threads: 1,
            },
            build: Ratio::STRICT,
            idle_interactive: Ratio::STRICT,
        },
    }
    .validate()
    .expect("profile")
}

pub fn atlas_shape() -> InstanceShape {
    InstanceShape {
        vcpus: 1,
        guest_memory_bytes: 512 * MIB,
        memory_class: MemoryClass::Guaranteed,
        private_storage_bytes: 0,
        workload: WorkloadClass::ApiWaiting,
        network_units: 1,
        descriptors: 16,
    }
    .validate()
    .expect("shape")
}
