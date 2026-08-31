//! One bounded filesystem request and the guest's single answer to it.

use crate::{FileOutcome, FileRequest, GuestMessage, HostMessage};

use super::super::{
    channel::channel_failure,
    deadline,
    error::{ControlError, ControlFailureClass, ControlStage},
};
use super::{HostControlIo, RepairedHostControl, fresh_operation, message_operation};

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Issues one filesystem request and returns the guest's answer with the reusable owner.
    ///
    /// The operation identity is minted rather than taken from the caller, for the reason
    /// [`super::operation`] gives.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once.
    pub fn file(mut self, request: FileRequest) -> Result<(Self, FileOutcome), ControlError> {
        let Some(operation) = fresh_operation() else {
            return Err(self
                .channel
                .fail(ControlStage::File, ControlFailureClass::Io));
        };
        if !self.operations.reserve(operation) {
            return Err(self
                .channel
                .fail(ControlStage::File, ControlFailureClass::Lifecycle));
        }
        let deadline = deadline::file();
        if let Err(failure) = self
            .channel
            .send_host(&HostMessage::file(operation, request), deadline)
        {
            return Err(channel_failure(
                &mut self.channel,
                ControlStage::File,
                failure,
            ));
        }
        let response = match self.channel.receive_guest(deadline) {
            Ok(message) => message,
            Err(failure) => {
                return Err(channel_failure(
                    &mut self.channel,
                    ControlStage::File,
                    failure,
                ));
            }
        };
        if message_operation(&response) != operation {
            return Err(self
                .channel
                .fail(ControlStage::File, ControlFailureClass::Protocol));
        }
        match response {
            GuestMessage::FileOutcome { outcome, .. } => Ok((self, outcome)),
            GuestMessage::Stdout { .. }
            | GuestMessage::Stderr { .. }
            | GuestMessage::Terminal { .. }
            | GuestMessage::RepairComplete { .. }
            | GuestMessage::ShutdownAck { .. }
            | GuestMessage::PtyOutcome { .. } => Err(self
                .channel
                .fail(ControlStage::File, ControlFailureClass::Lifecycle)),
        }
    }
}
