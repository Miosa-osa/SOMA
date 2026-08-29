//! The requested Machine shape, its workload class, and the explicit memory class.

use std::fmt;

/// The workload pattern an operator certified an overcommit policy for.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[repr(u8)]
pub enum WorkloadClass {
    /// Agents that mostly wait on remote APIs; high CPU overcommit can work.
    ApiWaiting = 1,
    /// Build agents that are CPU, memory, and storage heavy; low overcommit is safer.
    Build = 2,
    /// Interactive sessions that idle and wake together.
    IdleInteractive = 3,
}

impl WorkloadClass {
    /// Every class in encoding order.
    pub const ALL: [Self; 3] = [Self::ApiWaiting, Self::Build, Self::IdleInteractive];

    /// Returns the stable encoding.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The index of this class in a per-class census.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize - 1
    }

    /// Decodes one class.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::ApiWaiting),
            2 => Some(Self::Build),
            3 => Some(Self::IdleInteractive),
            _ => None,
        }
    }
}

/// How the guest memory promise is admitted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub enum MemoryClass {
    /// The complete guest memory plus measured overhead is reserved for the worst case.
    Guaranteed,
    /// Admitted against an expected resident set with weaker guarantees and pressure limits.
    Elastic {
        /// The resident bytes the operator expects this Instance to dirty.
        expected_resident_bytes: u64,
    },
}

impl MemoryClass {
    /// Returns the stable encoding of the class kind.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Guaranteed => 1,
            Self::Elastic { .. } => 2,
        }
    }
}

/// One requested Machine shape.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InstanceShape {
    /// Virtual processors visible to the guest.
    pub vcpus: u32,
    /// Guest memory in bytes.
    pub guest_memory_bytes: u64,
    /// The memory admission class.
    pub memory_class: MemoryClass,
    /// Private writable storage reserved in bytes.
    pub private_storage_bytes: u64,
    /// The certified workload class.
    pub workload: WorkloadClass,
    /// Network inventory units such as addresses and policy objects.
    pub network_units: u32,
    /// Host descriptors the worker needs.
    pub descriptors: u32,
}

/// Why a shape is not admissible on any host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapeError {
    /// Zero vCPUs.
    NoVcpus,
    /// Zero guest memory.
    NoMemory,
    /// An elastic expectation above the guest memory promise.
    ElasticAboveGuest,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoVcpus => formatter.write_str("shape has no vCPU"),
            Self::NoMemory => formatter.write_str("shape has no guest memory"),
            Self::ElasticAboveGuest => {
                formatter.write_str("elastic expectation exceeds guest memory")
            }
        }
    }
}

impl std::error::Error for ShapeError {}

/// An [`InstanceShape`] that passed [`InstanceShape::validate`].
///
/// Every admission and every estimate takes one, so a shape with no vCPU, no memory, or an
/// elastic expectation above its guest promise can never be accounted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ValidShape(InstanceShape);

impl ValidShape {
    /// The validated shape.
    #[must_use]
    pub const fn shape(&self) -> &InstanceShape {
        &self.0
    }

    /// Takes the validated shape back out.
    #[must_use]
    pub const fn into_shape(self) -> InstanceShape {
        self.0
    }
}

impl InstanceShape {
    /// Validates the shape.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    pub const fn validate(self) -> Result<ValidShape, ShapeError> {
        if self.vcpus == 0 {
            return Err(ShapeError::NoVcpus);
        }
        if self.guest_memory_bytes == 0 {
            return Err(ShapeError::NoMemory);
        }
        if let MemoryClass::Elastic {
            expected_resident_bytes,
        } = self.memory_class
            && expected_resident_bytes > self.guest_memory_bytes
        {
            return Err(ShapeError::ElasticAboveGuest);
        }
        Ok(ValidShape(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn shape() -> InstanceShape {
        InstanceShape {
            vcpus: 1,
            guest_memory_bytes: 512 << 20,
            memory_class: MemoryClass::Guaranteed,
            private_storage_bytes: 4 << 30,
            workload: WorkloadClass::ApiWaiting,
            network_units: 1,
            descriptors: 16,
        }
    }

    #[test]
    fn shapes_reject_empty_dimensions_and_over_promised_elastic_sets() {
        assert_eq!(shape().validate().map(ValidShape::into_shape), Ok(shape()));
        assert_eq!(
            InstanceShape {
                vcpus: 0,
                ..shape()
            }
            .validate(),
            Err(ShapeError::NoVcpus)
        );
        assert_eq!(
            InstanceShape {
                guest_memory_bytes: 0,
                ..shape()
            }
            .validate(),
            Err(ShapeError::NoMemory)
        );
        assert_eq!(
            InstanceShape {
                memory_class: MemoryClass::Elastic {
                    expected_resident_bytes: 1 << 30
                },
                ..shape()
            }
            .validate(),
            Err(ShapeError::ElasticAboveGuest)
        );
        for class in WorkloadClass::ALL {
            assert_eq!(WorkloadClass::from_code(class.code()), Some(class));
        }
        assert_eq!(WorkloadClass::from_code(9), None);
    }
}
