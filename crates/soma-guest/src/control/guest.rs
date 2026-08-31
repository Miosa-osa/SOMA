use crate::{GuestCommand, GuestMessage, GuestSessionMaterial, HostMessage, TerminalStatus};
use std::time::Instant;

use super::{
    channel::{AuthChannel, channel_failure},
    error::{ControlError, ControlFailureClass, ControlStage},
    exchange::{OutputAccounting, output_chunk},
    guest_connect, guest_idle,
    guest_state::{ActiveExchange, GuestState, active_stage, receive_stage},
    io::ControlIo,
    operation_ledger::OperationLedger,
    request::GuestRequest,
};

/// Guest owner of one authenticated lifecycle and byte transport.
pub struct GuestControl<I: ControlIo> {
    pub(super) channel: AuthChannel<I>,
    pub(super) state: GuestState,
    pub(super) operations: OperationLedger,
}

impl<I: ControlIo> GuestControl<I> {
    /// Authenticates message one and writes message two before the caller deadline.
    ///
    /// # Errors
    ///
    /// Returns a redacted Handshake error after poisoning the transport exactly once.
    pub fn connect(
        material: GuestSessionMaterial,
        io: I,
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        let (channel, launch_operation) = guest_connect::connect(material, io, deadline)?;
        Ok(Self {
            channel,
            state: GuestState::AwaitPrepare(launch_operation),
            operations: OperationLedger::new(launch_operation),
        })
    }

    /// Receives one lifecycle-valid request before the caller's absolute idle deadline.
    ///
    /// # Errors
    ///
    /// Returns a redacted lifecycle, protocol, authentication, or input/output error after poison.
    pub fn next_request(self, deadline: Instant) -> Result<(Self, GuestRequest), ControlError> {
        let Self {
            mut channel,
            state,
            operations,
        } = self;
        if !matches!(
            state,
            GuestState::AwaitPrepare(_) | GuestState::RepairedIdle
        ) {
            return Err(channel.fail(active_stage(&state), ControlFailureClass::Lifecycle));
        }
        let request = match channel.receive_host(deadline) {
            Ok(request) => request,
            Err(failure) => {
                return Err(channel_failure(
                    &mut channel,
                    receive_stage(&state),
                    failure,
                ));
            }
        };
        match (state, request) {
            (GuestState::AwaitPrepare(expected), HostMessage::PrepareAndProbe { operation })
                if operation == expected =>
            {
                let exchange = ActiveExchange {
                    operation,
                    accounting: OutputAccounting::new(
                        GuestCommand::readiness_probe().output_bytes(),
                    ),
                };
                Ok((
                    Self {
                        channel,
                        state: GuestState::ProbeAwaitRepair(exchange),
                        operations,
                    },
                    GuestRequest::PrepareAndProbe { operation },
                ))
            }
            (GuestState::AwaitPrepare(_), HostMessage::PrepareAndProbe { .. }) => {
                Err(channel.fail(ControlStage::Repair, ControlFailureClass::Protocol))
            }
            // Every other message is one this state cannot serve, whether it names repair
            // work that is already done or ordinary work that repair has not admitted yet.
            (GuestState::AwaitPrepare(_), _) => {
                Err(channel.fail(ControlStage::Repair, ControlFailureClass::Lifecycle))
            }
            (GuestState::RepairedIdle, request) => guest_idle::accept(channel, operations, request),
            _ => unreachable!("active states return before receiving"),
        }
    }

    /// Reports Repair completion exactly once before any self-probe result and caller deadline.
    ///
    /// # Errors
    ///
    /// Returns a redacted Repair error after poisoning an illegal or failed report.
    pub fn repair_complete(self, deadline: Instant) -> Result<Self, ControlError> {
        let Self {
            mut channel,
            state,
            operations,
        } = self;
        let exchange = match state {
            GuestState::ProbeAwaitRepair(exchange) => exchange,
            other => return Err(channel.fail(active_stage(&other), ControlFailureClass::Lifecycle)),
        };
        if let Err(failure) =
            channel.send_guest(&GuestMessage::repair_complete(exchange.operation), deadline)
        {
            return Err(channel_failure(&mut channel, ControlStage::Repair, failure));
        }
        Ok(Self {
            channel,
            state: GuestState::ProbeStreaming(exchange),
            operations,
        })
    }

