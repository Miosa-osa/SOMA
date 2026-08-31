//! One authenticated command turned into the exact operands of one `execve`.
//!
//! Every bound the wire contract already enforced is enforced again here. The protocol crate
//! and the agent are built together today, but the agent is the process that hands bytes to the
//! kernel, and it does not delegate the decision of what it is willing to hand over.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt as _;

use soma_guest::{EnvironmentPair, GuestCommand};

use super::user::{self, Credentials};
use super::{MAX_ARGUMENTS, MAX_ENVIRONMENT, MAX_FIELD_BYTES, WORKING_DIRECTORY, merge};

/// Argument vector, environment, directory, and account for one direct `execve` without any
/// shell.
#[derive(Debug, Eq, PartialEq)]
pub struct Invocation {
    program: OsString,
    arguments: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    working_directory: OsString,
    credentials: Option<Credentials>,
}

/// A command that violates the local execution bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidInvocation {
    /// The program path was empty, relative, oversized, or contained NUL.
    Program,
    /// Too many arguments were supplied.
    ArgumentCount,
    /// One argument was oversized or contained NUL.
    Argument,
    /// Too many environment variables were supplied, or one name or value was inadmissible.
    Environment,
    /// The working directory was empty, relative, oversized, or contained NUL.
    WorkingDirectory,
    /// The named user does not exist in the guest's own account database.
    User,
}

impl Invocation {
    /// Re-validates every bound locally and converts the command into `execve` operands.
    ///
    /// # Errors
    ///
    /// Returns the first violated bound.
    pub fn from_command(command: &GuestCommand) -> Result<Self, InvalidInvocation> {
        let program = command.program();
        if program.is_empty()
            || !program.starts_with(b"/")
            || program.len() > MAX_FIELD_BYTES
            || program.contains(&0)
        {
            return Err(InvalidInvocation::Program);
        }
        if command.arguments().len() > MAX_ARGUMENTS {
            return Err(InvalidInvocation::ArgumentCount);
        }
        let mut arguments = Vec::with_capacity(command.arguments().len());
        for argument in command.arguments() {
            if argument.len() > MAX_FIELD_BYTES || argument.contains(&0) {
                return Err(InvalidInvocation::Argument);
            }
            arguments.push(os_string(argument));
        }
        check_environment(command.environment())?;
        Ok(Self {
            program: os_string(program),
            arguments,
            environment: merge(command.environment()),
            working_directory: working_directory(command.working_directory())?,
            credentials: credentials(command.user())?,
        })
    }

    /// Returns the exact program path.
    #[must_use]
    pub fn program(&self) -> &OsString {
        &self.program
    }

    /// Returns the exact argument vector after `argv[0]`.
    #[must_use]
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// Returns the complete environment the program will see.
    #[must_use]
    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    /// Returns the directory the program will run in.
    #[must_use]
    pub fn working_directory(&self) -> &OsString {
        &self.working_directory
    }

    /// Returns the account the program will run as, or `None` for the agent's own.
    #[must_use]
    pub const fn credentials(&self) -> Option<Credentials> {
        self.credentials
    }
}

fn check_environment(environment: &[EnvironmentPair]) -> Result<(), InvalidInvocation> {
    if environment.len() > MAX_ENVIRONMENT {
        return Err(InvalidInvocation::Environment);
    }
    for (name, value) in environment {
        if name.is_empty()
            || name.len() > MAX_FIELD_BYTES
            || name.contains(&0)
            || name.contains(&b'=')
            || value.len() > MAX_FIELD_BYTES
            || value.contains(&0)
        {
            return Err(InvalidInvocation::Environment);
        }
    }
    Ok(())
}

fn working_directory(named: Option<&[u8]>) -> Result<OsString, InvalidInvocation> {
    let Some(path) = named else {
        return Ok(OsString::from(WORKING_DIRECTORY));
    };
    if path.is_empty()
        || !path.starts_with(b"/")
        || path.len() > MAX_FIELD_BYTES
        || path.contains(&0)
    {
        return Err(InvalidInvocation::WorkingDirectory);
    }
    Ok(os_string(path))
}

fn credentials(named: Option<&[u8]>) -> Result<Option<Credentials>, InvalidInvocation> {
    match named {
        None => Ok(None),
        Some(name) => user::resolve(name).map(Some).ok_or(InvalidInvocation::User),
    }
}

fn os_string(bytes: &[u8]) -> OsString {
    OsString::from(OsStr::from_bytes(bytes))
}
