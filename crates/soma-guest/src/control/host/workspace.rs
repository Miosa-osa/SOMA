//! An authenticated workspace mount request and its single outcome.

use crate::{GuestMessage, HostMessage, WorkspaceAction, WorkspaceOutcome};

use super::super::{
    channel::channel_failure,
    deadline,
    error::{ControlError, ControlFailureClass, ControlStage},
};
use super::{GuestApi, HostControlIo, RepairedHostControl, fresh_operation, message_operation};

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Issues one workspace mount request and returns the guest's answer with the reusable owner.
    ///
    /// The operation identity is minted rather than taken from the caller, for the reason
    /// the host-control operation policy gives.
    ///
    /// # Errors
    ///
    /// Returns a redacted Workspace error after poisoning the transport exactly once.
    pub fn workspace(
        mut self,
        action: WorkspaceAction,
    ) -> Result<(Self, WorkspaceOutcome), ControlError> {
        if !self.api.is_some_and(GuestApi::has_workspace_mounts) {
            return Ok((self, WorkspaceOutcome::Unsupported));
        }
        let Some(operation) = fresh_operation() else {
            return Err(self
                .channel
                .fail(ControlStage::Workspace, ControlFailureClass::Io));
        };
        if !self.operations.reserve(operation) {
            return Err(self
                .channel
                .fail(ControlStage::Workspace, ControlFailureClass::Lifecycle));
        }
        let deadline = deadline::file();
        if let Err(failure) = self
            .channel
            .send_host(&HostMessage::Workspace { operation, action }, deadline)
        {
            return Err(channel_failure(
                &mut self.channel,
                ControlStage::Workspace,
                failure,
            ));
        }
        let response = match self.channel.receive_guest(deadline) {
            Ok(message) => message,
            Err(failure) => {
                return Err(channel_failure(
                    &mut self.channel,
                    ControlStage::Workspace,
                    failure,
                ));
            }
        };
        if message_operation(&response) != operation {
            return Err(self
                .channel
                .fail(ControlStage::Workspace, ControlFailureClass::Protocol));
        }
        match response {
            GuestMessage::WorkspaceOutcome { outcome, .. } => Ok((self, outcome)),
            GuestMessage::Stdout { .. }
            | GuestMessage::Stderr { .. }
            | GuestMessage::Terminal { .. }
            | GuestMessage::RepairComplete { .. }
            | GuestMessage::SuspendAck { .. }
            | GuestMessage::ShutdownAck { .. }
            | GuestMessage::FileOutcome { .. }
            | GuestMessage::PtyOutcome { .. } => Err(self
                .channel
                .fail(ControlStage::Workspace, ControlFailureClass::Lifecycle)),
        }
    }
}
