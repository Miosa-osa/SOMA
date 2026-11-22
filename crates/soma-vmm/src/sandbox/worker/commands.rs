//! Serving bounded commands to the owner of one repaired session.
//!
//! Reaching Ready and answering commands are different jobs: one happens once and decides
//! whether the sandbox exists at all, the other repeats until the owner is done with it. Keeping
//! them apart is what lets the two launch paths share this half without sharing the other.

use std::sync::mpsc::{Receiver, Sender};

use soma_guest::{GuestApi, GuestCommand, OperationId, RepairedHostControl};
use soma_kvm::head_reads::HeadSpan;
use soma_kvm::x86_64::{Milestone, SandboxMachine};

use super::super::identity::fresh16;
use super::super::io::HostIo;
use super::super::memory;
use super::super::session::{Completed, Request, Response, SessionError};
use super::Ending;
use super::{files, pty};

/// Announces Ready and serves bounded commands until the owner shuts the sandbox down.
///
/// It reports how the loop ended, because the two endings finish the machine differently: a
/// guest that acknowledged shutdown is left the grace to leave `KVM_RUN` on its own, and an
/// aborted one is not given any.
pub(super) fn serve_commands(
    machine: &SandboxMachine,
    mut repaired: RepairedHostControl<HostIo<'_>>,
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
    head_reads: &[HeadSpan],
    suspendable: bool,
) -> Result<Ending, SessionError> {
    responses
        .send(Response::Ready {
            head_reads: head_reads.to_vec(),
        })
        .map_err(|_| SessionError::Gone)?;

    // What the guest answered to the API level probe, learned lazily by the first level 2 file
    // operation or the first command with a long argument, and kept for the life of this session.
    // A guest that cannot decode a command is answered `ExecuteRefused` with nothing sent, so the
    // sandbox keeps answering every command it can.
    let mut guest_api: Option<GuestApi> = None;

    // A closed request channel is an ordinary end: the owner dropped the session, so the guest is
    // shut down exactly as an explicit shutdown would.
    while let Ok(request) = requests.recv() {
        if let Request::Execute(command) | Request::ExecuteStreaming(command) = &request {
            let (next, admitted) = admit(repaired, command, &mut guest_api, responses)?;
            repaired = next;
            if !admitted {
                continue;
            }
        }
        match request {
            // The link is already open by the time commands are served; answering a repeated
            // request keeps the owner's exchange idempotent rather than deadlocking it.
            Request::RaiseLink => {
                machine.set_network_link(true);
                // The ranges travel with both answers, because a launch whose activation is
                // deferred sees only this one.
                responses
                    .send(Response::Ready {
                        head_reads: head_reads.to_vec(),
                    })
                    .map_err(|_| SessionError::Gone)?;
            }
            Request::Execute(command) => {
                let operation = OperationId::new(fresh16()).map_err(|_| SessionError::Execute)?;
                let (next, outcome) = repaired
                    .execute(operation, command)
                    .map_err(|_| SessionError::Execute)?;
                repaired = next;
                machine.mark(Milestone::Execute);
                responses
                    .send(Response::Executed(Box::new(Completed {
                        status: outcome.status(),
                        stdout: outcome.stdout().to_vec(),
                        stderr: outcome.stderr().to_vec(),
                    })))
                    .map_err(|_| SessionError::Gone)?;
            }
            Request::ExecuteStreaming(command) => {
                repaired = stream_command(machine, repaired, command, responses)?;
            }
            Request::SetRequestedMemory(bytes) => {
                let outcome = memory::apply(&memory::device_of(machine), bytes);
                responses
                    .send(Response::Memory(outcome))
                    .map_err(|_| SessionError::Gone)?;
            }
            Request::GetMemory => {
                let level = memory::level(&memory::device_of(machine));
                responses
                    .send(Response::Memory(Ok(level)))
                    .map_err(|_| SessionError::Gone)?;
            }
            Request::File(operation) => {
                let (next, answer) =
                    files::perform(repaired, &operation, soma::MAX_FILE_BYTES, &mut guest_api)?;
                repaired = next;
                responses
                    .send(Response::FileAnswered(Box::new(answer)))
                    .map_err(|_| SessionError::Gone)?;
            }
            Request::Pty(operation) => {
                let (next, answer) = pty::perform(repaired, &operation)?;
                repaired = next;
                responses
                    .send(Response::PtyAnswered(Box::new(answer)))
                    .map_err(|_| SessionError::Gone)?;
            }
            // A machine that is already serving an Instance cannot be assigned another. The
            // pool only ever sends this to a parked sterile worker, so reaching it here means
            // the session is being addressed by something that does not own it.
            Request::Assign(_) => return Err(SessionError::Execute),
            Request::Shutdown => break,
            Request::Suspend(target) => {
                match suspend(repaired, target, suspendable, &mut guest_api, responses)? {
                    SuspendStep::Refused(untouched) => repaired = *untouched,
                    SuspendStep::Suspended(ending) => return Ok(ending),
                }
            }
            // The guest is not told. The session is abandoned with the machine, which the
            // caller finishes without waiting for a guest that was never asked to leave.
            Request::Abort => return Ok(Ending::Aborted),
        }
    }
    let operation = OperationId::new(fresh16()).map_err(|_| SessionError::Execute)?;
    repaired
        .shutdown(operation)
        .map_err(|_| SessionError::Gone)?;
    machine.mark(Milestone::Shutdown);
    Ok(Ending::ShutDown)
}

