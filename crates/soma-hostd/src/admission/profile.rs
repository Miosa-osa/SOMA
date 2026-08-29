//! The certified host profile: what the host has, what it reserves for itself, the measured
//! per-VM overhead, the per-dimension limits, and the per-class CPU overcommit policy.

use std::fmt;

use sha2::{Digest, Sha256};

use crate::{HostProfileDigest, WorkloadClass};

/// The measured host-side memory cost of one VM beyond its guest memory.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MeasuredOverhead {
    /// Bytes per resident Instance.
    pub bytes_per_instance: u64,
    /// Where the number comes from; a placeholder must say so.
    pub evidence: &'static str,
}

impl MeasuredOverhead {
    /// The 64 MiB explanatory placeholder from the visual atlas; not a measurement.
    pub const ATLAS_PLACEHOLDER: Self = Self {
        bytes_per_instance: 64 << 20,
        evidence: "docs/architecture/visual-atlas.md section 15 placeholder; not measured",
    };

    /// The single-sample non-guest resident total of the `x86_64` PVH boot proof, rounded up
    /// to 4 MiB; a debug-build diagnostic, not a certified per-VM figure.
    pub const PVH_BOOT_SINGLE_SAMPLE: Self = Self {
        bytes_per_instance: 4 << 20,
        evidence: "docs/evidence/2026-08-29-x86_64-pvh-kernel-boot.md single-sample debug-build non-guest resident total of about 3.6 MiB; not certified",
    };
}

/// Hardware threads and NUMA topology.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CpuInventory {
    /// Schedulable hardware threads.
    pub hardware_threads: u32,
    /// Threads reserved for the host.
    pub reserved_threads: u32,
    /// NUMA nodes.
    pub numa_nodes: u32,
}

/// Physical memory and its reserve.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MemoryInventory {
    /// Total bytes.
    pub total_bytes: u64,
    /// Bytes reserved for the host.
    pub reserved_bytes: u64,
    /// Measured per-VM overhead.
    pub overhead: MeasuredOverhead,
    /// The share of admissible memory the elastic class may use.
    pub elastic_budget_bytes: u64,
}

/// Private writable storage budget.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StorageInventory {
    /// Bytes reserved for private heads after the emergency reserve.
    pub private_budget_bytes: u64,
}

/// Network inventory units such as addresses, policy objects, and bandwidth classes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NetworkInventory {
    /// Units available.
    pub units: u32,
}

/// Process and descriptor limits.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProcessInventory {
    /// VMM processes.
    pub processes: u32,
    /// Descriptors.
    pub descriptors: u32,
}

/// Operator-certified limits that are separate from raw inventory.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OperatorLimits {
    /// Resident Instances.
    pub resident_instances: u32,
    /// Concurrent Launch operations.
    pub concurrent_launches: u32,
    /// Runnable vCPUs.
    pub runnable_vcpus: u32,
    /// Private dirty memory.
    pub dirty_memory_bytes: u64,
    /// Concurrent cleanups.
    pub cleanup_slots: u32,
}

/// vCPUs admitted per hardware thread unit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Ratio {
    /// vCPUs.
    pub vcpus: u32,
    /// Thread units.
    pub threads: u32,
}

impl Ratio {
    /// Strict 1:1.
    pub const STRICT: Self = Self {
        vcpus: 1,
        threads: 1,
    };
}

/// The certified overcommit ratio per workload class.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OvercommitPolicy {
    /// API-waiting agents.
    pub api_waiting: Ratio,
    /// Build agents.
    pub build: Ratio,
    /// Idle interactive sessions.
    pub idle_interactive: Ratio,
}

impl OvercommitPolicy {
    /// Strict 1:1 for every class.
    pub const STRICT: Self = Self {
        api_waiting: Ratio::STRICT,
        build: Ratio::STRICT,
        idle_interactive: Ratio::STRICT,
    };

    /// The ratio of one class.
    #[must_use]
    pub const fn ratio(&self, class: WorkloadClass) -> Ratio {
        match class {
            WorkloadClass::ApiWaiting => self.api_waiting,
            WorkloadClass::Build => self.build,
            WorkloadClass::IdleInteractive => self.idle_interactive,
        }
    }
}

