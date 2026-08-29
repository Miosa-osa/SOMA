use std::fmt;

use super::ValidationError;

#[derive(Clone, PartialEq, Eq)]
pub struct DirectCommand {
    executable: String,
    arguments: Vec<String>,
}

impl DirectCommand {
    pub const MAX_EXECUTABLE_BYTES: usize = 4_096;
    pub const MAX_ARGUMENTS: usize = 4_096;
    pub const MAX_ARGUMENT_BYTES: usize = 128 * 1024;
    pub const MAX_AGGREGATE_BYTES: usize = 1024 * 1024;

    /// Creates one bounded direct executable invocation without a shell.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidCommand`] for a relative executable, embedded NUL,
    /// excessive argument count, or any per-field or aggregate size violation.
    pub fn new<I, S>(executable: impl Into<String>, arguments: I) -> Result<Self, ValidationError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let executable = executable.into();
        let arguments: Vec<String> = arguments.into_iter().map(Into::into).collect();
        let total_bytes = executable.len()
            + arguments
                .iter()
                .map(String::len)
                .try_fold(0_usize, usize::checked_add)
                .ok_or(ValidationError::InvalidCommand)?;
        if !executable.starts_with('/')
            || executable.contains('\0')
            || executable.len() > Self::MAX_EXECUTABLE_BYTES
            || arguments.len() > Self::MAX_ARGUMENTS
            || arguments
                .iter()
                .any(|value| value.len() > Self::MAX_ARGUMENT_BYTES || value.contains('\0'))
            || total_bytes > Self::MAX_AGGREGATE_BYTES
        {
            return Err(ValidationError::InvalidCommand);
        }
        Ok(Self {
            executable,
            arguments,
        })
    }

    #[must_use]
    pub fn executable(&self) -> &str {
        &self.executable
    }

    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
}

impl fmt::Debug for DirectCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectCommand")
            .field("content", &"[REDACTED]")
            .finish()
    }
}
