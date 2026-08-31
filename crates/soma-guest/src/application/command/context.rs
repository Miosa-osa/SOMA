//! What a command runs with, beside its program and arguments.
//!
//! These four fields are gathered into one value rather than four more constructor parameters
//! so that adding the next one does not rewrite every call site, and so that a caller who wants
//! none of them keeps the plain constructor it already used.

use core::fmt;

use crate::Error;

use super::{EnvironmentPair, MAX_ENVIRONMENT, MAX_STDIN_BYTES, MAX_USER_BYTES, check_field};

/// The environment, working directory, user, and standard input of one command.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct CommandContext {
    /// Environment name and value pairs, applied in this order.
    ///
    /// The order is the caller's: a later pair with a name an earlier one already used is the
    /// caller's own decision, and the protocol carries it rather than silently resolving it.
    pub environment: Vec<(Vec<u8>, Vec<u8>)>,
    /// Absolute directory the command runs in, or `None` for the agent's default.
    pub working_directory: Option<Vec<u8>>,
    /// Name of the account the command runs as, or `None` for the agent's own.
    pub user: Option<Vec<u8>>,
    /// Bytes delivered on the command's standard input; empty means none.
    pub stdin: Vec<u8>,
}

/// The context after every bound was checked, in the representation the command stores.
pub(super) struct CheckedContext {
    pub(super) environment: Box<[EnvironmentPair]>,
    pub(super) working_directory: Option<Box<[u8]>>,
    pub(super) user: Option<Box<[u8]>>,
    pub(super) stdin: Box<[u8]>,
}

impl CommandContext {
    /// Checks every bound and converts the context into the command's own representation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidCommand`] for a name that is empty, oversized, nul-bearing, or
    /// carrying the `=` that separates a name from its value in every environment this
    /// protocol targets; for a relative, empty, oversized, or nul-bearing working directory;
    /// for an empty, oversized, or nul-bearing user; and for oversized standard input.
    pub(super) fn into_checked_parts(self) -> Result<CheckedContext, Error> {
        if self.environment.len() > MAX_ENVIRONMENT || self.stdin.len() > MAX_STDIN_BYTES {
            return Err(Error::InvalidCommand);
        }
        let mut environment = Vec::with_capacity(self.environment.len());
        for (name, value) in self.environment {
            check_name(&name)?;
            // A value may be empty: exporting an empty variable is different from not
            // exporting it at all, and both are things a caller legitimately asks for.
            check_field(&value, true)?;
            environment.push((name.into_boxed_slice(), value.into_boxed_slice()));
        }
        let working_directory = match self.working_directory {
            None => None,
            Some(path) => {
                check_field(&path, false)?;
                // A relative directory would be resolved against whatever the agent's own
                // working directory happened to be, so it names nothing the caller can predict.
                if !path.starts_with(b"/") {
                    return Err(Error::InvalidCommand);
                }
                Some(path.into_boxed_slice())
            }
        };
        let user = match self.user {
            None => None,
            Some(user) => {
                if user.is_empty() || user.len() > MAX_USER_BYTES || user.contains(&0) {
                    return Err(Error::InvalidCommand);
                }
                Some(user.into_boxed_slice())
            }
        };
        Ok(CheckedContext {
            environment: environment.into_boxed_slice(),
            working_directory,
            user,
            stdin: self.stdin.into_boxed_slice(),
        })
    }
}

impl fmt::Debug for CommandContext {
    /// Reports shapes and lengths only, for the same reason [`super::GuestCommand`] does.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandContext")
            .field("environment_count", &self.environment.len())
            .field(
                "working_directory_bytes",
                &self.working_directory.as_ref().map(Vec::len),
            )
            .field("user_bytes", &self.user.as_ref().map(Vec::len))
            .field("stdin_bytes", &self.stdin.len())
            .finish()
    }
}

/// Rejects an environment name this protocol will not carry.
///
/// Beyond the ordinary field bounds a name may not be empty and may not contain `=`: the
/// operating-system representation of an environment is a list of `name=value` strings, so a
/// name holding that byte would name a different variable once the agent assembled it.
fn check_name(name: &[u8]) -> Result<(), Error> {
    check_field(name, false)?;
    if name.contains(&b'=') {
        return Err(Error::InvalidCommand);
    }
    Ok(())
}
