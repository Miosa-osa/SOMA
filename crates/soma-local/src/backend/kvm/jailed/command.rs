//! Running one bounded command inside a jailed machine.
//!
//! The command crosses as one request packet and its result comes back in two parts: the
//! terminal status with the byte counts of each stream, and then those bytes read out of the
//! receipt the worker retains, one bounded window at a time. A single packet cannot carry
//! sixteen mebibytes, and a worker that answered with as much as fitted would be reporting a
//! different command than the one that ran.

use std::time::{Duration, Instant};

use soma::BackendFailureKind;
use soma_guest::GuestCommand;
use soma_vmm::control::{
    MAX_OUTPUT_WINDOW_BYTES, MAX_REQUEST_BYTES, Outcome, OutputStream, OutputWindow, Request,
};
use soma_vmm::sandbox::Completed;
use soma_vmm::{
    Argument, Execute, ExecuteContext, ExecutionLimits, OperationId, OutputBytes, Program,
    TimeoutMillis,
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
    /// that arrives late would be read as the next command's. A refusal the worker itself
    /// stated is not uncertain: it answered this request and is waiting for the next one, so
    /// the handle keeps the machine and the caller keeps the sandbox.
    pub(in crate::backend::kvm) fn execute(
        &mut self,
        command: &GuestCommand,
    ) -> Result<Completed, BackendFailureKind> {
        if self.poisoned {
            return Err(BackendFailureKind::Unavailable);
        }
        let operation =
            OperationId::new(fresh16()).map_err(|_| BackendFailureKind::WorkloadRejected)?;
        let request = self.execute_request(operation, command, false)?;
        let within = Duration::from_millis(u64::from(command.timeout_millis())) + COMMAND_SLACK;
        let answered = self.control.ask(&request, within);
        let (status, stdout_bytes, stderr_bytes) = match answered {
            Ok(reply) => match outcome::executed(&reply) {
                Ok(completed) => completed,
                Err(kind) if outcome::is_refusal(kind) => return Err(kind),
                Err(kind) => return Err(self.poison(kind)),
            },
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

    /// Runs one bounded command and hands each output chunk to `sink` while it runs.
    ///
    /// The worker answers a streaming request with any number of chunk packets and then the one
    /// final reply, so the output has already crossed by the time the command ends and nothing
    /// is read back by window. The chunks are checked against the byte counts the final reply
    /// names, and against the command's own output allowance as they arrive, so a worker that
    /// sends more or fewer bytes than it reports poisons the handle exactly as an uncertain
    /// buffered command does.
    ///
    /// A worker that predates the request refuses it with one `rejected` packet before sending
    /// any chunk, which is read as the command having been turned down.
    pub(in crate::backend::kvm) fn execute_streaming(
        &mut self,
        command: &GuestCommand,
        sink: soma::OutputSink<'_>,
    ) -> Result<Completed, BackendFailureKind> {
        if self.poisoned {
            return Err(BackendFailureKind::Unavailable);
        }
        let operation =
            OperationId::new(fresh16()).map_err(|_| BackendFailureKind::WorkloadRejected)?;
        let request = self.execute_request(operation, command, true)?;
        let end = Instant::now()
            + Duration::from_millis(u64::from(command.timeout_millis()))
            + COMMAND_SLACK;
        self.control
            .tell(&request)
            .map_err(|kind| self.poison(kind))?;
        let allowance = command.output_bytes();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let (status, stdout_bytes, stderr_bytes) = loop {
            let remaining = end.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.poison(BackendFailureKind::Unavailable));
            }
            let text = self
                .control
                .receive(remaining)
                .map_err(|kind| self.poison(kind))?;
            match Outcome::decode(&text) {
                Ok(Outcome::Chunk(stream, bytes)) => {
                    let total = (stdout.len() + stderr.len() + bytes.len()) as u64;
                    if total > allowance {
                        return Err(self.poison(BackendFailureKind::GuestFailure));
                    }
                    let (channel, retained) = match stream {
                        OutputStream::Stdout => (soma::OutputChannel::Stdout, &mut stdout),
                        OutputStream::Stderr => (soma::OutputChannel::Stderr, &mut stderr),
                    };
                    sink(channel, &bytes);
                    retained.extend_from_slice(&bytes);
                }
                Ok(reply) => match outcome::executed(&reply) {
                    Ok(completed) => break completed,
                    Err(kind) if outcome::is_refusal(kind) => return Err(kind),
                    Err(kind) => return Err(self.poison(kind)),
                },
                Err(_) => return Err(self.poison(BackendFailureKind::Unavailable)),
            }
        };
        if stdout.len() as u64 != stdout_bytes || stderr.len() as u64 != stderr_bytes {
            return Err(self.poison(BackendFailureKind::GuestFailure));
        }
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
        streaming: bool,
    ) -> Result<Request, BackendFailureKind> {
        let rejected = |_| BackendFailureKind::WorkloadRejected;
        let limits = ExecutionLimits::new(
            TimeoutMillis::new(command.timeout_millis()).map_err(rejected)?,
            OutputBytes::new(command.output_bytes()).map_err(rejected)?,
        );
        let request = Execute::new(
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
        .map(|execute| {
            let execute = execute.with_context(ExecuteContext {
                environment: command
                    .environment()
                    .iter()
                    .map(|(name, value)| (name.to_vec(), value.to_vec()))
                    .collect(),
                working_directory: command.working_directory().map(<[u8]>::to_vec),
                user: command.user().map(<[u8]>::to_vec),
                stdin: command.stdin().to_vec(),
            });
            if streaming {
                Request::ExecuteStreaming(execute)
            } else {
                Request::Execute(execute)
            }
        })
        .map_err(rejected)?;
        // One control packet is one datagram, and a datagram longer than the worker's receive
        // buffer is truncated by the kernel rather than refused. The program and every argument
        // travel hex encoded, so a command the packet cannot carry would arrive cut short and be
        // decoded as a shorter command than the caller asked for. The length is therefore
        // decided here, while the request is still this process's to turn down and no worker has
        // been addressed at all.
        if request.encode().len() > MAX_REQUEST_BYTES {
            return Err(BackendFailureKind::WorkloadRejected);
        }
        Ok(request)
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
