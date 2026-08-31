use crate::{
    DeliveredHostLaunchMaterial, GuestCommand, GuestMessage, HostMessage, OperationId,
    SessionBinding,
};

use super::{
    channel::{AuthChannel, channel_failure},
    deadline,
    error::{ControlError, ControlFailureClass, ControlStage},
    exchange::OutputAccounting,
    host_connect,
    io::HostControlIo,
    operation_ledger::OperationLedger,
    outcome::ExecuteOutcome,
};

mod file;
mod network;
mod operation;
mod pty;
mod secret;
mod whole_file;

use operation::fresh_operation;

pub use secret::{SecretPlacement, SecretStage};

pub use whole_file::{WholeFileRead, WholeFileWrite};

/// Host owner of one authenticated pre-repair guest-control session.
pub struct HostControl<I: HostControlIo> {
    channel: AuthChannel<I>,
    binding: SessionBinding,
    launch_operation: OperationId,
    operations: OperationLedger,
}

/// Host owner after authenticated repair, repair commit, and the fixed self-probe succeed.
pub struct RepairedHostControl<I: HostControlIo> {
    channel: AuthChannel<I>,
    binding: SessionBinding,
    operations: OperationLedger,
}

impl<I: HostControlIo> HostControl<I> {
    /// Completes both handshake messages over the owned byte transport.
    ///
    /// # Errors
    ///
    /// Returns a redacted Handshake error after poisoning the transport exactly once.
    pub fn connect(material: DeliveredHostLaunchMaterial, io: I) -> Result<Self, ControlError> {
        let (channel, binding, launch_operation) = host_connect::connect(material, io)?;
        Ok(Self {
            channel,
            binding,
            launch_operation,
            operations: OperationLedger::new(launch_operation),
        })
    }