/// Whether this session may send `command`, answering the owner `ExecuteRefused` when it may not.
///
/// A command whose arguments every guest decodes is admitted without a word. One with an argument
/// longer than a guest below API level 3 decodes is admitted only once the guest answered level 3
/// (ADR 0051, amended); otherwise it is refused with nothing sent and the session is untouched.
fn admit<'a>(
    repaired: RepairedHostControl<HostIo<'a>>,
    command: &GuestCommand,
    guest_api: &mut Option<GuestApi>,
    responses: &Sender<Response>,
) -> Result<(RepairedHostControl<HostIo<'a>>, bool), SessionError> {
    let (repaired, admitted) = repaired
        .admits(command, guest_api, || {
            files::nonce_text(&fresh16()).into_bytes()
        })
        .map_err(|_| SessionError::Execute)?;
    if !admitted {
        responses
            .send(Response::ExecuteRefused)
            .map_err(|_| SessionError::Gone)?;
    }
    Ok((repaired, admitted))
}

/// Runs one command and hands each output chunk to the owner as it arrives, then the result.
fn stream_command<'a>(
    machine: &SandboxMachine,
    repaired: RepairedHostControl<HostIo<'a>>,
    command: GuestCommand,
    responses: &Sender<Response>,
) -> Result<RepairedHostControl<HostIo<'a>>, SessionError> {
    let operation = OperationId::new(fresh16()).map_err(|_| SessionError::Execute)?;
    // A closed response channel cannot abort the guest exchange half way, so the loss is
    // remembered and reported once the exchange has ended cleanly.
    let mut owner_gone = false;
    let (repaired, outcome) = repaired
        .execute_streaming(operation, command, |source, bytes| {
            if responses
                .send(Response::Output(source, bytes.to_vec()))
                .is_err()
            {
                owner_gone = true;
            }
        })
        .map_err(|_| SessionError::Execute)?;
    machine.mark(Milestone::Execute);
    if owner_gone {
        return Err(SessionError::Gone);
    }
    responses
        .send(Response::Executed(Box::new(Completed {
            status: outcome.status(),
            stdout: outcome.stdout().to_vec(),
            stderr: outcome.stderr().to_vec(),
        })))
        .map_err(|_| SessionError::Gone)?;
    Ok(repaired)
}

/// What one Suspend request left: the untouched session, or the ending that leaves the loop.
enum SuspendStep<'a> {
    Refused(Box<RepairedHostControl<HostIo<'a>>>),
    Suspended(Ending),
}

/// Suspends the session, or refuses with nothing sent and hands the untouched session back.
///
/// A guest below API level 4 would refuse Suspend and poison its session, and a cold-booted
/// machine has no snapshot to resume into, so either is answered `SuspendRefused` instead.
fn suspend<'a>(
    repaired: RepairedHostControl<HostIo<'a>>,
    target: Box<super::super::session::SuspendTarget>,
    suspendable: bool,
    guest_api: &mut Option<GuestApi>,
    responses: &Sender<Response>,
) -> Result<SuspendStep<'a>, SessionError> {
    let (repaired, level) = if suspendable {
        let (next, level) = repaired
            .ensure_guest_api(guest_api, || files::nonce_text(&fresh16()).into_bytes())
            .map_err(|_| SessionError::Execute)?;
        (next, Some(level))
    } else {
        (repaired, None)
    };
    if !level.is_some_and(GuestApi::has_suspend) {
        responses
            .send(Response::SuspendRefused)
            .map_err(|_| SessionError::Gone)?;
        return Ok(SuspendStep::Refused(Box::new(repaired)));
    }
    let operation = OperationId::new(fresh16()).map_err(|_| SessionError::Execute)?;
    repaired
        .suspend(operation)
        .map_err(|_| SessionError::Suspend)?;
    Ok(SuspendStep::Suspended(Ending::Suspended(target)))
}
