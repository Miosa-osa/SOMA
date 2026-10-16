//! One bounded direct executable invocation, and the machine contract it is bounded by.
//!
//! The four bounds below are the machine's own exec contract rather than a second opinion about
//! it. Every field of a command travels to the guest as one `u16`-prefixed length, and the whole
//! command must fit one authenticated record, so a command this type admits is one the machine
//! can be asked to run. Admitting more is not generosity: a command between this contract and a
//! looser one was refused inside the engine, after the operation had already been taken, and a
//! refusal there is indistinguishable from a lost machine.
//!
//! `crates/soma-guest/src/application/command.rs` states the same four values. The two are
//! pinned to each other by a test in `soma-api`, which depends on both.

use std::fmt;

use super::{ValidationError, command_context::CommandContext};

#[derive(Clone, PartialEq, Eq)]
pub struct DirectCommand {
    executable: String,
    arguments: Vec<String>,
    context: CommandContext,
}

impl DirectCommand {
    /// The largest executable path this facade accepts.
    pub const MAX_EXECUTABLE_BYTES: usize = 4_096;
    /// The most arguments one command may carry.
    pub const MAX_ARGUMENTS: usize = 64;
    /// The largest one argument may be.
    pub const MAX_ARGUMENT_BYTES: usize = 4_096;
    /// The largest the executable and arguments may total, counted the way the guest codec
    /// counts them: the executable's bytes plus two length bytes plus the bytes of each
    /// argument. This is one guest record's body allowance less its fixed part.
    pub const MAX_AGGREGATE_BYTES: usize = 65_459;
    /// The most one guest command body may hold: the guest record payload less the application
    /// header.
    pub const MAX_GUEST_BODY_BYTES: usize =
        Self::MAX_AGGREGATE_BYTES + Self::GUEST_FIXED_BODY_BYTES;
    /// The bytes every guest command body spends before any variable-length field.
    pub const GUEST_FIXED_BODY_BYTES: usize = 22;

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
        // Each argument spends a two byte length prefix on the wire, so it is charged for it
        // here; the aggregate bound is in the unit the guest codec actually measures.
        let total_bytes = arguments
            .iter()
            .try_fold(executable.len(), |total, value| {
                total.checked_add(2)?.checked_add(value.len())
            })
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
            context: CommandContext::default(),
        })
    }

    /// Runs the command with an environment, working directory, user, and standard input.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidCommand`] when the whole command, measured the way the
    /// guest codec measures it, no longer fits one guest record.
    pub fn with_context(mut self, context: CommandContext) -> Result<Self, ValidationError> {
        let total = Self::GUEST_FIXED_BODY_BYTES
            + self.executable.len()
            + self
                .arguments
                .iter()
                .map(|value| 2 + value.len())
                .sum::<usize>()
            + context.encoded_size();
        if total > Self::MAX_GUEST_BODY_BYTES {
            return Err(ValidationError::InvalidCommand);
        }
        self.context = context;
        Ok(self)
    }

    #[must_use]
    pub const fn context(&self) -> &CommandContext {
        &self.context
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
