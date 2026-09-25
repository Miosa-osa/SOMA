//! The capture-time warm plan a Generation declares for its guest agent.
//!
//! A restored Instance starts from captured guest memory, so whatever a command faulted in
//! before capture is resident for every Instance at no cost. Reading a runtime binary pages in
//! its text but not what executing it maps: the dynamic linker, shared libraries, and the
//! interpreter's own startup data. A warm plan names a few commands the agent executes once,
//! at the disconnected repair point and before any launch material exists, so those pages are
//! captured too.
//!
//! The plan is part of the Generation, not of an Instance: the compiler writes its canonical
//! bytes into the initramfs, whose digest the manifest binds, and the agent reads the same
//! bytes back. Both sides use this one codec, so the text the compiler certified is exactly the
//! text the guest executes.
//!
//! The encoding is deliberately narrow. Each command is one line of space-separated arguments
//! drawn from a small character set with no quoting, globbing, or shell syntax, and the first
//! argument is an absolute executable path. A plan that does not re-encode to its own bytes is
//! rejected, so one plan has exactly one byte spelling and therefore one digest.

use std::fmt;

/// Most commands one plan may declare.
pub const MAX_WARM_COMMANDS: usize = 8;
/// Most arguments, including the executable, in one command.
pub const MAX_WARM_ARGUMENTS: usize = 16;
/// Most bytes of one encoded command line, excluding its newline.
pub const MAX_WARM_LINE_BYTES: usize = 256;
/// Most bytes of one encoded plan.
pub const MAX_WARM_PLAN_BYTES: usize = MAX_WARM_COMMANDS * (MAX_WARM_LINE_BYTES + 1);

/// Why a warm plan was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureWarmError {
    /// The plan declares no command; an absent plan is spelled by having none at all.
    Empty,
    /// The plan exceeds a command, argument, or byte bound.
    TooLarge,
    /// A command's executable is not an absolute path without parent components.
    Executable,
    /// An argument is empty or holds a byte outside the permitted set.
    Argument,
    /// The bytes are not the canonical encoding of the plan they decode to.
    NotCanonical,
}

impl fmt::Display for CaptureWarmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "capture warm plan declares no command",
            Self::TooLarge => "capture warm plan exceeds its bounds",
            Self::Executable => "capture warm command must name an absolute executable",
            Self::Argument => "capture warm argument holds a byte outside the permitted set",
            Self::NotCanonical => "capture warm plan is not canonically encoded",
        })
    }
}

impl std::error::Error for CaptureWarmError {}

/// One validated command: an absolute executable followed by its arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WarmCommand {
    argv: Vec<String>,
}

impl WarmCommand {
    /// Validates one command from its argument vector.
    ///
    /// # Errors
    ///
    /// Returns the first bound or character-set violation.
    pub fn new(argv: Vec<String>) -> Result<Self, CaptureWarmError> {
        if argv.is_empty() || argv.len() > MAX_WARM_ARGUMENTS {
            return Err(CaptureWarmError::TooLarge);
        }
        if !argv.iter().all(|argument| permitted(argument)) {
            return Err(CaptureWarmError::Argument);
        }
        let executable = &argv[0];
        if !executable.starts_with('/') || executable.split('/').any(|part| part == "..") {
            return Err(CaptureWarmError::Executable);
        }
        let line = argv.iter().map(String::len).sum::<usize>() + argv.len() - 1;
        if line > MAX_WARM_LINE_BYTES {
            return Err(CaptureWarmError::TooLarge);
        }
        Ok(Self { argv })
    }

    /// Parses one command from its space-separated spelling.
    ///
    /// # Errors
    ///
    /// Returns the first violation; doubled or edge spaces produce an empty argument.
    pub fn parse(line: &str) -> Result<Self, CaptureWarmError> {
        Self::new(line.split(' ').map(str::to_owned).collect())
    }

    /// The absolute executable path.
    #[must_use]
    pub fn executable(&self) -> &str {
        &self.argv[0]
    }

    /// The arguments after the executable.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.argv[1..]
    }
}

/// A validated, non-empty, bounded list of capture-time warm commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureWarmPlan {
    commands: Vec<WarmCommand>,
}

impl CaptureWarmPlan {
    /// Builds a plan from validated commands.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureWarmError::Empty`] or [`CaptureWarmError::TooLarge`].
    pub fn new(commands: Vec<WarmCommand>) -> Result<Self, CaptureWarmError> {
        if commands.is_empty() {
            return Err(CaptureWarmError::Empty);
        }
        if commands.len() > MAX_WARM_COMMANDS {
            return Err(CaptureWarmError::TooLarge);
        }
        Ok(Self { commands })
    }

    /// Decodes and validates the canonical bytes.
    ///
    /// # Errors
    ///
    /// Returns the first violation, including any spelling that is not canonical.
    pub fn decode(bytes: &[u8]) -> Result<Self, CaptureWarmError> {
        if bytes.len() > MAX_WARM_PLAN_BYTES {
            return Err(CaptureWarmError::TooLarge);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| CaptureWarmError::Argument)?;
        let Some(body) = text.strip_suffix('\n') else {
            return Err(if text.is_empty() {
                CaptureWarmError::Empty
            } else {
                CaptureWarmError::NotCanonical
            });
        };
        let commands = body
            .split('\n')
            .map(WarmCommand::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let plan = Self::new(commands)?;
        if plan.encode() != bytes {
            return Err(CaptureWarmError::NotCanonical);
        }
        Ok(plan)
    }

    /// The canonical bytes: one command per line, arguments joined by one space, each line
    /// terminated by a newline.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for command in &self.commands {
            bytes.extend_from_slice(command.argv.join(" ").as_bytes());
            bytes.push(b'\n');
        }
        bytes
    }

    /// The commands in declared order.
    #[must_use]
    pub fn commands(&self) -> &[WarmCommand] {
        &self.commands
    }
}

/// Whether one argument is non-empty and drawn only from the permitted set.
///
/// The set admits paths, flags, and simple values. It excludes whitespace, quotes, shell
/// metacharacters, and every control byte, so an argument cannot change how a line splits.
fn permitted(argument: &str) -> bool {
    !argument.is_empty()
        && argument.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'/' | b'.' | b'_' | b'-' | b'+' | b'=' | b':' | b',' | b'@'
                )
        })
}

#[cfg(test)]
mod tests;
