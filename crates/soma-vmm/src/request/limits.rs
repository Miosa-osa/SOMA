use std::num::{NonZeroU32, NonZeroU64};

use super::CommandError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeoutMillis(NonZeroU32);

impl TimeoutMillis {
    /// Creates a bounded execution timeout.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the timeout is zero or greater than one hour.
    pub fn new(value: u32) -> Result<Self, CommandError> {
        let value = NonZeroU32::new(value).ok_or(CommandError::Zero("timeout milliseconds"))?;
        if value.get() > 3_600_000 {
            return Err(CommandError::TooLarge("timeout milliseconds"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// Maximum combined stdout and stderr retained for one Execute operation.
///
/// If an adapter exceeds this limit, SOMA retains stdout first, uses any remaining allowance for
/// stderr, and reports [`ExitStatus::OutputLimit`](crate::ExitStatus::OutputLimit).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputBytes(NonZeroU64);

impl OutputBytes {
    /// Creates a bounded output allowance.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the allowance is zero or greater than 16 MiB.
    pub fn new(value: u64) -> Result<Self, CommandError> {
        let value = NonZeroU64::new(value).ok_or(CommandError::Zero("output bytes"))?;
        if value.get() > 16 * 1024 * 1024 {
            return Err(CommandError::TooLarge("output bytes"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionLimits {
    timeout: TimeoutMillis,
    output: OutputBytes,
}

impl ExecutionLimits {
    #[must_use]
    pub const fn new(timeout: TimeoutMillis, output: OutputBytes) -> Self {
        Self { timeout, output }
    }

    #[must_use]
    pub const fn timeout(self) -> TimeoutMillis {
        self.timeout
    }

    #[must_use]
    pub const fn output(self) -> OutputBytes {
        self.output
    }
}
