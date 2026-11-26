//! Reading one reply packet back on the supervisor's side.
//!
//! The worker encodes a borrowed receipt; a supervisor holds no receipt to borrow, so it
//! decodes into owned values. The two live beside each other so the encoder and the decoder
//! cannot drift: every form the worker can send is a form this names.
//!
//! Only what a supervisor acts on is recovered. A failure's phase, recovery and milestone trail
//! are kept as the text the worker sent, because a supervisor forwards them and never branches
//! on them, and decoding a value nobody reads is a way for the two sides to disagree silently.

use crate::{ExitStatus, FailureKind, InstanceId, OperationId};

use super::{ControlError, field};

/// What one reply packet says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The Instance reached authenticated command readiness.
    Ready {
        operation_id: OperationId,
    },
    /// One command completed, and its output is this many bytes on each stream.
    Executed {
        operation_id: OperationId,
        status: ExitStatus,
        stdout_bytes: u64,
        stderr_bytes: u64,
    },
    /// The machine paused and is still resident.
    Paused(InstanceId),
    /// The resident machine resumed with the same identity.
    Resumed(InstanceId),
    /// The memory device's level, or why it refused.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Memory(Result<crate::sandbox::MemoryLevel, crate::sandbox::MemoryError>),
    /// The authenticated workspace result for the matching Instance.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Workspace(InstanceId, soma_guest::WorkspaceOutcome),
    Checkpointed(InstanceId, [u8; 32]),
    CheckpointUnconfirmed,
    FileComplete(InstanceId),
    /// The machine stopped and proved its cleanup.
    Stopped {
        guest_acknowledged: bool,
        forced: bool,
    },
    /// The request was performed and failed, with the contract's typed reason.
    Failure {
        kind: FailureKind,
        detail: String,
    },
    /// One bounded window of a completed command's output.
    Output(Vec<u8>),
    /// One chunk of a streaming command's output, before the command ended.
    Chunk(super::OutputStream, Vec<u8>),
    /// One bounded terminal answer.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Pty(soma::PtyAnswer),
    /// The filter reached its steady-state phase.
    Sealed,
    /// The request was not performed and Machine state did not change.
    Rejected(String),
}

impl Outcome {
    /// Parses one reply packet.
    ///
    /// # Errors
    ///
    /// Returns [`ControlError::UnknownRequest`] for a packet whose first word is not a reply
    /// form, or the [`ControlError`] naming the first field the packet does not satisfy.
    pub fn decode(text: &str) -> Result<Self, ControlError> {
        let (head, rest) = text.split_once(' ').unwrap_or((text, ""));
        match head {
            "sealed" => Ok(Self::Sealed),
            "rejected" => Ok(Self::Rejected(rest.to_owned())),
            "output" => field::bytes(Some(rest.trim()), "output").map(Self::Output),
            "chunk" => chunk(rest),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            "pty" => serde_json::from_slice(&field::bytes(Some(rest.trim()), "terminal answer")?)
                .map(Self::Pty)
                .map_err(|_| ControlError::InvalidValue("terminal answer")),
            "ready" => Ok(Self::Ready {
                operation_id: operation(rest)?,
            }),
            "executed" => executed(rest),
            "stopped" => stopped(rest),
            "paused" => resident_instance(rest).map(Self::Paused),
            "resumed" => resident_instance(rest).map(Self::Resumed),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            "memory" => super::memory::level(rest).map(|level| Self::Memory(Ok(level))),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            "memory_refused" => super::memory::refusal(rest).map(|error| Self::Memory(Err(error))),
            "file_complete" => resident_instance(rest).map(Self::FileComplete),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            "workspace" => workspace(rest),
            "checkpointed" => checkpointed(rest),
            "checkpoint_unconfirmed" => {
                field::end(rest.split_whitespace()).map(|()| Self::CheckpointUnconfirmed)
            }
            "failure" => failure(rest),
            _ => Err(ControlError::UnknownRequest),
        }
    }
}

