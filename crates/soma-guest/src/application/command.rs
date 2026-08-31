//! One bounded, shell-free guest invocation carried by one authenticated record.
//!
//! A program and its arguments are not enough to work in a sandbox: an agent that builds a
//! project needs the token in an environment variable, the directory the build runs in, the
//! account it runs as, and the bytes a tool expects on its standard input. Those four travel
//! with the invocation they belong to rather than as separate messages, because a record that
//! set an environment and a record that ran a program could interleave with a third party's
//! record and hand one caller's secret to another caller's process.
//!
//! Everything here is bounded twice: each field has its own ceiling, and the encoded whole must
//! fit one record, so a caller cannot assemble an inadmissible message out of individually
//! admissible parts.

mod codec;
mod context;

#[cfg(test)]
mod tests;

pub use context::CommandContext;

use core::fmt;

use crate::Error;

/// Maximum number of direct arguments in one authenticated command.
pub const MAX_ARGUMENTS: usize = 64;
/// Maximum byte length of the program, one argument, one environment name or value, or the
/// working directory.
pub const MAX_FIELD_BYTES: usize = 4096;
/// Maximum number of environment variables in one authenticated command.
pub const MAX_ENVIRONMENT: usize = 64;
/// Maximum byte length of the user name a command runs as.
///
/// A user is named rather than numbered, and no system this protocol targets admits a login
/// name anywhere near this long, so the bound is generous without being a second path ceiling.
pub const MAX_USER_BYTES: usize = 256;
/// Maximum byte length of the standard input delivered with a command.
///
/// The ceiling sits below the capacity of a Linux pipe, so the agent can hand the whole of it
/// to the child in one write that cannot block on a child that never reads its input.
pub const MAX_STDIN_BYTES: usize = 32 * 1024;
/// Maximum command timeout represented by the protocol.
pub const MAX_TIMEOUT_MILLIS: u32 = 3_600_000;
/// Maximum combined output allowance represented by the protocol.
pub const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

/// Bytes every body spends before any variable-length field: the timeout, the output
/// allowance, the program length, the argument count, the environment count, the two optional
/// presence flags, and the standard-input length.
pub(super) const FIXED_BODY_SIZE: usize = 4 + 8 + 2 + 2 + 2 + 1 + 1 + 2;
/// One environment name and the value bound to it, in the form a command stores them.
pub type EnvironmentPair = (Box<[u8]>, Box<[u8]>);

/// A shell-free, bounded direct guest process invocation.
#[derive(Clone, Eq, PartialEq)]
pub struct GuestCommand {
    pub(super) program: Box<[u8]>,
    pub(super) arguments: Box<[Box<[u8]>]>,
    pub(super) timeout_millis: u32,
    pub(super) output_bytes: u64,
    pub(super) environment: Box<[EnvironmentPair]>,
    pub(super) working_directory: Option<Box<[u8]>>,
    pub(super) user: Option<Box<[u8]>>,
    pub(super) stdin: Box<[u8]>,
}

impl GuestCommand {
    /// Creates a command with an empty context: no environment, no working directory, no user,
    /// and no standard input.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidCommand`] for invalid paths, NUL, count, field, limit, or aggregate
    /// bounds.
    pub fn new(
        program: Vec<u8>,
        arguments: Vec<Vec<u8>>,
        timeout_millis: u32,
        output_bytes: u64,
    ) -> Result<Self, Error> {
        check_field(&program, false)?;
        if !program.starts_with(b"/")
            || arguments.len() > MAX_ARGUMENTS
            || timeout_millis == 0
            || timeout_millis > MAX_TIMEOUT_MILLIS
            || output_bytes == 0
            || output_bytes > MAX_OUTPUT_BYTES
        {
            return Err(Error::InvalidCommand);
        }
        for argument in &arguments {
            check_field(argument, true)?;
        }
        let command = Self {
            program: program.into_boxed_slice(),
            arguments: arguments.into_iter().map(Vec::into_boxed_slice).collect(),
            timeout_millis,
            output_bytes,
            environment: Box::default(),
            working_directory: None,
            user: None,
            stdin: Box::default(),
        };
        command.check_encoded_size()?;
        Ok(command)
    }

