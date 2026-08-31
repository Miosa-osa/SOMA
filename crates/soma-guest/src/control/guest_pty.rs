//! The one reply that answers an interactive terminal request.
//!
//! It sits beside the request loop for the same reason the filesystem reply does: the loop
//! carries the whole command lifecycle, and a terminal exchange shares none of it. There is no
//! streaming stage and no output accounting here, because terminal output is not command output:
//! it is not charged against a command's allowance and it does not end with a process status.

use std::time::Instant;

use crate::{GuestMessage, PtyOutcome};

use super::{
    channel::channel_failure,
    error::{ControlError, ControlFailureClass, ControlStage},
    guest::GuestControl,
    guest_state::{GuestState, active_stage},
    io::ControlIo,
};

impl<I: ControlIo> GuestControl<I> {
    /// Sends the one outcome that answers the pending terminal request and returns to idle.
    ///
    /// # Errors
    ///
    /// Returns a redacted Pty error after poisoning an illegal or failed outcome.
    pub fn pty_outcome(
        self,
        outcome: &PtyOutcome,
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        let Self {
            mut channel,
            state,
            operations,
        } = self;
        let operation = match state {
            GuestState::PtyPending(operation) => operation,
            other => return Err(channel.fail(active_stage(&other), ControlFailureClass::Lifecycle)),
        };
        if let Err(failure) = channel.send_guest(
            &GuestMessage::pty_outcome(operation, outcome.clone()),
            deadline,
        ) {
            return Err(channel_failure(&mut channel, ControlStage::Pty, failure));
        }
        Ok(Self {
            channel,
            state: GuestState::RepairedIdle,
            operations,
        })
    }
}
