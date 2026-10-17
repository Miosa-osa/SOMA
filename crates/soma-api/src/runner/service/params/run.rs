//! The `run` block a create may carry: one command to run in the sandbox it creates.
//!
//! The command is built by the same function the exec endpoint builds its own with, against the
//! same contract the machine applies, so a create-and-run command is one the exec door would have
//! admitted rather than a second opinion about it. It is built here, before the sandbox exists,
//! so a command the runtime would refuse refuses the whole create: answering `201` for a sandbox
//! whose command could never run would be a sandbox the caller did not ask for.

#[allow(
    clippy::wildcard_imports,
    reason = "this module is its parent's body moved wholesale; naming every import would obscure it"
)]
use super::*;

use crate::runner::service::command::exec_command_in;

/// The command a create-and-run request carries, through every check the exec door makes.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::runner::service) struct RunCommand {
    pub(in crate::runner::service) command: DirectCommand,
    /// The deadline the command runs with, in milliseconds, as the exec door's own timeout is.
    pub(in crate::runner::service) timeout_ms: u64,
}

/// Every field the `run` block defines.
const RUN_FIELDS: [&str; 2] = ["command", "timeout_ms"];

impl RunCommand {
    /// Reads the block, or answers why it cannot be run.
    ///
    /// A block that is absent or null is a create with no command, which is the create that
    /// existed before this field did.
    pub(in crate::runner::service) fn parse(
        params: &serde_json::Map<String, Value>,
        shell_free_exec: bool,
    ) -> Result<Option<Self>, PlatformError> {
        let Some(value) = params.get("run") else {
            return Ok(None);
        };
        let Value::Object(block) = value else {
            return match value {
                Value::Null => Ok(None),
                _ => Err(PlatformError::invalid_param("run", "run must be an object")),
            };
        };
        for name in block.keys() {
            if !RUN_FIELDS.contains(&name.as_str()) {
                return Err(PlatformError::invalid_param(
                    name,
                    &format!("unknown parameter: the `run` block does not define `{name}`"),
                ));
            }
        }
        let command = match block.get("command") {
            Some(Value::String(text)) if !text.is_empty() => text,
            _ => return Err(PlatformError::missing_command()),
        };
        let timeout_ms = match block.get("timeout_ms") {
            None | Some(Value::Null) => exec::DEFAULT_EXEC_TIMEOUT_MS,
            Some(value) => value
                .as_u64()
                .filter(|millis| (1..=exec::MAX_EXEC_TIMEOUT_MS).contains(millis))
                .ok_or_else(PlatformError::invalid_timeout)?,
        };
        // A create's command has no context of its own: the block names a command and a
        // deadline, so the command runs in the sandbox's own environment, as a bare exec does.
        let command = exec_command_in(
            command,
            shell_free_exec,
            timeout_ms,
            &soma::CommandContext::default(),
        )?;
        Ok(Some(Self {
            command,
            timeout_ms,
        }))
    }
}
