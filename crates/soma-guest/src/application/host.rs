use crate::Error;

use super::{GuestCommand, OperationId, frame};

/// A host-to-guest application message carried by one authenticated record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostMessage {
    /// Repairs cloned state and runs the fixed Ready no-op through the command path.
    PrepareAndProbe {
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
    /// Requests graceful termination of the guest agent.
    Shutdown {
        /// Identity of this exact Stop operation.
        operation: OperationId,
    },
}

impl HostMessage {
    /// Creates a Repair and readiness-probe message.
    #[must_use]
    pub const fn prepare_and_probe(operation: OperationId) -> Self {
        Self::PrepareAndProbe { operation }
    }

    /// Creates an Execute message.
    #[must_use]
    pub const fn execute(operation: OperationId, command: GuestCommand) -> Self {
        Self::Execute { operation, command }
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
            Self::PrepareAndProbe { operation } => frame::encode(
                frame::Kind::PrepareAndProbe,
                *operation,
                &GuestCommand::readiness_probe().encode_body(),
            ),
            Self::Execute { operation, command } => {
                frame::encode(frame::Kind::Execute, *operation, &command.encode_body())
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
            frame::Kind::PrepareAndProbe => {
                let command = GuestCommand::decode_body(decoded.body)?;
                if command != GuestCommand::readiness_probe() {
                    return Err(Error::ApplicationMessageRejected);
                }
                Ok(Self::prepare_and_probe(decoded.operation))
            }
            frame::Kind::Execute => Ok(Self::execute(
                decoded.operation,
                GuestCommand::decode_body(decoded.body)?,
            )),
            frame::Kind::Shutdown if decoded.body.is_empty() => {
                Ok(Self::shutdown(decoded.operation))
            }
            frame::Kind::Shutdown
            | frame::Kind::RepairComplete
            | frame::Kind::Stdout
            | frame::Kind::Stderr
            | frame::Kind::Terminal
            | frame::Kind::ShutdownAck => Err(Error::ApplicationMessageRejected),
        }
    }
}
