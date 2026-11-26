//! Waiting for the owner's next request while watching the guest.
//!
//! Between requests the machine is watched for the guest's own end, a shutdown or restart from
//! inside, which nobody would otherwise notice until the next request timed out against a
//! machine that is no longer there.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use soma_kvm::x86_64::{GuestEnd, SandboxMachine};

use super::super::session::Request;

/// How often a sandbox waiting for its next request checks whether the guest ended the machine.
const GUEST_END_POLL: Duration = Duration::from_millis(250);

/// What waiting for the next request found.
pub(super) enum Next {
    Request(Request),
    GuestEnded(GuestEnd),
    Closed,
}

pub(super) fn next(machine: &SandboxMachine, requests: &Receiver<Request>) -> Next {
    loop {
        match requests.recv_timeout(GUEST_END_POLL) {
            Ok(request) => return Next::Request(request),
            Err(RecvTimeoutError::Timeout) => {
                if let Some(end) = machine.guest_end() {
                    return Next::GuestEnded(end);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Next::Closed,
        }
    }
}

/// Whether a paused machine refuses `request`: only pause, resume and the two ends reach it.
pub(super) fn refused_while_paused(machine: &SandboxMachine, request: &Request) -> bool {
    machine.is_paused()
        && !matches!(
            request,
            Request::Pause | Request::Resume | Request::Shutdown | Request::Abort
        )
}
