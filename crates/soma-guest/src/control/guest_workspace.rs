//! The outcome of one authenticated workspace mount request.

use std::time::Instant;

use crate::{GuestMessage, WorkspaceOutcome};

use super::{
    channel::channel_failure,
    error::{ControlError, ControlFailureClass, ControlStage},
    guest::GuestControl,
    guest_state::{GuestState, active_stage},
    io::ControlIo,
};

impl<I: ControlIo> GuestControl<I> {
    /// Sends the one outcome that answers the pending workspace mount request and returns to idle.
    ///
    /// # Errors
    ///
    /// Returns a redacted Workspace error after poisoning an illegal or failed outcome.
    pub fn workspace_outcome(
        self,
        outcome: WorkspaceOutcome,
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        let Self {
            mut channel,
            state,
            operations,
        } = self;
        let operation = match state {
            GuestState::WorkspacePending(operation) => operation,
            other => return Err(channel.fail(active_stage(&other), ControlFailureClass::Lifecycle)),
        };
        if let Err(failure) = channel.send_guest(
            &GuestMessage::WorkspaceOutcome { operation, outcome },
            deadline,
        ) {
            return Err(channel_failure(
                &mut channel,
                ControlStage::Workspace,
                failure,
            ));
        }
        Ok(Self {
            channel,
            state: GuestState::RepairedIdle,
            operations,
        })
    }
}
