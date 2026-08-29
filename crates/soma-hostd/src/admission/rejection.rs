//! The typed capacity rejection that names the exact gate and the numbers behind it.

use std::fmt;

/// One admission gate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Gate {
    /// Effective CPU units under the class overcommit ratio.
    CpuUnits,
    /// Guaranteed memory including measured per-VM overhead.
    GuaranteedMemory,
    /// The elastic memory budget.
    ElasticMemory,
    /// Private writable storage reserve.
    PrivateStorage,
    /// Network inventory units.
    NetworkInventory,
    /// VMM process limit.
    ProcessLimit,
    /// Descriptor limit.
    DescriptorLimit,
    /// The operator's resident-Instance safety limit.
    OperatorSafetyLimit,
    /// Concurrent Launch operations.
    ConcurrentLaunches,
    /// Runnable vCPUs.
    RunnableVcpus,
    /// Private dirty memory.
    DirtyMemory,
    /// Concurrent cleanup work.
    CleanupSlots,
    /// No single NUMA node fits both CPU and memory.
    NumaFit,
    /// Checked arithmetic overflowed.
    Arithmetic,
}

/// A rejection with capacity evidence; every partial reservation was rolled back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityRejection {
    /// The gate that refused.
    pub gate: Gate,
    /// What the shape needed in the gate's unit.
    pub requested: u64,
    /// What was already committed.
    pub committed: u64,
    /// The gate's limit.
    pub limit: u64,
}

impl CapacityRejection {
    /// What remained under the limit.
    #[must_use]
    pub const fn available(&self) -> u64 {
        self.limit.saturating_sub(self.committed)
    }
}

impl fmt::Display for CapacityRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?}: requested {}, committed {} of {}, available {}",
            self.gate,
            self.requested,
            self.committed,
            self.limit,
            self.available()
        )
    }
}

impl std::error::Error for CapacityRejection {}
