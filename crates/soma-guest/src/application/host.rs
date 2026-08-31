use crate::Error;

use super::{FileRequest, GuestCommand, OperationId, PtyRequest, frame};

/// A host-to-guest application message carried by one authenticated record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostMessage {
    /// Commits repair of cloned state under this Instance's authenticated session.
    Prepare {
        /// Identity of this exact Launch operation.
        operation: OperationId,
    },
    /// Executes one bounded direct command after Repair.
    Execute {
        /// Identity of this exact Execute operation.
        operation: OperationId,
        /// Shell-free direct invocation.
        command: GuestCommand,
    },
    /// Asks one bounded filesystem operation of the guest after Repair.
    File {
        /// Identity of this exact operation.
        operation: OperationId,
        /// The bounded request.
        request: FileRequest,
    },
    /// Asks one interactive terminal operation of the guest after Repair.
    Pty {
        /// Identity of this exact operation.
        operation: OperationId,
        /// The bounded request.
        request: PtyRequest,
    },
    /// Requests graceful termination of the guest agent.
    Shutdown {
        /// Identity of this exact Stop operation.
        operation: OperationId,
    },
}

impl HostMessage {
    /// Creates a Repair message.
    #[must_use]
    pub const fn prepare(operation: OperationId) -> Self {
        Self::Prepare { operation }
    }

    /// Creates an Execute message.
    #[must_use]
    pub const fn execute(operation: OperationId, command: GuestCommand) -> Self {
        Self::Execute { operation, command }
    }

    /// Creates one filesystem message.
    #[must_use]
    pub const fn file(operation: OperationId, request: FileRequest) -> Self {
        Self::File { operation, request }
    }

    /// Creates one interactive terminal message.
    #[must_use]
    pub const fn pty(operation: OperationId, request: PtyRequest) -> Self {
        Self::Pty { operation, request }
    }

    /// Creates a graceful Shutdown message.
    #[must_use]
    pub const fn shutdown(operation: OperationId) -> Self {
        Self::Shutdown { operation }
    }

    /// Encodes this message as exactly one authenticated-record payload.
    ///
    /// # Errors
    ///
    /// Returns an error if the message cannot fit one record.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        match self {
            Self::Prepare { operation } => frame::encode(frame::Kind::Prepare, *operation, &[]),
            Self::Execute { operation, command } => {
                frame::encode(frame::Kind::Execute, *operation, &command.encode_body())
            }
            Self::File { operation, request } => {
                frame::encode(frame::Kind::File, *operation, &request.encode_body())
            }
            Self::Pty { operation, request } => {
                frame::encode(frame::Kind::Pty, *operation, &request.encode_body())
            }
            Self::Shutdown { operation } => frame::encode(frame::Kind::Shutdown, *operation, &[]),
        }
    }

    /// Decodes one exact host message from one authenticated-record payload.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ApplicationMessageRejected`] for every malformed message.
    pub fn decode(encoded: &[u8]) -> Result<Self, Error> {
        let decoded = frame::decode(encoded)?;
        match decoded.kind {
            frame::Kind::Prepare if decoded.body.is_empty() => Ok(Self::prepare(decoded.operation)),
            frame::Kind::Execute => Ok(Self::execute(
                decoded.operation,
                GuestCommand::decode_body(decoded.body)?,
            )),
            frame::Kind::File => Ok(Self::file(
                decoded.operation,
                FileRequest::decode_body(decoded.body)?,
            )),
            frame::Kind::Pty => Ok(Self::pty(
                decoded.operation,
                PtyRequest::decode_body(decoded.body)?,
            )),
            frame::Kind::Shutdown if decoded.body.is_empty() => {
                Ok(Self::shutdown(decoded.operation))
            }
            frame::Kind::Prepare
            | frame::Kind::Shutdown
            | frame::Kind::FileOutcome
            | frame::Kind::RepairComplete
            | frame::Kind::Stdout
            | frame::Kind::Stderr
            | frame::Kind::Terminal
            | frame::Kind::ShutdownAck
            | frame::Kind::PtyOutcome => Err(Error::ApplicationMessageRejected),
        }
    }
}
