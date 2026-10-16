//! The exec door's own body parameters, and the context one command runs with.
//!
//! They live apart from the create vocabulary so each door's parsing stays inside the source
//! budget; a child module may read the parent's imports, so the split costs nothing but a `use`.

use super::*;

/// The exec timeout when a request names none, as the fast lane applies it.
const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;
const MAX_EXEC_TIMEOUT_SECONDS: u64 = 86_400;

/// The exec body fields the fast lane accepted.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::runner::service) struct ExecParams {
    pub(in crate::runner::service) command: String,
    pub(in crate::runner::service) timeout_ms: u64,
    /// The environment, working directory and user the command runs with. Empty by default.
    pub(in crate::runner::service) context: soma::CommandContext,
}

impl ExecParams {
    pub(in crate::runner::service) fn parse(body: &[u8]) -> Result<Self, PlatformError> {
        let params = object(body)?;
        let command = match params.get("command") {
            Some(Value::String(command)) if !command.is_empty() => command.clone(),
            _ => return Err(PlatformError::missing_command()),
        };
        let context = exec_context(&params)?;
        let timeout_ms = match params.get("timeout") {
            None | Some(Value::Null) => DEFAULT_EXEC_TIMEOUT_MS,
            Some(value) => value
                .as_u64()
                .filter(|seconds| (1..=MAX_EXEC_TIMEOUT_SECONDS).contains(seconds))
                .map(|seconds| seconds * 1_000)
                .ok_or_else(PlatformError::invalid_timeout)?,
        };
        Ok(Self {
            command,
            timeout_ms,
            context,
        })
    }
}

/// The context an exec body names: `cwd`, `env` and `user`, each optional and null-tolerant.
///
/// `env` is a JSON object of string values, applied in the sorted order the object decodes to,
/// so one body always means one environment. Every bound is the guest protocol's own, checked
/// again against the guest constructor before a machine is asked.
fn exec_context(
    params: &serde_json::Map<String, Value>,
) -> Result<soma::CommandContext, PlatformError> {
    let text = |name: &str| match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(PlatformError::invalid_param(
            name,
            &format!("`{name}` must be a string"),
        )),
    };
    let environment = match params.get("env") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(variables)) => variables
            .iter()
            .map(|(name, value)| match value {
                Value::String(value) => Ok((name.clone(), value.clone())),
                _ => Err(PlatformError::invalid_param(
                    "env",
                    "every `env` value must be a string",
                )),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(PlatformError::invalid_param(
                "env",
                "`env` must be an object of string values",
            ));
        }
    };
    let stdin = match (text("stdin")?, text("stdin_base64")?) {
        (Some(_), Some(_)) => {
            return Err(PlatformError::invalid_param(
                "stdin",
                "send `stdin` or `stdin_base64`, not both",
            ));
        }
        (Some(text), None) => text.into_bytes(),
        (None, Some(encoded)) => {
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            STANDARD.decode(encoded).map_err(|_| {
                PlatformError::invalid_param("stdin_base64", "`stdin_base64` is not base64")
            })?
        }
        (None, None) => Vec::new(),
    };
    soma::CommandContext::new(environment, text("cwd")?, text("user")?, stdin).map_err(|_| {
        PlatformError::invalid_param(
            "env",
            "the environment, working directory, user or standard input is outside the exec contract",
        )
    })
}
