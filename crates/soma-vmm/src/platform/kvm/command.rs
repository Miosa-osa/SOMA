//! The guest command one Execute asks for, under its own limits and context.

use super::{Execute, GuestCommand, PlatformFailure, Recovery};

/// Builds the guest command, refusing one this guest cannot be given as a request fault.
pub(super) fn guest_command(execute: &Execute) -> Result<GuestCommand, PlatformFailure> {
    let limits = execute.limits();
    GuestCommand::new(
        execute.program().as_bytes().to_vec(),
        execute
            .arguments()
            .iter()
            .map(|argument| argument.as_bytes().to_vec())
            .collect(),
        limits.timeout().get(),
        limits.output().get(),
    )
    .and_then(|command| {
        if execute.context().is_empty() {
            Ok(command)
        } else {
            command.with_context(execute.context().to_guest())
        }
    })
    .map_err(|_| PlatformFailure::new(Recovery::DoNotRetry))
}
