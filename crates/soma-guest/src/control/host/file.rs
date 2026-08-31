//! One bounded filesystem request and the guest's single answer to it.

use crate::{FileOutcome, FileRequest, GuestMessage, HostMessage, OperationId};

use super::super::{
    channel::channel_failure,
    deadline,
    error::{ControlError, ControlFailureClass, ControlStage},
};
use super::{HostControlIo, RepairedHostControl, message_operation};

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Issues one filesystem request and returns the guest's answer with the reusable owner.
    ///
    /// The operation identity is minted here rather than taken from the caller. An Execute
    /// identity names work the caller tracks across the whole system, while a filesystem
    /// identity exists only to pair one answer with one question, and a caller forced to invent
    /// one could reuse a value this session has already spent and lose the transport for it.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once.
    pub fn file(mut self, request: FileRequest) -> Result<(Self, FileOutcome), ControlError> {
        let Some(operation) = fresh_operation() else {
            // The only thing that can fail here is local randomness, and a session that cannot
            // name its next request can no longer tell an answer from a replay of an older one.
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
            | GuestMessage::ShutdownAck { .. } => Err(self
                .channel
                .fail(ControlStage::File, ControlFailureClass::Lifecycle)),
        }
    }
}

/// Mints the identity of one filesystem request from operating-system randomness.
///
/// A random identity is not one the caller can also have chosen for an Execute or a Shutdown, so
/// minting one here never spends a value the caller was still going to need.
fn fresh_operation() -> Option<OperationId> {
    let mut bytes = [0_u8; 16];
    crate::resolver::fill_os_random(&mut bytes).ok()?;
    OperationId::new(bytes).ok()
}
