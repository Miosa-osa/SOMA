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
    context: ExecuteContext,
}

/// What a command runs with, beside its program and arguments.
///
/// The fields are plain bytes here; the bounds are the guest protocol's and are enforced where
/// the guest command is built, so this type cannot drift from them. An empty context is the
/// default and encodes exactly as a request did before contexts existed.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ExecuteContext {
    pub environment: Vec<(Vec<u8>, Vec<u8>)>,
    pub working_directory: Option<Vec<u8>>,
    pub user: Option<Vec<u8>>,
    pub stdin: Vec<u8>,
}

impl ExecuteContext {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.environment.is_empty()
            && self.working_directory.is_none()
            && self.user.is_none()
            && self.stdin.is_empty()
    }

    /// This context as the guest protocol's own, which re-checks every bound.
    ///
    /// The guest protocol is linked only where a machine can run, which is also the only place
    /// a worker decodes and performs a command.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[must_use]
    pub fn to_guest(&self) -> soma_guest::CommandContext {
        soma_guest::CommandContext {
            environment: self.environment.clone(),
            working_directory: self.working_directory.clone(),
            user: self.user.clone(),
            stdin: self.stdin.clone(),
        }
    }

    /// The bytes of every field, for memory accounting.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.environment
            .iter()
            .fold(0_usize, |total, (name, value)| {
                total.saturating_add(name.len()).saturating_add(value.len())
            })
            .saturating_add(self.working_directory.as_ref().map_or(0, Vec::len))
            .saturating_add(self.user.as_ref().map_or(0, Vec::len))
            .saturating_add(self.stdin.len())
    }
}

impl fmt::Debug for ExecuteContext {
    /// Shapes and lengths only: an environment value or standard input is tenant data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecuteContext")
            .field("environment_count", &self.environment.len())
            .field("has_working_directory", &self.working_directory.is_some())
            .field("has_user", &self.user.is_some())
            .field("stdin_bytes", &self.stdin.len())
            .finish()
    }
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
            context: ExecuteContext::default(),
        })
    }

    /// Runs the command with an environment, working directory, user and standard input.
    #[must_use]
    pub fn with_context(mut self, context: ExecuteContext) -> Self {
        self.context = context;
        self
    }

    #[must_use]
    pub const fn context(&self) -> &ExecuteContext {
        &self.context
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
            .field("context", &self.context)
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
