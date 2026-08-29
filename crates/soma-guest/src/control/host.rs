use crate::{
    DeliveredHostLaunchMaterial, GuestCommand, GuestMessage, HostMessage, OperationId,
    ResponderPublicKey, TerminalStatus,
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

/// Host owner of one authenticated pre-repair guest-control session.
pub struct HostControl<I: HostControlIo> {
    channel: AuthChannel<I>,
    launch_operation: OperationId,
    operations: OperationLedger,
}

/// Host owner after authenticated repair, repair commit, and the fixed self-probe succeed.
pub struct RepairedHostControl<I: HostControlIo> {
    channel: AuthChannel<I>,
    operations: OperationLedger,
}

impl<I: HostControlIo> HostControl<I> {
    /// Completes both handshake messages over the owned byte transport.
    ///
    /// # Errors
    ///
    /// Returns a redacted Handshake error after poisoning the transport exactly once.
    pub fn connect(
        material: DeliveredHostLaunchMaterial,
        responder: &ResponderPublicKey,
        io: I,
    ) -> Result<Self, ControlError> {
        let (channel, launch_operation) = host_connect::connect(material, responder, io)?;
        Ok(Self {
            channel,
            launch_operation,
            operations: OperationLedger::new(launch_operation),
        })
    }

    /// Commits authenticated repair and requires the fixed zero-output self-probe to succeed.
    ///
    /// # Errors
    ///
    /// Returns a redacted Repair or Probe error after poisoning the transport exactly once.
    pub fn prepare_and_probe(mut self) -> Result<RepairedHostControl<I>, ControlError> {
        let repair_deadline = deadline::repair();
        let request = HostMessage::prepare_and_probe(self.launch_operation);
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
        self.finish_probe()
    }

    fn finish_probe(mut self) -> Result<RepairedHostControl<I>, ControlError> {
        let deadline = deadline::probe();
        let accounting = OutputAccounting::new(GuestCommand::readiness_probe().output_bytes());
        let terminal = match self.channel.receive_guest(deadline) {
            Ok(message) => message,
            Err(failure) => {
                return Err(channel_failure(
                    &mut self.channel,
                    ControlStage::Probe,
                    failure,
                ));
            }
        };
        let report = match terminal {
            GuestMessage::Terminal { operation, report } if operation == self.launch_operation => {
                report
            }
            GuestMessage::Terminal { .. } => {
                return Err(self
                    .channel
                    .fail(ControlStage::Probe, ControlFailureClass::Protocol));
            }
            GuestMessage::Stdout { .. } | GuestMessage::Stderr { .. } => {
                return Err(self
                    .channel
                    .fail(ControlStage::Probe, ControlFailureClass::Accounting));
            }
            _ => {
                return Err(self
                    .channel
                    .fail(ControlStage::Probe, ControlFailureClass::Lifecycle));
            }
        };
        if accounting.validate(report).is_err() || report.status() != TerminalStatus::Exited(0) {
            return Err(self
                .channel
                .fail(ControlStage::Probe, ControlFailureClass::Accounting));
        }
        Ok(RepairedHostControl {
            channel: self.channel,
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
                GuestMessage::RepairComplete { .. } | GuestMessage::ShutdownAck { .. } => {
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
        | GuestMessage::ShutdownAck { operation } => *operation,
    }
}
