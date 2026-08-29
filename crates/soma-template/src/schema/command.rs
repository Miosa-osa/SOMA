//! The default command started once an Instance is Ready.

use super::{MAX_ARGUMENTS, MAX_STRING_BYTES};
use crate::error::BoundError;

/// The default program started once an Instance is Ready.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Command {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) working_directory: Option<String>,
    pub(crate) user: Option<String>,
}

impl Command {
    /// Creates a command without a working directory or user override.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError`] for an empty or oversized program, too many arguments, an
    /// oversized argument, or a NUL byte.
    pub fn new(program: &str, args: &[&str]) -> Result<Self, BoundError> {
        bounded_text("command.program", program)?;
        if args.len() > MAX_ARGUMENTS {
            return Err(BoundError::TooMany {
                field: "command.args".to_owned(),
                maximum: MAX_ARGUMENTS,
            });
        }
        for arg in args {
            if arg.len() > MAX_STRING_BYTES || arg.contains('\0') {
                return Err(BoundError::TooLong {
                    field: "command.args".to_owned(),
                    maximum: MAX_STRING_BYTES,
                });
            }
        }
        Ok(Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            working_directory: None,
            user: None,
        })
    }

    /// Sets the working directory; its shape is checked during validation.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError`] for an empty or oversized value.
    pub fn with_working_directory(mut self, directory: &str) -> Result<Self, BoundError> {
        bounded_text("command.working_directory", directory)?;
        self.working_directory = Some(directory.to_owned());
        Ok(self)
    }

    /// Sets the user; its shape is checked during validation.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError`] for an empty or oversized value.
    pub fn with_user(mut self, user: &str) -> Result<Self, BoundError> {
        bounded_text("command.user", user)?;
        self.user = Some(user.to_owned());
        Ok(self)
    }

    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    #[must_use]
    pub fn working_directory(&self) -> Option<&str> {
        self.working_directory.as_deref()
    }

    #[must_use]
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }
}

fn bounded_text(field: &str, value: &str) -> Result<(), BoundError> {
    if value.is_empty() {
        return Err(BoundError::Empty {
            field: field.to_owned(),
        });
    }
    if value.len() > MAX_STRING_BYTES {
        return Err(BoundError::TooLong {
            field: field.to_owned(),
            maximum: MAX_STRING_BYTES,
        });
    }
    if value.contains('\0') {
        return Err(BoundError::ForbiddenCharacter {
            field: field.to_owned(),
        });
    }
    Ok(())
}
