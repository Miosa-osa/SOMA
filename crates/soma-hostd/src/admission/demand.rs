//! What one Machine shape consumes, in the unit of every gate, with checked arithmetic.

use super::usage::class_milli_units;
use crate::{CapacityRejection, CertifiedProfile, Gate, MemoryClass, ValidShape, WorkloadClass};

/// What one shape consumes, in the units of every gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Demand {
    /// CPU milli-units after the class ratio; evidence only, never the gate.
    pub cpu_milli_units: u64,
    /// Raw vCPUs, which the CPU gate applies the class ratio to once.
    pub vcpus: u64,
    /// The certified workload class the ratio comes from.
    pub workload: WorkloadClass,
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
    pub fn of(profile: &CertifiedProfile, shape: &ValidShape) -> Result<Self, CapacityRejection> {
        let profile = profile.profile();
        let shape = shape.shape();
        let overflow = |requested| CapacityRejection {
            gate: Gate::Arithmetic,
            requested,
            committed: 0,
            limit: u64::MAX,
        };
        let vcpus = u64::from(shape.vcpus);
        let cpu_milli_units = class_milli_units(vcpus, profile.overcommit.ratio(shape.workload))
            .ok_or_else(|| overflow(vcpus))?;
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
            vcpus,
            workload: shape.workload,
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
