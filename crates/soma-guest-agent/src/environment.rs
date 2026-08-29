//! Fixed process environment policy for direct command execution.
//!
//! Version 1 of the application wire contract carries no environment, working-directory, or
//! input fields, so the agent applies one fixed policy: an empty inherited environment replaced
//! by the allowlist below, the root directory as working directory, and a closed standard input.
//! Adding caller-controlled environment requires a new wire version and an ADR.

use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;

use soma_guest::GuestCommand;

/// Working directory for every direct command.
pub const WORKING_DIRECTORY: &str = "/";

/// The complete environment visible to a direct command.
pub const ENVIRONMENT: &[(&str, &str)] = &[
    ("HOME", "/root"),
    ("LANG", "C.UTF-8"),
    (
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    ),
    ("SOMA_SANDBOX", "1"),
    ("TERM", "dumb"),
];

/// Maximum number of arguments the agent will pass to `execve`.
pub const MAX_ARGUMENTS: usize = 64;
/// Maximum byte length of the program path or one argument.
pub const MAX_FIELD_BYTES: usize = 4096;

/// Argument vector for one direct `execve` without any shell.
#[derive(Debug, Eq, PartialEq)]
pub struct Invocation {
    program: OsString,
    arguments: Vec<OsString>,
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
            arguments.push(OsString::from(std::ffi::OsStr::from_bytes(argument)));
        }
        Ok(Self {
            program: OsString::from(std::ffi::OsStr::from_bytes(program)),
            arguments,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(program: &[u8], arguments: &[&[u8]]) -> GuestCommand {
        GuestCommand::new(
            program.to_vec(),
            arguments.iter().map(|argument| argument.to_vec()).collect(),
            1_000,
            1,
        )
        .expect("bounded command")
    }

    #[test]
    fn converts_program_and_arguments_without_interpretation() {
        let invocation =
            Invocation::from_command(&command(b"/bin/echo", &[b"$HOME", b"a b", b"", b"\xff"]))
                .expect("valid invocation");

        assert_eq!(invocation.program(), "/bin/echo");
        assert_eq!(invocation.arguments().len(), 4);
        assert_eq!(invocation.arguments()[0], "$HOME");
        assert_eq!(invocation.arguments()[1], "a b");
        assert_eq!(invocation.arguments()[2], "");
        assert_eq!(invocation.arguments()[3].as_bytes(), b"\xff");
    }

    #[test]
    fn the_environment_allowlist_is_sorted_and_free_of_shells() {
        let names: Vec<&str> = ENVIRONMENT.iter().map(|(name, _)| *name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();

        assert_eq!(names, sorted);
        assert!(ENVIRONMENT.iter().all(|(name, _)| name != &"SHELL"));
        assert_eq!(WORKING_DIRECTORY, "/");
    }

    #[test]
    fn local_bounds_match_the_wire_contract() {
        assert_eq!(MAX_ARGUMENTS, 64);
        assert_eq!(MAX_FIELD_BYTES, 4096);
        let sixty_four: Vec<&[u8]> = vec![b"x"; 64];
        assert!(Invocation::from_command(&command(b"/bin/true", &sixty_four)).is_ok());
        assert!(GuestCommand::new(b"bin/true".to_vec(), vec![], 1, 1).is_err());
    }
}
