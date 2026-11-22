//! The two ways a host ends an authenticated session: Shutdown, which stops the guest agent, and
//! Suspend (ADR 0034, guest API level 4), which has it retire the session and park at the
//! suspension point with every process kept.
//!
//! Both are one request answered by one exact acknowledgement naming the same operation, and both
//! consume the owner: after either, nothing on this session can be sent again.

use crate::{GuestMessage, HostMessage, OperationId};

use super::super::{
    channel::channel_failure,
    deadline,
    error::{ControlError, ControlFailureClass, ControlStage},
};
use super::{HostControlIo, RepairedHostControl};

/// Which ending a request asks for.
#[derive(Clone, Copy)]
enum End {
    Shutdown,
    Suspend,
}

impl End {
    const fn stage(self) -> ControlStage {
        match self {
            Self::Shutdown => ControlStage::Shutdown,
            Self::Suspend => ControlStage::Suspend,
        }
    }

    const fn request(self, operation: OperationId) -> HostMessage {
        match self {
            Self::Shutdown => HostMessage::shutdown(operation),
            Self::Suspend => HostMessage::suspend(operation),
        }
    }

    /// The operation an acknowledgement of this kind names, if the message is one.
    const fn acknowledged(self, message: &GuestMessage) -> Option<OperationId> {
        match (self, message) {
            (Self::Shutdown, GuestMessage::ShutdownAck { operation })
            | (Self::Suspend, GuestMessage::SuspendAck { operation }) => Some(*operation),
            _ => None,
        }
    }
}

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Requires one exact authenticated acknowledgement and consumes the session.
    ///
    /// # Errors
    ///
    /// Returns a redacted Shutdown error after poisoning the transport exactly once.
    pub fn shutdown(self, operation: OperationId) -> Result<(), ControlError> {
        self.end(End::Shutdown, operation)
    }

    /// Asks the guest to retire this session and park at the suspension point, and consumes the
    /// owner once it acknowledged. Send it only to a guest that answered API level 4 or higher:
    /// an older guest refuses the request and poisons the session.
    ///
    /// # Errors
    ///
    /// Returns a redacted Suspend error after poisoning the transport exactly once.
    pub fn suspend(self, operation: OperationId) -> Result<(), ControlError> {
        self.end(End::Suspend, operation)
    }

    fn end(mut self, end: End, operation: OperationId) -> Result<(), ControlError> {
        let stage = end.stage();
        if !self.operations.reserve(operation) {
            return Err(self.channel.fail(stage, ControlFailureClass::Lifecycle));
        }
        let deadline = deadline::shutdown();
        if let Err(failure) = self.channel.send_host(&end.request(operation), deadline) {
            return Err(channel_failure(&mut self.channel, stage, failure));
        }
        let response = match self.channel.receive_guest(deadline) {
            Ok(message) => message,
            Err(failure) => return Err(channel_failure(&mut self.channel, stage, failure)),
        };
        match end.acknowledged(&response) {
            Some(answered) if answered == operation => Ok(()),
            Some(_) => Err(self.channel.fail(stage, ControlFailureClass::Protocol)),
            None => Err(self.channel.fail(stage, ControlFailureClass::Lifecycle)),
        }
    }
}
