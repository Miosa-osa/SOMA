use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use super::ValidationError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExecutionLimits {
    timeout_ms: u64,
    max_output_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionLimitsWire {
    timeout_ms: u64,
    max_output_bytes: u64,
}

impl<'de> Deserialize<'de> for ExecutionLimits {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ExecutionLimitsWire::deserialize(deserializer)?;
        Self::new(wire.timeout_ms, wire.max_output_bytes).map_err(D::Error::custom)
    }
}

impl ExecutionLimits {
    pub const MIN_TIMEOUT_MS: u64 = 1;
    pub const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1_000;
    pub const MIN_OUTPUT_BYTES: u64 = 1;
    pub const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
    pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
    pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

    /// Creates bounded execution limits for one direct command.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidLimits`] when timeout or combined output allowance is
    /// outside the published portable range.
    pub fn new(timeout_ms: u64, max_output_bytes: u64) -> Result<Self, ValidationError> {
        if !(Self::MIN_TIMEOUT_MS..=Self::MAX_TIMEOUT_MS).contains(&timeout_ms)
            || !(Self::MIN_OUTPUT_BYTES..=Self::MAX_OUTPUT_BYTES).contains(&max_output_bytes)
        {
            return Err(ValidationError::InvalidLimits);
        }
        Ok(Self {
            timeout_ms,
            max_output_bytes,
        })
    }

    #[must_use]
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    #[must_use]
    pub fn max_output_bytes(&self) -> u64 {
        self.max_output_bytes
    }
}