/// Why a profile is not usable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileError {
    /// The thread reserve leaves no admissible unit.
    NoAdmissibleCpu,
    /// The memory reserve leaves no admissible byte.
    NoAdmissibleMemory,
    /// Zero NUMA nodes.
    NoNumaNode,
    /// A ratio has a zero side.
    ZeroRatio,
    /// The elastic budget exceeds admissible memory.
    ElasticBudget,
    /// The per-VM overhead is zero.
    ZeroOverhead,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NoAdmissibleCpu => "reserve leaves no admissible CPU unit",
            Self::NoAdmissibleMemory => "reserve leaves no admissible memory",
            Self::NoNumaNode => "profile has no NUMA node",
            Self::ZeroRatio => "an overcommit ratio has a zero side",
            Self::ElasticBudget => "elastic budget exceeds admissible memory",
            Self::ZeroOverhead => "per-VM overhead is zero",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for ProfileError {}

/// The complete certified host profile.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HostProfile {
    /// CPU.
    pub cpu: CpuInventory,
    /// Memory.
    pub memory: MemoryInventory,
    /// Storage.
    pub storage: StorageInventory,
    /// Network.
    pub network: NetworkInventory,
    /// Processes.
    pub process: ProcessInventory,
    /// Operator limits.
    pub limits: OperatorLimits,
    /// Overcommit policy.
    pub overcommit: OvercommitPolicy,
}

impl HostProfile {
    /// Validates the profile.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    pub const fn validate(self) -> Result<Self, ProfileError> {
        if self.cpu.hardware_threads <= self.cpu.reserved_threads {
            return Err(ProfileError::NoAdmissibleCpu);
        }
        if self.memory.total_bytes <= self.memory.reserved_bytes {
            return Err(ProfileError::NoAdmissibleMemory);
        }
        if self.cpu.numa_nodes == 0 {
            return Err(ProfileError::NoNumaNode);
        }
        let ratios = [
            self.overcommit.api_waiting,
            self.overcommit.build,
            self.overcommit.idle_interactive,
        ];
        let mut index = 0;
        while index < ratios.len() {
            if ratios[index].vcpus == 0 || ratios[index].threads == 0 {
                return Err(ProfileError::ZeroRatio);
            }
            index += 1;
        }
        if self.memory.elastic_budget_bytes > self.admissible_memory_bytes() {
            return Err(ProfileError::ElasticBudget);
        }
        if self.memory.overhead.bytes_per_instance == 0 {
            return Err(ProfileError::ZeroOverhead);
        }
        Ok(self)
    }

    /// Hardware thread units after the host reserve.
    #[must_use]
    pub const fn admissible_cpu_units(&self) -> u32 {
        self.cpu
            .hardware_threads
            .saturating_sub(self.cpu.reserved_threads)
    }

    /// Bytes after the host reserve.
    #[must_use]
    pub const fn admissible_memory_bytes(&self) -> u64 {
        self.memory
            .total_bytes
            .saturating_sub(self.memory.reserved_bytes)
    }

    /// Digests every numeric dimension and the overhead evidence label.
    #[must_use]
    pub fn digest(&self) -> HostProfileDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"SOMAHOSTPROFILE");
        for value in [
            u64::from(self.cpu.hardware_threads),
            u64::from(self.cpu.reserved_threads),
            u64::from(self.cpu.numa_nodes),
            self.memory.total_bytes,
            self.memory.reserved_bytes,
            self.memory.overhead.bytes_per_instance,
            self.memory.elastic_budget_bytes,
            self.storage.private_budget_bytes,
            u64::from(self.network.units),
            u64::from(self.process.processes),
            u64::from(self.process.descriptors),
            u64::from(self.limits.resident_instances),
            u64::from(self.limits.concurrent_launches),
            u64::from(self.limits.runnable_vcpus),
            self.limits.dirty_memory_bytes,
            u64::from(self.limits.cleanup_slots),
        ] {
            hasher.update(value.to_be_bytes());
        }
        for class in WorkloadClass::ALL {
            let ratio = self.overcommit.ratio(class);
            hasher.update([class.code()]);
            hasher.update(ratio.vcpus.to_be_bytes());
            hasher.update(ratio.threads.to_be_bytes());
        }
        hasher.update(self.memory.overhead.evidence.as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        HostProfileDigest::new(digest).unwrap_or_else(|_| unreachable!("SHA-256 output is nonzero"))
    }
}
