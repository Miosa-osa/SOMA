//! One interactive terminal request and the guest's single answer to it.
//!
//! The exchange has the shape of a filesystem request rather than of an Execute: one question,
//! one answer, and no streaming stage. Terminal output is a stream, but the record layer bounds
//! one message, so the stream is carried as a sequence of these exchanges with an explicit end
//! flag rather than as unsolicited guest records the host would have to be ready for at any
//! moment.

use crate::{GuestMessage, HostMessage, PtyOutcome, PtyRequest};

use super::super::{
    channel::channel_failure,
    deadline,
    error::{ControlError, ControlFailureClass, ControlStage},
};
use super::{HostControlIo, RepairedHostControl, fresh_operation, message_operation};

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Issues one terminal request and returns the guest's answer with the reusable owner.
    ///
    /// The operation identity is minted rather than taken from the caller, for the reason
    /// the host-control operation policy gives.
    ///
    /// # Errors
    ///
    /// Returns a redacted Pty error after poisoning the transport exactly once.
    pub fn pty(mut self, request: PtyRequest) -> Result<(Self, PtyOutcome), ControlError> {
        let Some(operation) = fresh_operation() else {
            return Err(self
                .channel
                .fail(ControlStage::Pty, ControlFailureClass::Io));
        };
        if !self.operations.reserve(operation) {
            return Err(self
                .channel
                .fail(ControlStage::Pty, ControlFailureClass::Lifecycle));
        }
        // The deadline is derived from the request because a read waits on purpose, so a fixed
        // stage budget would either cut a legitimate wait short or forgive a guest that stopped
        // answering the requests that do not wait at all.
        let deadline = deadline::pty(&request);
        if let Err(failure) = self
            .channel
            .send_host(&HostMessage::pty(operation, request), deadline)
        {
            return Err(channel_failure(
                &mut self.channel,
                ControlStage::Pty,
                failure,
            ));
        }
        let response = match self.channel.receive_guest(deadline) {
            Ok(message) => message,
            Err(failure) => {
                return Err(channel_failure(
                    &mut self.channel,
                    ControlStage::Pty,
                    failure,
                ));
            }
        };
        if message_operation(&response) != operation {
            return Err(self
                .channel
                .fail(ControlStage::Pty, ControlFailureClass::Protocol));
        }
        match response {
            GuestMessage::PtyOutcome { outcome, .. } => Ok((self, outcome)),
            GuestMessage::Stdout { .. }
            | GuestMessage::Stderr { .. }
            | GuestMessage::Terminal { .. }
            | GuestMessage::FileOutcome { .. }
            | GuestMessage::RepairComplete { .. }
            | GuestMessage::ShutdownAck { .. } => Err(self
                .channel
                .fail(ControlStage::Pty, ControlFailureClass::Lifecycle)),
        }
    }
}
