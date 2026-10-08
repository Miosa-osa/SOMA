//! How a sandbox that reached Ready stopped serving, and how it is released.
//!
//! The two endings are not the same promise. A guest that acknowledged shutdown is waited on to
//! leave `KVM_RUN` by itself, while an aborted one was never asked and is kicked out at once.
//! The difference is decided here, because this is the only place that knows both the outcome
//! and the machine it must finish.

use std::sync::mpsc::Sender;
use std::time::Duration;

use soma_kvm::x86_64::SandboxMachine;

use super::super::session::{EXIT_GRACE, Response, SessionError};

/// How a sandbox that reached Ready stopped serving its owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ending {
    /// The guest acknowledged a shutdown and is leaving `KVM_RUN` on its own.
    ShutDown,
    /// The owner abandoned the guest without asking it, so nothing waits for it to leave.
    Aborted,
}

/// Finishes the machine and reports its evidence, or the failure that ended it.
///
/// An aborted machine is halted and its owner answered before it is finished, so the owner
/// does not wait for the release. An aborted guest is given no exit grace: finishing with a zero deadline kicks the vCPU out of
/// `KVM_RUN` at once, where a guest that acknowledged shutdown is waited on to leave by itself.
pub fn report(
    mut sandbox: SandboxMachine,
    outcome: Result<Ending, SessionError>,
    responses: &Sender<Response>,
    instance: [u8; 16],
) {
    if outcome == Ok(Ending::Aborted) {
        // The owner is answered once the guest can no longer run, and the release it would
        // otherwise wait on, which is closing the VM and returning its memory, happens after.
        sandbox.halt();
        let _ignored = responses.send(Response::Halted);
        drop(sandbox.finish(exit_grace(outcome)));
        return;
    }
    let evidence = sandbox.finish(exit_grace(outcome));
    match outcome {
        Ok(_) => {
            let _ignored = responses.send(Response::Finished(Box::new(evidence)));
        }
        Err(error) => {
            // A failed sandbox never reaches cleanup, so this is the only chance to keep what it
            // recorded. The caller still receives the same typed failure.
            super::super::timeline::dump_failure(&hex(instance), &evidence, &format!("{error:?}"));
            let _ignored = responses.send(Response::Failed(error));
        }
    }
}

/// How long a finishing machine waits for its vCPU to leave `KVM_RUN` on its own.
///
/// Only a guest that was asked to leave is waited on. An aborted one was never asked, so waiting
/// would spend the whole grace on a guest idling in its command loop before the kick ended it.
const fn exit_grace(outcome: Result<Ending, SessionError>) -> Duration {
    match outcome {
        Ok(Ending::Aborted) => Duration::ZERO,
        Ok(Ending::ShutDown) | Err(_) => EXIT_GRACE,
    }
}

/// The Instance identity as the lowercase hexadecimal the receipt reports.
fn hex(instance: [u8; 16]) -> String {
    use std::fmt::Write as _;
    instance
        .iter()
        .fold(String::with_capacity(32), |mut out, byte| {
            let _ignored = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{EXIT_GRACE, Ending, SessionError, exit_grace};

    #[test]
    fn an_aborted_machine_is_kicked_at_once() {
        assert_eq!(exit_grace(Ok(Ending::Aborted)), Duration::ZERO);
    }

    #[test]
    fn a_guest_asked_to_leave_and_a_failed_session_keep_the_exit_grace() {
        assert_eq!(exit_grace(Ok(Ending::ShutDown)), EXIT_GRACE);
        assert_eq!(exit_grace(Err(SessionError::Execute)), EXIT_GRACE);
    }
}