    /// Attaches the environment, working directory, user, and standard input this command runs
    /// with, replacing whatever it carried before.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidCommand`] when a context field breaks its own bound or when the
    /// command with that context no longer fits one record.
    pub fn with_context(mut self, context: CommandContext) -> Result<Self, Error> {
        let parts = context.into_checked_parts()?;
        self.environment = parts.environment;
        self.working_directory = parts.working_directory;
        self.user = parts.user;
        self.stdin = parts.stdin;
        self.check_encoded_size()?;
        Ok(self)
    }

    /// Returns the exact executable path bytes.
    #[must_use]
    pub fn program(&self) -> &[u8] {
        &self.program
    }

    /// Returns the exact shell-free argument byte strings.
    #[must_use]
    pub fn arguments(&self) -> &[Box<[u8]>] {
        &self.arguments
    }

    /// Returns the command timeout in whole milliseconds.
    #[must_use]
    pub const fn timeout_millis(&self) -> u32 {
        self.timeout_millis
    }

    /// Returns the combined stdout and stderr allowance.
    #[must_use]
    pub const fn output_bytes(&self) -> u64 {
        self.output_bytes
    }

    /// Returns the environment name and value pairs in the order the caller gave them.
    #[must_use]
    pub fn environment(&self) -> &[EnvironmentPair] {
        &self.environment
    }

    /// Returns the absolute working directory, or `None` to leave the agent's default in place.
    #[must_use]
    pub fn working_directory(&self) -> Option<&[u8]> {
        self.working_directory.as_deref()
    }

    /// Returns the user name to run as, or `None` to leave the agent's own account in place.
    #[must_use]
    pub fn user(&self) -> Option<&[u8]> {
        self.user.as_deref()
    }

    /// Returns the standard input delivered with the command, which is empty when there is none.
    #[must_use]
    pub fn stdin(&self) -> &[u8] {
        &self.stdin
    }

    /// Rejects a command whose encoded body would not fit one record.
    ///
    /// Individually admissible fields still add up, so the aggregate is checked once here
    /// rather than left to be inferred from the per-field bounds.
    fn check_encoded_size(&self) -> Result<(), Error> {
        if self.encoded_size() > super::MAX_BODY_SIZE {
            return Err(Error::InvalidCommand);
        }
        Ok(())
    }

    pub(super) fn encoded_size(&self) -> usize {
        fn field(bytes: &[u8]) -> usize {
            2 + bytes.len()
        }
        FIXED_BODY_SIZE
            + self.program.len()
            + self
                .arguments
                .iter()
                .map(|argument| field(argument))
                .sum::<usize>()
            + self
                .environment
                .iter()
                .map(|(name, value)| field(name) + field(value))
                .sum::<usize>()
            + self.working_directory.as_deref().map_or(0, field)
            + self.user.as_deref().map_or(0, field)
            + self.stdin.len()
    }
}

impl fmt::Debug for GuestCommand {
    /// Reports shapes and lengths, and never a byte of what the caller sent.
    ///
    /// An environment value, a working directory, and standard input are tenant data: a token
    /// in an environment variable is the ordinary case rather than the exceptional one, so none
    /// of them may reach a log through a formatter.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestCommand")
            .field("program_bytes", &self.program.len())
            .field("argument_count", &self.arguments.len())
            .field("timeout_millis", &self.timeout_millis)
            .field("output_bytes", &self.output_bytes)
            .field("environment_count", &self.environment.len())
            .field(
                "working_directory_bytes",
                &self.working_directory.as_ref().map(|path| path.len()),
            )
            .field("user_bytes", &self.user.as_ref().map(|user| user.len()))
            .field("stdin_bytes", &self.stdin.len())
            .finish()
    }
}

/// Rejects a field this protocol will not carry: oversized, nul-bearing, or empty where a value
/// is required.
pub(super) fn check_field(field: &[u8], empty_allowed: bool) -> Result<(), Error> {
    if (!empty_allowed && field.is_empty()) || field.len() > MAX_FIELD_BYTES || field.contains(&0) {
        return Err(Error::InvalidCommand);
    }
    Ok(())
}