/// `chunk <stream> <hex>`.
fn chunk(text: &str) -> Result<Outcome, ControlError> {
    let mut tokens = text.split_whitespace();
    let stream = tokens
        .next()
        .and_then(super::OutputStream::from_token)
        .ok_or(ControlError::InvalidValue("stream"))?;
    let bytes = field::bytes(tokens.next(), "chunk")?;
    field::end(tokens)?;
    Ok(Outcome::Chunk(stream, bytes))
}

/// The value of one `name=value` field of a reply.
fn value<'a>(text: &'a str, name: &'static str) -> Result<&'a str, ControlError> {
    text.split_whitespace()
        .find_map(|token| token.strip_prefix(name)?.strip_prefix('='))
        .ok_or(ControlError::MissingField(name))
}

fn operation(text: &str) -> Result<OperationId, ControlError> {
    let bytes = field::identifier(Some(value(text, "operation")?), "operation")?;
    OperationId::new(bytes).map_err(|_| ControlError::InvalidValue("operation"))
}

fn number(text: &str, name: &'static str) -> Result<u64, ControlError> {
    field::number(Some(value(text, name)?), name)
}

fn boolean(text: &str, name: &'static str) -> Result<bool, ControlError> {
    match value(text, name)? {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ControlError::InvalidValue(name)),
    }
}

fn executed(text: &str) -> Result<Outcome, ControlError> {
    let status = ExitStatus::from_token(value(text, "status")?)
        .ok_or(ControlError::InvalidValue("status"))?;
    Ok(Outcome::Executed {
        operation_id: operation(text)?,
        status,
        stdout_bytes: number(text, "stdout")?,
        stderr_bytes: number(text, "stderr")?,
    })
}

fn resident_instance(text: &str) -> Result<InstanceId, ControlError> {
    let mut tokens = text.split_whitespace();
    let value = tokens
        .next()
        .and_then(|token| token.strip_prefix("instance="));
    let instance = InstanceId::new(field::identifier(value, "instance")?)
        .map_err(|_| ControlError::InvalidValue("instance"))?;
    field::end(tokens)?;
    Ok(instance)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn workspace(text: &str) -> Result<Outcome, ControlError> {
    use soma_guest::WorkspaceOutcome;

    let mut tokens = text.split_whitespace();
    let instance = InstanceId::new(field::identifier(tokens.next(), "instance")?)
        .map_err(|_| ControlError::InvalidValue("instance"))?;
    let outcome = match tokens.next() {
        Some("0") => WorkspaceOutcome::Ready,
        Some("1") => WorkspaceOutcome::Refused,
        Some("3") => WorkspaceOutcome::Unsupported,
        _ => return Err(ControlError::InvalidValue("workspace outcome")),
    };
    field::end(tokens)?;
    Ok(Outcome::Workspace(instance, outcome))
}

fn checkpointed(text: &str) -> Result<Outcome, ControlError> {
    let mut tokens = text.split_whitespace();
    let instance = InstanceId::new(field::identifier(tokens.next(), "instance")?)
        .map_err(|_| ControlError::InvalidValue("instance"))?;
    let digest = field::identifier(tokens.next(), "checkpoint digest")?;
    field::end(tokens)?;
    Ok(Outcome::Checkpointed(instance, digest))
}

fn stopped(text: &str) -> Result<Outcome, ControlError> {
    Ok(Outcome::Stopped {
        guest_acknowledged: boolean(text, "acknowledged")?,
        forced: boolean(text, "forced")?,
    })
}

fn failure(text: &str) -> Result<Outcome, ControlError> {
    let kind =
        FailureKind::from_name(value(text, "kind")?).ok_or(ControlError::InvalidValue("kind"))?;
    Ok(Outcome::Failure {
        kind,
        detail: text.to_owned(),
    })
}

#[cfg(test)]
mod tests;
