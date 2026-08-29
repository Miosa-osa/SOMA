use std::fmt;

use serde::Serialize;

use super::{RequestError, RequestErrorReason};

const MAX_ARGUMENTS: usize = 4_096;
const MAX_COMMAND_BYTES: usize = 1_048_576;

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct GuestCommand {
    program: String,
    arguments: Box<[String]>,
}

impl GuestCommand {
    /// Creates a bounded command with a caller-selected absolute guest path.
    ///
    /// # Errors
    ///
    /// Returns an error when the program is not absolute, a value contains NUL, or the command
    /// exceeds its argument or byte allowance.
    pub fn new(
        program: impl Into<String>,
        arguments: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, RequestError> {
        let program = program.into();
        if !program.starts_with('/') {
            return Err(RequestError::new(
                "program",
                RequestErrorReason::NotAbsolute,
            ));
        }
        if program.contains('\0') {
            return Err(RequestError::new(
                "program",
                RequestErrorReason::ContainsNul,
            ));
        }
        let arguments = arguments.into_iter().map(Into::into).collect::<Vec<_>>();
        if arguments.len() > MAX_ARGUMENTS {
            return Err(RequestError::new("arguments", RequestErrorReason::TooLarge));
        }
        if arguments.iter().any(|argument| argument.contains('\0')) {
            return Err(RequestError::new(
                "arguments",
                RequestErrorReason::ContainsNul,
            ));
        }
        let bytes = arguments.iter().fold(program.len(), |total, argument| {
            total.saturating_add(argument.len())
        });
        if bytes > MAX_COMMAND_BYTES {
            return Err(RequestError::new("command", RequestErrorReason::TooLarge));
        }
        Ok(Self {
            program,
            arguments: arguments.into_boxed_slice(),
        })
    }

    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
}

impl fmt::Debug for GuestCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestCommand")
            .field("program_bytes", &self.program.len())
            .field("argument_count", &self.arguments.len())
            .field(
                "argument_bytes",
                &self.arguments.iter().map(String::len).sum::<usize>(),
            )
            .finish()
    }
}