    /// Sends one bounded stdout chunk by the caller deadline and advances accounting.
    ///
    /// # Errors
    ///
    /// Returns a redacted Execute error after poisoning an invalid or failed output report.
    pub fn stdout(self, bytes: Vec<u8>, deadline: Instant) -> Result<Self, ControlError> {
        self.output(bytes, true, deadline)
    }

    /// Sends one bounded stderr chunk by the caller deadline and advances accounting.
    ///
    /// # Errors
    ///
    /// Returns a redacted Execute error after poisoning an invalid or failed output report.
    pub fn stderr(self, bytes: Vec<u8>, deadline: Instant) -> Result<Self, ControlError> {
        self.output(bytes, false, deadline)
    }

    /// Sends the exact counted terminal result by the caller deadline and returns to idle.
    ///
    /// # Errors
    ///
    /// Returns a redacted Probe or Execute error after poisoning an invalid or failed terminal.
    pub fn terminal(self, status: TerminalStatus, deadline: Instant) -> Result<Self, ControlError> {
        let Self {
            mut channel,
            state,
            operations,
        } = self;
        let (exchange, stage, probe) = match state {
            GuestState::ProbeStreaming(exchange) => (exchange, ControlStage::Probe, true),
            GuestState::ExecuteStreaming(exchange) => (exchange, ControlStage::Execute, false),
            other => return Err(channel.fail(active_stage(&other), ControlFailureClass::Lifecycle)),
        };
        let Ok(report) = exchange.accounting.report(status) else {
            return Err(channel.fail(stage, ControlFailureClass::Accounting));
        };
        if probe && status != TerminalStatus::Exited(0) {
            return Err(channel.fail(stage, ControlFailureClass::Accounting));
        }
        if let Err(failure) = channel.send_guest(
            &GuestMessage::terminal(exchange.operation, report),
            deadline,
        ) {
            return Err(channel_failure(&mut channel, stage, failure));
        }
        Ok(Self {
            channel,
            state: GuestState::RepairedIdle,
            operations,
        })
    }

    /// Sends one exact Shutdown acknowledgement by the caller deadline and consumes the owner.
    ///
    /// # Errors
    ///
    /// Returns a redacted Shutdown error after poisoning an illegal or failed acknowledgement.
    pub fn shutdown_ack(self, deadline: Instant) -> Result<(), ControlError> {
        let Self {
            mut channel,
            state,
            operations: _,
        } = self;
        let operation = match state {
            GuestState::ShutdownPending(operation) => operation,
            other => return Err(channel.fail(active_stage(&other), ControlFailureClass::Lifecycle)),
        };
        if let Err(failure) = channel.send_guest(&GuestMessage::shutdown_ack(operation), deadline) {
            return Err(channel_failure(
                &mut channel,
                ControlStage::Shutdown,
                failure,
            ));
        }
        Ok(())
    }

    fn output(self, bytes: Vec<u8>, stdout: bool, deadline: Instant) -> Result<Self, ControlError> {
        let Self {
            mut channel,
            state,
            operations,
        } = self;
        let mut exchange = match state {
            GuestState::ExecuteStreaming(exchange) => exchange,
            GuestState::ProbeStreaming(_) => {
                return Err(channel.fail(ControlStage::Probe, ControlFailureClass::Accounting));
            }
            other => return Err(channel.fail(active_stage(&other), ControlFailureClass::Lifecycle)),
        };
        let Ok(chunk) = output_chunk(bytes) else {
            return Err(channel.fail(ControlStage::Execute, ControlFailureClass::Accounting));
        };
        let accounted = if stdout {
            exchange.accounting.push_stdout(&chunk)
        } else {
            exchange.accounting.push_stderr(&chunk)
        };
        if accounted.is_err() {
            return Err(channel.fail(ControlStage::Execute, ControlFailureClass::Accounting));
        }
        let message = if stdout {
            GuestMessage::stdout(exchange.operation, chunk)
        } else {
            GuestMessage::stderr(exchange.operation, chunk)
        };
        if let Err(failure) = channel.send_guest(&message, deadline) {
            return Err(channel_failure(
                &mut channel,
                ControlStage::Execute,
                failure,
            ));
        }
        Ok(Self {
            channel,
            state: GuestState::ExecuteStreaming(exchange),
            operations,
        })
    }
}
