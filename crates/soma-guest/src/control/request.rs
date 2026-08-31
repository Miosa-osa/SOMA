use crate::{FileRequest, GuestCommand, OperationId, PtyRequest};

/// One lifecycle-validated request received by the trusted guest agent.
#[derive(Debug, Eq, PartialEq)]
pub enum GuestRequest {
    /// Complete certified repair, then run the fixed version 1 self-probe.
    PrepareAndProbe {
        /// Exact Launch operation bound into the authenticated session.
        operation: OperationId,
    },
    /// Execute one validated shell-free command.
    Execute {
        /// Exact Execute operation identity.
        operation: OperationId,
        /// Validated bounded direct command.
        command: GuestCommand,
    },
    /// Perform one bounded filesystem request and answer it with exactly one outcome.
    File {
        /// Exact filesystem operation identity.
        operation: OperationId,
        /// The decoded request, whose path this protocol has already bounded.
        request: FileRequest,
    },
    /// Perform one interactive terminal request and answer it with exactly one outcome.
    Pty {
        /// Exact terminal operation identity.
        operation: OperationId,
        /// The decoded request, whose dimensions and chunk this protocol has already bounded.
        request: PtyRequest,
    },
    /// Stop the trusted guest agent.
    Shutdown {
        /// Exact Stop operation identity.
        operation: OperationId,
    },
}
