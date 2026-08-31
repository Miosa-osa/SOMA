//! Authenticated request loop: the repair report, bounded Execute exchanges, and Shutdown.
//!
//! The protocol crate owns the Noise state, framing, accounting, and poisoning; this module
//! only sequences the executor and the typestate controller around it.

use std::time::{Duration, Instant};

use soma_guest::{
    ControlError, ControlIo, GuestCommand, GuestControl, GuestRequest, TerminalStatus,
};

use crate::executor::{self, ExecutorFault, OutputSink, SinkFault};
use crate::filesystem;
use crate::pty::Terminal;
use crate::repair::{Controller, Fault, Poisoned, Ready};
use crate::shutdown;
use crate::timings::{self, Step};

/// Longest idle wait for the next authenticated request before the guest fails closed.
pub const IDLE_CEILING: Duration = Duration::from_hours(24);
/// Delivery grace added to a command timeout for its output and terminal report.
pub const DELIVERY_GRACE: Duration = Duration::from_millis(500);
/// Budget for the single `RepairComplete` report.
pub const REPORT_BUDGET: Duration = Duration::from_secs(5);
/// Delivery budget for the single outcome that answers a terminal request.
///
/// Like the filesystem budget, it starts once the work is done, so it bounds delivery of one
/// bounded record and not the wait a read carried.
pub const TERMINAL_BUDGET: Duration = Duration::from_secs(5);
/// Delivery budget for the single outcome that answers a filesystem request.
///
/// The budget starts once the operation is done, so it bounds delivery of one bounded record and
/// not the work behind it; a removal of a large tree takes as long as it takes.
pub const OUTCOME_BUDGET: Duration = Duration::from_secs(5);

const AGENT_FAILED_INVOCATION: i32 = 2;
const AGENT_FAILED_PROCESS_GROUP: i32 = 3;

type SendChunk<I> = fn(GuestControl<I>, Vec<u8>, Instant) -> Result<GuestControl<I>, ControlError>;

struct ControlSink<I: ControlIo> {
    control: Option<GuestControl<I>>,
    deadline: Instant,
}

impl<I: ControlIo> ControlSink<I> {
    /// Copies one already admitted chunk into the record the protocol crate owns.
    ///
    /// The copy is bounded by the executor's fixed chunk size and is released as soon as the
    /// record is sealed, so no queue forms behind a slow or hostile peer.
    fn send(&mut self, bytes: &[u8], send: SendChunk<I>) -> Result<(), SinkFault> {
        let control = self.control.take().ok_or(SinkFault)?;
        match send(control, bytes.to_vec(), self.deadline) {
            Ok(control) => {
                self.control = Some(control);
                Ok(())
            }
            Err(_) => Err(SinkFault),
        }
    }
}

impl<I: ControlIo> OutputSink for ControlSink<I> {
    fn stdout(&mut self, bytes: &[u8]) -> Result<(), SinkFault> {
        self.send(bytes, GuestControl::stdout)
    }

    fn stderr(&mut self, bytes: &[u8]) -> Result<(), SinkFault> {
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
    let control = timings::measure(Step::TerminalReport, || control.terminal(status, deadline))
        .map_err(|_| Fault::Control)?;
    Ok((control, status))
}

/// Receives `Prepare` and reports repair complete under the authenticated session.
///
/// Nothing is executed here. The report is the readiness evidence: it is authenticated with
/// this Instance's own session key, so it already proves the guest that answers is the guest
/// this launch repaired, and running a command to say so again only costs a process.
///
/// # Errors
///
/// Returns the fault that must poison the guest.
pub fn prepare<I: ControlIo>(control: GuestControl<I>) -> Result<GuestControl<I>, Fault> {
    let (control, request) = timings::measure(Step::RequestWait, || {
        control.next_request(Instant::now() + IDLE_CEILING)
    })
    .map_err(|_| Fault::Control)?;
    if !matches!(request, GuestRequest::Prepare { .. }) {
        return Err(Fault::Control);
    }
    timings::measure(Step::RepairReport, || {
        control.repair_complete(Instant::now() + REPORT_BUDGET)
    })
    .map_err(|_| Fault::Control)
}

/// Serves authenticated requests until shutdown powers the machine off or a fault poisons it.
pub fn serve<I: ControlIo>(
    controller: Controller<Ready>,
    control: GuestControl<I>,
) -> Controller<Poisoned> {
    let mut controller = controller;
    let mut control = control;
    // The one terminal outlives every request, because a session a caller opened has to still
    // be there when the caller's next request arrives. It is created here rather than lazily so
    // that the loop owns it for exactly as long as it owns the session.
    let mut terminal = Terminal::new();
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
            GuestRequest::File { request, .. } => {
                // A filesystem request is not a command, so it takes no lifecycle transition:
                // the controller counts commands, and counting these among them would make the
                // ledger describe work that never ran a process.
                let outcome = filesystem::perform(&request);
                match next.file_outcome(&outcome, Instant::now() + OUTCOME_BUDGET) {
                    Ok(after) => control = after,
                    Err(_) => return controller.poison(Fault::Control),
                }
            }
            GuestRequest::Pty { request, .. } => {
                // A terminal request is not a command either, so it takes no lifecycle
                // transition, for the same reason a filesystem request takes none.
                let outcome = terminal.perform(&request);
                match next.pty_outcome(&outcome, Instant::now() + TERMINAL_BUDGET) {
                    Ok(after) => control = after,
                    Err(_) => return controller.poison(Fault::Control),
                }
            }
            GuestRequest::Shutdown { .. } => match controller.stop(Ok(())) {
                Ok((_stopping, ())) => shutdown::perform(next),
                Err(poisoned) => return poisoned,
            },
            GuestRequest::Prepare { .. } => return controller.poison(Fault::Control),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_are_failure_ceilings_not_latency_targets() {
        assert_eq!(IDLE_CEILING, Duration::from_hours(24));
        assert_eq!(DELIVERY_GRACE, Duration::from_millis(500));
        assert!(REPORT_BUDGET <= Duration::from_secs(5));
        assert!(OUTCOME_BUDGET <= Duration::from_secs(5));
        assert!(TERMINAL_BUDGET <= Duration::from_secs(5));
    }
}
