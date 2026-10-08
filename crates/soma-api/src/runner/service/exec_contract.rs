//! The machine's exec contract, applied before a machine is asked to run anything.
//!
//! This runner's own request type admits commands the sandbox runtime cannot run. Its bounds allow
//! one argument of up to 128 KiB and up to 4096 of them, while the guest protocol carries at most
//! 4 KiB in any one field and 64 arguments in a whole command. A command between the two was
//! admitted here and refused later, inside the engine, and an exec that fails after it was
//! admitted is not a refusal: the engine releases the machine and writes its terminal phase. The
//! caller lost the sandbox, and read the answer as an agent outage.
//!
//! So the size has to be decided here, where turning a request down costs nothing, and it has to be
//! decided against the contract the machine actually applies. The bounds below are imported from
//! `soma_guest`, and the verdict is the guest protocol's own constructor, so the set admitted here
//! is the set every later layer carries rather than a second opinion about it.

use soma_guest::{
    FIXED_BODY_SIZE, MAX_ARGUMENTS, MAX_BODY_SIZE, MAX_FIELD_BYTES, MAX_TIMEOUT_MILLIS,
};

use super::super::public_wire::PlatformError;

/// Why the runner will not ask a machine to run one command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Refusal {
    /// One field of the command is larger than the guest protocol carries.
    TooLarge {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
    /// Each field fits, but the command as a whole does not fit one guest message.
    TooLargeTogether { limit: usize, actual: usize },
    /// The command asks for a deadline the sandbox runtime cannot bound.
    TimeoutUnsupported {
        limit_millis: u32,
        actual_millis: u64,
    },
    /// The command is not one this contract can express at all.
    Invalid,
}

impl Refusal {
    /// The answer one caller reads for this refusal.
    ///
    /// The codes live here rather than in the wire module because the refusal and its answer are
    /// one decision, and because each of these has to be something a caller can act on: a request
    /// that names the field it overshot is fixable, while the `AGENT_UNAVAILABLE` an oversize
    /// command used to produce told the caller to retry a request that could never succeed.
    pub(super) fn error(self) -> PlatformError {
        match self {
            Self::TooLarge {
                field,
                limit,
                actual,
            } => too_large(field, limit, actual),
            Self::TooLargeTogether { limit, actual } => too_large("command", limit, actual),
            Self::TimeoutUnsupported {
                limit_millis,
                actual_millis,
            } => timeout_unsupported(limit_millis, actual_millis),
            Self::Invalid => PlatformError::invalid_param(
                "command",
                "the command is not one the sandbox runtime can run",
            ),
        }
    }
}

/// Admits one planned command, or says which part of it the machine would refuse.
///
/// The verdict is the machine's: the parts are offered to [`soma_guest::GuestCommand::new`], the
/// same constructor the guest protocol enforces, so nothing this admits can be refused later. The
/// checks written out before it exist only to name the part that was too large, because that
/// constructor answers one error for every refusal.
pub(super) fn admit(program: &str, arguments: &[&str], timeout_millis: u64) -> Result<(), Refusal> {
    // The plan only ever yields an absolute program, and the guest execs an absolute path only, so
    // this arm is a guard on that rather than a case a caller can reach.
    if program.is_empty()
        || !program.starts_with('/')
        || program.contains('\0')
        || arguments.iter().any(|argument| argument.contains('\0'))
    {
        return Err(Refusal::Invalid);
    }
    if program.len() > MAX_FIELD_BYTES {
        return Err(Refusal::TooLarge {
            field: "program",
            limit: MAX_FIELD_BYTES,
            actual: program.len(),
        });
    }
    if let Some(argument) = arguments
        .iter()
        .find(|argument| argument.len() > MAX_FIELD_BYTES)
    {
        return Err(Refusal::TooLarge {
            field: "argument",
            limit: MAX_FIELD_BYTES,
            actual: argument.len(),
        });
    }
    if arguments.len() > MAX_ARGUMENTS {
        return Err(Refusal::TooLarge {
            field: "argument count",
            limit: MAX_ARGUMENTS,
            actual: arguments.len(),
        });
    }
    let Some(timeout) = u32::try_from(timeout_millis)
        .ok()
        .filter(|timeout| (1..=MAX_TIMEOUT_MILLIS).contains(timeout))
    else {
        return Err(Refusal::TimeoutUnsupported {
            limit_millis: MAX_TIMEOUT_MILLIS,
            actual_millis: timeout_millis,
        });
    };
    match soma_guest::GuestCommand::new(
        program.as_bytes().to_vec(),
        arguments
            .iter()
            .map(|argument| argument.as_bytes().to_vec())
            .collect(),
        timeout,
        soma::ExecutionLimits::DEFAULT_MAX_OUTPUT_BYTES,
    ) {
        Ok(_) => Ok(()),
        // Everything else the constructor refuses was checked above, so what is left is the
        // command's own total: the fields fit one at a time but not together.
        Err(_) => Err(Refusal::TooLargeTogether {
            limit: MAX_BODY_SIZE,
            actual: encoded_body_bytes(program, arguments),
        }),
    }
}

/// The body bytes one command of these parts spends inside one guest message.
///
/// The fixed part and each field's length prefix are counted exactly as the guest codec counts
/// them, so the number a refused caller reads is the size the message would have been.
fn encoded_body_bytes(program: &str, arguments: &[&str]) -> usize {
    let arguments = arguments.iter().fold(0_usize, |total, argument| {
        total.saturating_add(2 + argument.len())
    });
    FIXED_BODY_SIZE
        .saturating_add(program.len())
        .saturating_add(arguments)
}

/// The answer for a command the runtime's exec contract will not carry.
///
/// `limit` and `actual` are the bytes of one field when a field is named, and body bytes when the
/// field is the whole `command`.
fn too_large(field: &str, limit: usize, actual: usize) -> PlatformError {
    PlatformError::new(
        413,
        "RUNNER_COMMAND_TOO_LARGE",
        "the command exceeds the sandbox runtime's exec contract",
        false,
    )
    .with_details(serde_json::json!({"field": field, "limit": limit, "actual": actual}))
}

/// The answer for a deadline longer than the runtime can bound one command to.
fn timeout_unsupported(limit_millis: u32, actual_millis: u64) -> PlatformError {
    PlatformError::new(
        400,
        "EXEC_TIMEOUT_UNSUPPORTED",
        "timeout is longer than the sandbox runtime can bound one command to",
        false,
    )
    .with_details(
        serde_json::json!({"field": "timeout", "limit_ms": limit_millis, "actual_ms": actual_millis}),
    )
}

#[cfg(test)]
#[path = "exec_contract_tests.rs"]
mod tests;
