use serde::{Deserialize, Serialize};

use crate::{MachineShape, ValidationError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationUnavailable {
    NotReached,
    NotVerified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
pub enum Observation<T> {
    Observed(T),
    Unavailable(ObservationUnavailable),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EffectiveShape {
    vcpu_count: Observation<u16>,
    memory_mib: Observation<u64>,
    storage_mib: Observation<u64>,
}

impl EffectiveShape {
    /// Creates per-dimension effective shape evidence.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidShape`] when an observed vCPU count or memory size is
    /// zero; observed storage of zero is a sandbox with no writable disk.
    pub fn new(
        vcpu_count: Observation<u16>,
        memory_mib: Observation<u64>,
        storage_mib: Observation<u64>,
    ) -> Result<Self, ValidationError> {
        // Zero observed storage is a real backend answer: the sandbox was given no writable
        // disk, which is what a request for none asks for.
        if matches!(vcpu_count, Observation::Observed(0))
            || matches!(memory_mib, Observation::Observed(0))
        {
            return Err(ValidationError::InvalidShape);
        }
        Ok(Self {
            vcpu_count,
            memory_mib,
            storage_mib,
        })
    }

    #[must_use]
    pub fn fully_observed(shape: &MachineShape) -> Self {
        Self {
            vcpu_count: Observation::Observed(shape.vcpu_count()),
            memory_mib: Observation::Observed(shape.memory_mib()),
            storage_mib: Observation::Observed(shape.storage_mib()),
        }
    }

    #[must_use]
    pub fn unavailable(reason: ObservationUnavailable) -> Self {
        Self {
            vcpu_count: Observation::Unavailable(reason),
            memory_mib: Observation::Unavailable(reason),
            storage_mib: Observation::Unavailable(reason),
        }
    }

    #[must_use]
    pub const fn vcpu_count(&self) -> &Observation<u16> {
        &self.vcpu_count
    }

    #[must_use]
    pub const fn memory_mib(&self) -> &Observation<u64> {
        &self.memory_mib
    }

    #[must_use]
    pub const fn storage_mib(&self) -> &Observation<u64> {
        &self.storage_mib
    }

    pub(crate) fn matches_request(&self, requested: &MachineShape) -> bool {
        observation_matches(&self.vcpu_count, &requested.vcpu_count())
            && observation_matches(&self.memory_mib, &requested.memory_mib())
            && observation_matches(&self.storage_mib, &requested.storage_mib())
    }

    pub(crate) fn all_unavailable(&self) -> bool {
        matches!(self.vcpu_count, Observation::Unavailable(_))
            && matches!(self.memory_mib, Observation::Unavailable(_))
            && matches!(self.storage_mib, Observation::Unavailable(_))
    }
}

fn observation_matches<T: PartialEq>(observation: &Observation<T>, requested: &T) -> bool {
    match observation {
        Observation::Observed(value) => value == requested,
        Observation::Unavailable(_) => true,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectiveShapeWire {
    vcpu_count: Observation<u16>,
    memory_mib: Observation<u64>,
    storage_mib: Observation<u64>,
}

impl<'de> Deserialize<'de> for EffectiveShape {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;

        let wire = EffectiveShapeWire::deserialize(deserializer)?;
        Self::new(wire.vcpu_count, wire.memory_mib, wire.storage_mib).map_err(D::Error::custom)
    }
}
