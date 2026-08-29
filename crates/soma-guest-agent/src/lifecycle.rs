//! Authenticated request loop: the readiness probe, bounded Execute exchanges, and Shutdown.
//!
//! The protocol crate owns the Noise state, framing, accounting, and poisoning; this module
//! only sequences the executor and the typestate controller around it.

use std::time::{Duration, Instant};

use soma_guest::{
    ControlError, ControlIo, GuestCommand, GuestControl, GuestRequest, TerminalStatus,
};

use crate::executor::{self, ExecutorFault, OutputSink, SinkFault};
use crate::repair::{Controller, Fault, Poisoned, Ready};
use crate::shutdown;

/// Longest idle wait for the next authenticated request before the guest fails closed.
pub const IDLE_CEILING: Duration = Duration::from_hours(24);
/// Delivery grace added to a command timeout for its output and terminal report.
pub const DELIVERY_GRACE: Duration = Duration::from_millis(500);
/// Budget for the single `RepairComplete` report.
pub const REPORT_BUDGET: Duration = Duration::from_secs(5);

const PROBE_PROGRAM: &[u8] = b"/proc/self/exe";
/// The reserved self-check argument of the fixed version 1 readiness probe.
pub const PROBE_ARGUMENT: &[u8] = b"--soma-ready-probe-v1";
const PROBE_TIMEOUT_MILLIS: u32 = 1_000;
const PROBE_OUTPUT_BYTES: u64 = 1;
const AGENT_FAILED_INVOCATION: i32 = 2;
const AGENT_FAILED_PROCESS_GROUP: i32 = 3;

/// Returns the fixed readiness self-probe command executed through the production executor.
#[must_use]
pub fn readiness_probe() -> GuestCommand {
    GuestCommand::new(
        PROBE_PROGRAM.to_vec(),
        vec![PROBE_ARGUMENT.to_vec()],
        PROBE_TIMEOUT_MILLIS,
        PROBE_OUTPUT_BYTES,
    )
    .expect("the fixed probe satisfies the wire contract")
}

type SendChunk<I> = fn(GuestControl<I>, Vec<u8>, Instant) -> Result<GuestControl<I>, ControlError>;

struct ControlSink<I: ControlIo> {
    control: Option<GuestControl<I>>,
    deadline: Instant,
}

impl<I: ControlIo> ControlSink<I> {
    fn send(&mut self, bytes: Vec<u8>, send: SendChunk<I>) -> Result<(), SinkFault> {
        let control = self.control.take().ok_or(SinkFault)?;
        match send(control, bytes, self.deadline) {
            Ok(control) => {
                self.control = Some(control);
                Ok(())
            }
            Err(_) => Err(SinkFault),
        }
    }
}

impl<I: ControlIo> OutputSink for ControlSink<I> {
    fn stdout(&mut self, bytes: Vec<u8>) -> Result<(), SinkFault> {
        self.send(bytes, GuestControl::stdout)
    }

    fn stderr(&mut self, bytes: Vec<u8>) -> Result<(), SinkFault> {
        self.send(bytes, GuestControl::stderr)
    }
}

fn run_command<I: ControlIo>(
    control: GuestControl<I>,
    command: &GuestCommand,
) -> Result<(GuestControl<I>, TerminalStatus), Fault> {
    let deadline = Instant::now()
        + Duration::from_millis(u64::from(command.timeout_millis()))
        + DELIVERY_GRACE;
    let mut sink = ControlSink {
        control: Some(control),
        deadline,
    };
    let completion = executor::execute(command, &mut sink);
    let control = sink.control.ok_or(Fault::Control)?;
    let status = match completion {
        Ok(completion) => completion.status,
        Err(ExecutorFault::Sink) => return Err(Fault::Control),
        Err(ExecutorFault::Invocation(_)) => TerminalStatus::AgentFailed(AGENT_FAILED_INVOCATION),
        Err(ExecutorFault::ProcessGroup) => TerminalStatus::AgentFailed(AGENT_FAILED_PROCESS_GROUP),
    };
    let control = control
        .terminal(status, deadline)
        .map_err(|_| Fault::Control)?;
    Ok((control, status))
}

/// Receives `PrepareAndProbe`, reports repair, and runs the fixed self-probe to completion.
///
/// # Errors
///
/// Returns the fault that must poison the guest.
pub fn probe<I: ControlIo>(control: GuestControl<I>) -> Result<GuestControl<I>, Fault> {
    let (control, request) = control
        .next_request(Instant::now() + IDLE_CEILING)
        .map_err(|_| Fault::Control)?;
    if !matches!(request, GuestRequest::PrepareAndProbe { .. }) {
        return Err(Fault::Control);
    }
    let control = control
        .repair_complete(Instant::now() + REPORT_BUDGET)
        .map_err(|_| Fault::Control)?;
    let (control, status) = run_command(control, &readiness_probe())?;
    if status != TerminalStatus::Exited(0) {
        return Err(Fault::Executor);
    }
    Ok(control)
}

/// Serves authenticated requests until shutdown powers the machine off or a fault poisons it.
pub fn serve<I: ControlIo>(
    controller: Controller<Ready>,
    control: GuestControl<I>,
) -> Controller<Poisoned> {
    let mut controller = controller;
    let mut control = control;
    loop {
        let Ok((next, request)) = control.next_request(Instant::now() + IDLE_CEILING) else {
            return controller.poison(Fault::Control);
        };
        match request {
            GuestRequest::Execute { command, .. } => {
                let running = match controller.run(Ok(())) {
                    Ok((running, ())) => running,
                    Err(poisoned) => return poisoned,
                };
                match run_command(next, &command) {
                    Ok((after, _)) => {
                        control = after;
                        controller = match running.finish(Ok(())) {
                            Ok((ready, ())) => ready,
                            Err(poisoned) => return poisoned,
                        };
                    }
                    Err(fault) => return running.poison(fault),
                }
            }
            GuestRequest::Shutdown { .. } => match controller.stop(Ok(())) {
                Ok((_stopping, ())) => shutdown::perform(next),
                Err(poisoned) => return poisoned,
            },
            GuestRequest::PrepareAndProbe { .. } => return controller.poison(Fault::Control),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_readiness_probe_matches_the_fixed_version_one_contract() {
        let probe = readiness_probe();

        assert_eq!(probe.program(), b"/proc/self/exe");
        assert_eq!(probe.arguments().len(), 1);
        assert_eq!(&*probe.arguments()[0], b"--soma-ready-probe-v1");
        assert_eq!(probe.timeout_millis(), 1_000);
        assert_eq!(probe.output_bytes(), 1);
    }

    #[test]
    fn budgets_are_failure_ceilings_not_latency_targets() {
        assert_eq!(IDLE_CEILING, Duration::from_hours(24));
        assert_eq!(DELIVERY_GRACE, Duration::from_millis(500));
        assert!(REPORT_BUDGET <= Duration::from_secs(5));
    }
}
