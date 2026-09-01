use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Serialize, Serializer, ser::SerializeStruct as _};
use soma::{BackendKind, CommandStatus, InstanceId, MachineState, TerminalStatus};

use crate::envelope::ApiError;

/// A lifecycle transition result.
///
/// `state` is the state the transition establishes, spelled with the same words the CLI prints,
/// so the two surfaces do not name the same sandbox state differently.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SandboxReport {
    pub instance_id: InstanceId,
    pub state: &'static str,
}

/// An observed sandbox, reusing the facade's own state and backend enums as the wire values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InspectionReport {
    pub instance_id: InstanceId,
    pub state: MachineState,
    pub backend: BackendKind,
}

/// One sandbox in a listing.
///
/// `state` is what the durable record says the last completed transition left it in, and `host`
/// is what the backend could still reach at the moment of the listing. They are separate members
/// because they answer different questions and a client that needs a sandbox it can use has to
/// read both: a record can say `active` while the process that held its machine is gone.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SandboxListEntry {
    pub instance_id: InstanceId,
    pub state: &'static str,
    pub host: &'static str,
    pub backend: BackendKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Every sandbox this service's durable state holds that has not been released.
///
/// The count is stated beside the list rather than left to the client to derive, so a client
/// reading a partial response can tell that it did.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SandboxListReport {
    pub sandboxes: Vec<SandboxListEntry>,
    pub count: usize,
}

impl SandboxListReport {
    /// Builds the document from what the engine enumerated.
    #[must_use]
    pub fn new(entries: &[soma::SandboxEntry]) -> Self {
        let sandboxes: Vec<SandboxListEntry> = entries
            .iter()
            .map(|entry| SandboxListEntry {
                instance_id: entry.instance_id().clone(),
                state: entry.phase().code(),
                host: entry.liveness().code(),
                backend: entry.backend(),
                name: entry.name().map(|name| name.as_str().to_owned()),
            })
            .collect();
        Self {
            count: sandboxes.len(),
            sandboxes,
        }
    }
}

/// A completed command and its captured output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CommandReport {
    pub instance_id: InstanceId,
    pub execution: CommandStatus,
    pub stdout: OutputBytes,
    pub stderr: OutputBytes,
}

/// Guest output on the wire.
///
/// Guest bytes are not required to be UTF-8, so they are base64 encoded and shipped with their
/// decoded length. The encoding is stated in the document rather than assumed, which is the same
/// contract the CLI publishes, so a client written against one reads the other unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputBytes(Box<[u8]>);

impl OutputBytes {
    #[must_use]
    pub fn new(bytes: &[u8]) -> Self {
        Self(Box::from(bytes))
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Serialize for OutputBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut output = serializer.serialize_struct("EncodedBytes", 3)?;
        output.serialize_field("encoding", "base64")?;
        output.serialize_field("byte_length", &self.len())?;
        output.serialize_field("data", &STANDARD.encode(&self.0))?;
        output.end()
    }
}

/// Narrows a receipt's terminal status to the four outcomes a command can end with.
///
/// A lifecycle terminal status reaching this function would mean the engine reported a command
/// receipt that did not describe a command, so it is refused as a contract failure rather than
/// coerced into a plausible exit code.
///
/// # Errors
///
/// Returns a 500 refusal for any terminal status that is not a command outcome.
pub fn command_status(status: TerminalStatus) -> Result<CommandStatus, ApiError> {
    match status {
        TerminalStatus::Exited { code } => Ok(CommandStatus::Exited { code }),
        TerminalStatus::Signaled { signal } => Ok(CommandStatus::Signaled { signal }),
        TerminalStatus::TimedOut => Ok(CommandStatus::TimedOut),
        TerminalStatus::OutputLimitExceeded => Ok(CommandStatus::OutputLimitExceeded),
        _ => Err(ApiError::internal(
            "the execution receipt did not carry a command terminal status",
        )),
    }
}
