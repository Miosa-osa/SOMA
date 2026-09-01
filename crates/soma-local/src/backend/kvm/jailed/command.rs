//! Running one bounded command inside a jailed machine.
//!
//! The command crosses as one request packet and its result comes back in two parts: the
//! terminal status with the byte counts of each stream, and then those bytes read out of the
//! receipt the worker retains, one bounded window at a time. A single packet cannot carry
//! sixteen mebibytes, and a worker that answered with as much as fitted would be reporting a
//! different command than the one that ran.

use std::time::Duration;

use soma::BackendFailureKind;
use soma_guest::GuestCommand;
use soma_vmm::control::{MAX_OUTPUT_WINDOW_BYTES, OutputStream, OutputWindow, Request};
use soma_vmm::sandbox::Completed;
use soma_vmm::{
    Argument, Execute, ExecutionLimits, OperationId, OutputBytes, Program, TimeoutMillis,
};

use super::outcome;
use super::{ATTESTATION_CEILING, Jailed};
use crate::backend::kvm::identity::fresh16;

/// How much longer than its own deadline one command may take before the worker is gone.
const COMMAND_SLACK: Duration = Duration::from_secs(60);

impl Jailed {
    /// Runs one bounded command inside the jailed machine and reads its output back.
    ///
    /// # Errors
    ///
    /// Returns the typed refusal. Every uncertain outcome poisons the handle, because a reply
    /// that arrives late would be read as the next command's.
    pub(in crate::backend::kvm) fn execute(
        &mut self,
        command: &GuestCommand,
    ) -> Result<Completed, BackendFailureKind> {
        if self.poisoned {
            return Err(BackendFailureKind::Unavailable);
        }
        let operation =
            OperationId::new(fresh16()).map_err(|_| BackendFailureKind::WorkloadRejected)?;
        let request = self.execute_request(operation, command)?;
        let within = Duration::from_millis(u64::from(command.timeout_millis())) + COMMAND_SLACK;
        let answered = self.control.ask(&request, within);
        let (status, stdout_bytes, stderr_bytes) = match answered {
            Ok(reply) => outcome::executed(&reply).map_err(|kind| self.poison(kind))?,
            Err(kind) => return Err(self.poison(kind)),
        };
        let stdout = self
            .read_output(operation, outcome::STDOUT, stdout_bytes)
            .map_err(|kind| self.poison(kind))?;
        let stderr = self
            .read_output(operation, outcome::STDERR, stderr_bytes)
            .map_err(|kind| self.poison(kind))?;
        Ok(Completed {
            status,
            stdout,
            stderr,
        })
    }

    fn execute_request(
        &self,
        operation: OperationId,
        command: &GuestCommand,
    ) -> Result<Request, BackendFailureKind> {
        let rejected = |_| BackendFailureKind::WorkloadRejected;
        let limits = ExecutionLimits::new(
            TimeoutMillis::new(command.timeout_millis()).map_err(rejected)?,
            OutputBytes::new(command.output_bytes()).map_err(rejected)?,
        );
        Execute::new(
            operation,
            self.instance,
            Program::new(command.program().to_vec()).map_err(rejected)?,
            command
                .arguments()
                .iter()
                .map(|argument| Argument::new(argument.to_vec()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(rejected)?,
            limits,
        )
        .map(Request::Execute)
        .map_err(rejected)
    }

    /// Reads one stream of a completed command back, one bounded window at a time.
    fn read_output(
        &self,
        operation: OperationId,
        stream: OutputStream,
        total: u64,
    ) -> Result<Vec<u8>, BackendFailureKind> {
        let mut bytes = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
        while (bytes.len() as u64) < total {
            let offset = bytes.len() as u64;
            let length = (total - offset).min(MAX_OUTPUT_WINDOW_BYTES);
            let window = OutputWindow::new(operation, stream, offset, length)
                .ok_or(BackendFailureKind::Unavailable)?;
            let read = outcome::output(
                self.control
                    .ask(&Request::Output(window), ATTESTATION_CEILING)?,
            )?;
            if read.is_empty() {
                // The worker answered a window inside the length it reported with nothing, so
                // the two sides disagree about what the command produced.
                return Err(BackendFailureKind::GuestFailure);
            }
            bytes.extend_from_slice(&read);
        }
        Ok(bytes)
    }
}