    /// Commits authenticated repair, which is the whole of the readiness transition.
    ///
    /// The guest's `RepairComplete` is authenticated under this Instance's own session, so it
    /// proves the same thing a command round trip would have proved about who is answering,
    /// and it proves it without one.
    ///
    /// # Errors
    ///
    /// Returns a redacted Repair error after poisoning the transport exactly once.
    pub fn prepare(mut self) -> Result<RepairedHostControl<I>, ControlError> {
        let repair_deadline = deadline::repair();
        let request = HostMessage::prepare(self.launch_operation);
        if let Err(failure) = self.channel.send_host(&request, repair_deadline) {
            return Err(channel_failure(
                &mut self.channel,
                ControlStage::Repair,
                failure,
            ));
        }
        let repair = match self.channel.receive_guest(repair_deadline) {
            Ok(message) => message,
            Err(failure) => {
                return Err(channel_failure(
                    &mut self.channel,
                    ControlStage::Repair,
                    failure,
                ));
            }
        };
        match repair {
            GuestMessage::RepairComplete { operation } if operation == self.launch_operation => {}
            GuestMessage::RepairComplete { .. } => {
                return Err(self
                    .channel
                    .fail(ControlStage::Repair, ControlFailureClass::Protocol));
            }
            _ => {
                return Err(self
                    .channel
                    .fail(ControlStage::Repair, ControlFailureClass::Lifecycle));
            }
        }
        if self.channel.io.commit_repair(repair_deadline).is_err() {
            return Err(self
                .channel
                .fail(ControlStage::Repair, ControlFailureClass::Io));
        }
        Ok(RepairedHostControl {
            channel: self.channel,
            binding: self.binding,
            operations: self.operations,
        })
    }
}

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Executes one operation and returns the only reusable repaired owner on success.
    ///
    /// # Errors
    ///
    /// Returns a redacted Execute error after poisoning the transport exactly once.
    pub fn execute(
        mut self,
        operation: OperationId,
        command: GuestCommand,
    ) -> Result<(Self, ExecuteOutcome), ControlError> {
        if !self.operations.reserve(operation) {
            return Err(self
                .channel
                .fail(ControlStage::Execute, ControlFailureClass::Lifecycle));
        }
        let allowance = command.output_bytes();
        let deadline = deadline::execute(&command);
        if let Err(failure) = self
            .channel
            .send_host(&HostMessage::execute(operation, command), deadline)
        {
            return Err(channel_failure(
                &mut self.channel,
                ControlStage::Execute,
                failure,
            ));
        }
        let mut accounting = OutputAccounting::new(allowance);
        loop {
            let response = match self.channel.receive_guest(deadline) {
                Ok(message) => message,
                Err(failure) => {
                    return Err(channel_failure(
                        &mut self.channel,
                        ControlStage::Execute,
                        failure,
                    ));
                }
            };
            if message_operation(&response) != operation {
                return Err(self
                    .channel
                    .fail(ControlStage::Execute, ControlFailureClass::Protocol));
            }
            match response {
                GuestMessage::Stdout { chunk, .. } => {
                    if accounting.push_stdout(&chunk).is_err() {
                        return Err(self
                            .channel
                            .fail(ControlStage::Execute, ControlFailureClass::Accounting));
                    }
                }
                GuestMessage::Stderr { chunk, .. } => {
                    if accounting.push_stderr(&chunk).is_err() {
                        return Err(self
                            .channel
                            .fail(ControlStage::Execute, ControlFailureClass::Accounting));
                    }
                }
                GuestMessage::Terminal { report, .. } => {
                    if accounting.validate(report).is_err() {
                        return Err(self
                            .channel
                            .fail(ControlStage::Execute, ControlFailureClass::Accounting));
                    }
                    let status = report.status();
                    let (stdout, stderr) = accounting.into_output();
                    return Ok((self, ExecuteOutcome::new(status, stdout, stderr)));
                }
                GuestMessage::RepairComplete { .. }
                | GuestMessage::ShutdownAck { .. }
                | GuestMessage::FileOutcome { .. }
                | GuestMessage::PtyOutcome { .. } => {
                    return Err(self
                        .channel
                        .fail(ControlStage::Execute, ControlFailureClass::Lifecycle));
                }
            }
        }
    }

    /// Requires one exact authenticated acknowledgement and consumes the session.
    ///
    /// # Errors
    ///
    /// Returns a redacted Shutdown error after poisoning the transport exactly once.
    pub fn shutdown(mut self, operation: OperationId) -> Result<(), ControlError> {
        if !self.operations.reserve(operation) {
            return Err(self
                .channel
                .fail(ControlStage::Shutdown, ControlFailureClass::Lifecycle));
        }
        let deadline = deadline::shutdown();
        if let Err(failure) = self
            .channel
            .send_host(&HostMessage::shutdown(operation), deadline)
        {
            return Err(channel_failure(
                &mut self.channel,
                ControlStage::Shutdown,
                failure,
            ));
        }
        let response = match self.channel.receive_guest(deadline) {
            Ok(message) => message,
            Err(failure) => {
                return Err(channel_failure(
                    &mut self.channel,
                    ControlStage::Shutdown,
                    failure,
                ));
            }
        };
        match response {
            GuestMessage::ShutdownAck {
                operation: response_operation,
            } if response_operation == operation => Ok(()),
            GuestMessage::ShutdownAck { .. } => Err(self
                .channel
                .fail(ControlStage::Shutdown, ControlFailureClass::Protocol)),
            _ => Err(self
                .channel
                .fail(ControlStage::Shutdown, ControlFailureClass::Lifecycle)),
        }
    }
}

fn message_operation(message: &GuestMessage) -> OperationId {
    match message {
        GuestMessage::RepairComplete { operation }
        | GuestMessage::Stdout { operation, .. }
        | GuestMessage::Stderr { operation, .. }
        | GuestMessage::Terminal { operation, .. }
        | GuestMessage::FileOutcome { operation, .. }
        | GuestMessage::PtyOutcome { operation, .. }
        | GuestMessage::ShutdownAck { operation } => *operation,
    }
}
