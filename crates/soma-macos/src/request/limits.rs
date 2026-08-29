use serde::Serialize;

use super::{RequestError, RequestErrorReason};

const MEBIBYTE: u64 = 1_048_576;
pub(crate) const MAX_OUTPUT_BYTES: u64 = 16 * MEBIBYTE;
const MAX_TIMEOUT_MILLIS: u64 = 86_400_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MachineShape {
    vcpus: u16,
    memory_bytes: u64,
}

impl MachineShape {
    /// Creates a shape accepted by the Apple runtime.
    ///
    /// # Errors
    ///
    /// Returns an error when vCPU or memory is zero, or memory is not an exact MiB multiple.
    pub fn new(vcpus: u16, memory_bytes: u64) -> Result<Self, RequestError> {
        if vcpus == 0 {
            return Err(RequestError::new("vcpus", RequestErrorReason::Zero));
        }
        if memory_bytes == 0 {
            return Err(RequestError::new("memory_bytes", RequestErrorReason::Zero));
        }
        if !memory_bytes.is_multiple_of(MEBIBYTE) {
            return Err(RequestError::new(
                "memory_bytes",
                RequestErrorReason::NotMebibyteAligned,
            ));
        }
        Ok(Self {
            vcpus,
            memory_bytes,
        })
    }

    #[must_use]
    pub const fn vcpus(self) -> u16 {
        self.vcpus
    }

    #[must_use]
    pub const fn memory_bytes(self) -> u64 {
        self.memory_bytes
    }

    #[must_use]
    pub(crate) const fn memory_mebibytes(self) -> u64 {
        self.memory_bytes / MEBIBYTE
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionLimits {
    timeout_millis: u64,
    output_bytes: u64,
}

impl ExecutionLimits {
    /// Creates bounded execution limits.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or implementation-exceeding limits.
    pub fn new(timeout_millis: u64, output_bytes: u64) -> Result<Self, RequestError> {
        validate_timeout(timeout_millis)?;
        if output_bytes == 0 {
            return Err(RequestError::new("output_bytes", RequestErrorReason::Zero));
        }
        if output_bytes > MAX_OUTPUT_BYTES {
            return Err(RequestError::new(
                "output_bytes",
                RequestErrorReason::TooLarge,
            ));
        }
        Ok(Self {
            timeout_millis,
            output_bytes,
        })
    }

    #[must_use]
    pub const fn timeout_millis(self) -> u64 {
        self.timeout_millis
    }

    #[must_use]
    pub const fn output_bytes(self) -> u64 {
        self.output_bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ControlLimits {
    timeout_millis: u64,
    output_bytes: u64,
}

impl ControlLimits {
    /// Creates bounded limits for a lifecycle control command.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or implementation-exceeding limits.
    pub fn new(timeout_millis: u64, output_bytes: u64) -> Result<Self, RequestError> {
        ExecutionLimits::new(timeout_millis, output_bytes)?;
        Ok(Self {
            timeout_millis,
            output_bytes,
        })
    }

    #[must_use]
    pub const fn timeout_millis(self) -> u64 {
        self.timeout_millis
    }

    #[must_use]
    pub const fn output_bytes(self) -> u64 {
        self.output_bytes
    }
}

fn validate_timeout(timeout_millis: u64) -> Result<(), RequestError> {
    if timeout_millis == 0 {
        return Err(RequestError::new(
            "timeout_millis",
            RequestErrorReason::Zero,
        ));
    }
    if timeout_millis > MAX_TIMEOUT_MILLIS {
        return Err(RequestError::new(
            "timeout_millis",
            RequestErrorReason::TooLarge,
        ));
    }
    Ok(())
}
