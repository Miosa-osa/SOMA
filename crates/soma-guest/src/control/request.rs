use crate::{GuestCommand, OperationId};

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
    /// Stop the trusted guest agent.
    Shutdown {
        /// Exact Stop operation identity.
        operation: OperationId,
    },
}
