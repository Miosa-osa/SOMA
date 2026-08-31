//! The one reply that answers a bounded filesystem request.
//!
//! It lives beside the request loop rather than inside it because the loop already carries the
//! whole command lifecycle, and a file exchange shares none of that: it has no streaming stage
//! and no output accounting, only a single outcome that returns the owner to idle.

use std::time::Instant;

use crate::{FileOutcome, GuestMessage};

use super::{
    channel::channel_failure,
    error::{ControlError, ControlFailureClass, ControlStage},
    guest::GuestControl,
    guest_state::{GuestState, active_stage},
    io::ControlIo,
};

impl<I: ControlIo> GuestControl<I> {
    /// Sends the one outcome that answers the pending filesystem request and returns to idle.
    ///
    /// A filesystem request is answered exactly once, so the outcome consumes the pending state
    /// rather than streaming like command output does.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning an illegal or failed outcome.
    pub fn file_outcome(
        self,
        outcome: &FileOutcome,
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        let Self {
            mut channel,
            state,
            operations,
        } = self;
        let operation = match state {
            GuestState::FilePending(operation) => operation,
            other => return Err(channel.fail(active_stage(&other), ControlFailureClass::Lifecycle)),
        };
        if let Err(failure) = channel.send_guest(
            &GuestMessage::file_outcome(operation, outcome.clone()),
            deadline,
        ) {
            return Err(channel_failure(&mut channel, ControlStage::File, failure));
        }
        Ok(Self {
            channel,
            state: GuestState::RepairedIdle,
            operations,
        })
    }
}
