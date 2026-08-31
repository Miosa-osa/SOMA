//! What a repaired, idle guest does with each request the host can send it.
//!
//! It lives beside the request loop rather than inside it because the loop's own job is the
//! stages around the idle point: refusing a request in a state that cannot receive one, reading
//! the record, and deciding whether the message belongs to repair or to ordinary service. Once
//! it is ordinary service, every arm does the same three things in the same order, and each new
//! request kind adds one more of them.
//!
//! Every kind reserves its identity against the operation ledger first. That is what stops a
//! host replaying an identity this session has already spent, whichever kind of work it names.

use crate::HostMessage;

use super::{
    channel::AuthChannel,
    error::{ControlError, ControlFailureClass, ControlStage},
    exchange::OutputAccounting,
    guest::GuestControl,
    guest_state::{ActiveExchange, GuestState},
    io::ControlIo,
    operation_ledger::OperationLedger,
    request::GuestRequest,
};

/// Accepts one request into the state that serves it, or fails the session.
pub(super) fn accept<I: ControlIo>(
    mut channel: AuthChannel<I>,
    mut operations: OperationLedger,
    request: HostMessage,
) -> Result<(GuestControl<I>, GuestRequest), ControlError> {
    let (failing_stage, operation) = match &request {
        HostMessage::Execute { operation, .. } => (ControlStage::Execute, *operation),
        HostMessage::File { operation, .. } => (ControlStage::File, *operation),
        HostMessage::Pty { operation, .. } => (ControlStage::Pty, *operation),
        HostMessage::Shutdown { operation } => (ControlStage::Shutdown, *operation),
        // Repair happened once, before this owner was ever idle, so asking for it again is a
        // host that lost track of the lifecycle rather than a request this state can serve.
        HostMessage::PrepareAndProbe { .. } => {
            return Err(channel.fail(ControlStage::Repair, ControlFailureClass::Lifecycle));
        }
    };
    if !operations.reserve(operation) {
        return Err(channel.fail(failing_stage, ControlFailureClass::Lifecycle));
    }
    let (state, accepted) = match request {
        HostMessage::Execute { operation, command } => {
            let accounting = OutputAccounting::new(command.output_bytes());
            (
                GuestState::ExecuteStreaming(ActiveExchange {
                    operation,
                    accounting,
                }),
                GuestRequest::Execute { operation, command },
            )
        }
        HostMessage::File { operation, request } => (
            GuestState::FilePending(operation),
            GuestRequest::File { operation, request },
        ),
        HostMessage::Pty { operation, request } => (
            GuestState::PtyPending(operation),
            GuestRequest::Pty { operation, request },
        ),
        HostMessage::Shutdown { operation } => (
            GuestState::ShutdownPending(operation),
            GuestRequest::Shutdown { operation },
        ),
        HostMessage::PrepareAndProbe { .. } => unreachable!("repair was refused above"),
    };
    Ok((
        GuestControl {
            channel,
            state,
            operations,
        },
        accepted,
    ))
}
