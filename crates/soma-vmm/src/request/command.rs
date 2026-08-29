use std::fmt;

use crate::{InstanceId, OperationId};

use super::{CommandError, ExecutionLimits};

#[derive(Clone, Eq, PartialEq)]
pub struct Execute {
    operation_id: OperationId,
    instance_id: InstanceId,
    program: Program,
    arguments: Box<[Argument]>,
    limits: ExecutionLimits,
}

impl Execute {
    /// Creates a bounded direct-execution request.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::TooLarge`] when argument count or aggregate argument bytes exceed
    /// the contract limits.
    pub fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        program: Program,
        arguments: Vec<Argument>,
        limits: ExecutionLimits,
    ) -> Result<Self, CommandError> {
        if arguments.len() > 4_096 {
            return Err(CommandError::TooLarge("argument count"));
        }
        let argument_bytes = arguments.iter().try_fold(0_usize, |total, argument| {
            total.checked_add(argument.as_bytes().len())
        });
        if argument_bytes.is_none_or(|bytes| bytes > 1024 * 1024) {
            return Err(CommandError::TooLarge("argument bytes"));
        }

        Ok(Self {
            operation_id,
            instance_id,
            program,
            arguments: arguments.into_boxed_slice(),
            limits,
        })
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    #[must_use]
    pub fn arguments(&self) -> &[Argument] {
        &self.arguments
    }

    #[must_use]
    pub const fn limits(&self) -> ExecutionLimits {
        self.limits
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Program(Box<[u8]>);

impl Program {
    /// Creates a direct executable program value.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the value is not an absolute guest path, is empty, contains
    /// NUL, or exceeds 4 KiB.
    pub fn new(bytes: Vec<u8>) -> Result<Self, CommandError> {
        validate_command_bytes(&bytes, "program", false, 4_096)?;
        if !bytes.starts_with(b"/") {
            return Err(CommandError::NotAbsolute("program"));
        }
        Ok(Self(bytes.into_boxed_slice()))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Argument(Box<[u8]>);

impl Argument {
    /// Creates one direct-execution argument.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the value contains NUL or exceeds 128 KiB.
    pub fn new(bytes: Vec<u8>) -> Result<Self, CommandError> {
        validate_command_bytes(&bytes, "argument", true, 131_072)?;
        Ok(Self(bytes.into_boxed_slice()))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Execute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let argument_bytes = self
            .arguments
            .iter()
            .map(|argument| argument.as_bytes().len())
            .sum::<usize>();
        formatter
            .debug_struct("Execute")
            .field("operation_id", &self.operation_id)
            .field("instance_id", &self.instance_id)
            .field("program_bytes", &self.program.as_bytes().len())
            .field("argument_count", &self.arguments.len())
            .field("argument_bytes", &argument_bytes)
            .field("limits", &self.limits)
            .finish()
    }
}

impl fmt::Debug for Program {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Program")
            .field("bytes", &self.0.len())
            .finish()
    }
}

impl fmt::Debug for Argument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Argument")
            .field("bytes", &self.0.len())
            .finish()
    }
}

fn validate_command_bytes(
    bytes: &[u8],
    field: &'static str,
    empty_allowed: bool,
    maximum: usize,
) -> Result<(), CommandError> {
    if !empty_allowed && bytes.is_empty() {
        return Err(CommandError::Empty(field));
    }
    if bytes.contains(&0) {
        return Err(CommandError::ContainsNul(field));
    }
    if bytes.len() > maximum {
        return Err(CommandError::TooLarge(field));
    }
    Ok(())
}
