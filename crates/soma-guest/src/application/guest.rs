use crate::Error;

use super::{OperationId, OutputChunk, TerminalReport, frame};

/// A guest-to-host application message carried by one authenticated record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuestMessage {
    /// Reports completion of the fixed authenticated Repair contract.
    RepairComplete {
        /// Identity of the Launch operation being repaired.
        operation: OperationId,
    },
    /// Carries one bounded stdout chunk.
    Stdout {
        /// Identity of the current command operation.
        operation: OperationId,
        /// Exact binary output bytes.
        chunk: OutputChunk,
    },
    /// Carries one bounded stderr chunk.
    Stderr {
        /// Identity of the current command operation.
        operation: OperationId,
        /// Exact binary output bytes.
        chunk: OutputChunk,
    },
    /// Terminates one command output stream.
    Terminal {
        /// Identity of the completed command operation.
        operation: OperationId,
        /// Exact process or agent outcome.
        report: TerminalReport,
    },
    /// Acknowledges a graceful Shutdown request.
    ShutdownAck {
        /// Identity of the Stop operation.
        operation: OperationId,
    },
}

impl GuestMessage {
    /// Creates a Repair-complete message.
    #[must_use]
    pub const fn repair_complete(operation: OperationId) -> Self {
        Self::RepairComplete { operation }
    }

    /// Creates a stdout message.
    #[must_use]
    pub const fn stdout(operation: OperationId, chunk: OutputChunk) -> Self {
        Self::Stdout { operation, chunk }
    }

    /// Creates a stderr message.
    #[must_use]
    pub const fn stderr(operation: OperationId, chunk: OutputChunk) -> Self {
        Self::Stderr { operation, chunk }
    }

    /// Creates a terminal message.
    #[must_use]
    pub const fn terminal(operation: OperationId, report: TerminalReport) -> Self {
        Self::Terminal { operation, report }
    }

    /// Creates a graceful Shutdown acknowledgement.
    #[must_use]
    pub const fn shutdown_ack(operation: OperationId) -> Self {
        Self::ShutdownAck { operation }
    }

    /// Encodes this message as exactly one authenticated-record payload.
    ///
    /// # Errors
    ///
    /// Returns an error for a locally invalid terminal status.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        match self {
            Self::RepairComplete { operation } => {
                frame::encode(frame::Kind::RepairComplete, *operation, &[])
            }
            Self::Stdout { operation, chunk } => {
                frame::encode(frame::Kind::Stdout, *operation, chunk.as_bytes())
            }
            Self::Stderr { operation, chunk } => {
                frame::encode(frame::Kind::Stderr, *operation, chunk.as_bytes())
            }
            Self::Terminal { operation, report } => {
                frame::encode(frame::Kind::Terminal, *operation, &report.encode()?)
            }
            Self::ShutdownAck { operation } => {
                frame::encode(frame::Kind::ShutdownAck, *operation, &[])
            }
        }
    }

    /// Decodes one exact guest message from one authenticated-record payload.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ApplicationMessageRejected`] for every malformed message.
    pub fn decode(encoded: &[u8]) -> Result<Self, Error> {
        let decoded = frame::decode(encoded)?;
        match decoded.kind {
            frame::Kind::RepairComplete if decoded.body.is_empty() => {
                Ok(Self::repair_complete(decoded.operation))
            }
            frame::Kind::Stdout => Ok(Self::stdout(
                decoded.operation,
                OutputChunk::decode(decoded.body)?,
            )),
            frame::Kind::Stderr => Ok(Self::stderr(
                decoded.operation,
                OutputChunk::decode(decoded.body)?,
            )),
            frame::Kind::Terminal => Ok(Self::terminal(
                decoded.operation,
                TerminalReport::decode(decoded.body)?,
            )),
            frame::Kind::ShutdownAck if decoded.body.is_empty() => {
                Ok(Self::shutdown_ack(decoded.operation))
            }
            frame::Kind::PrepareAndProbe
            | frame::Kind::Execute
            | frame::Kind::Shutdown
            | frame::Kind::RepairComplete
            | frame::Kind::ShutdownAck => Err(Error::ApplicationMessageRejected),
        }
    }
}
