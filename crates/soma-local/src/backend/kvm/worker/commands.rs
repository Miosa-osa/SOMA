//! Serving bounded commands to the owner of one repaired session.
//!
//! Reaching Ready and answering commands are different jobs: one happens once and decides
//! whether the sandbox exists at all, the other repeats until the owner is done with it. Keeping
//! them apart is what lets the two launch paths share this half without sharing the other.

use std::sync::mpsc::{Receiver, Sender};

use soma_guest::{OperationId, RepairedHostControl};
use soma_kvm::x86_64::{Milestone, SandboxMachine};

use super::super::io::HostIo;
use super::super::session::{Completed, Request, Response, SessionError};

/// Announces Ready and serves bounded commands until the owner shuts the sandbox down.
pub(super) fn serve_commands(
    machine: &SandboxMachine,
    mut repaired: RepairedHostControl<HostIo<'_>>,
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) -> Result<(), SessionError> {
    responses
        .send(Response::Ready)
        .map_err(|_| SessionError::Gone)?;

    // A closed request channel is an ordinary end: the owner dropped the session, so the guest is
    // shut down exactly as an explicit shutdown would.
    while let Ok(request) = requests.recv() {
        match request {
            // The link is already open by the time commands are served; answering a repeated
            // request keeps the owner's exchange idempotent rather than deadlocking it.
            Request::RaiseLink => {
                machine.set_network_link(true);
                responses
                    .send(Response::Ready)
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
            Request::Shutdown => break,
        }
    }
    let operation = OperationId::new(fresh16()).map_err(|_| SessionError::Execute)?;
    repaired
        .shutdown(operation)
        .map_err(|_| SessionError::Gone)?;
    machine.mark(Milestone::Shutdown);
    Ok(())
}

/// Sixteen fresh bytes for one operation identity.
fn fresh16() -> [u8; 16] {
    use std::io::Read as _;
    let mut bytes = [0_u8; 16];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        let _ignored = file.read_exact(&mut bytes);
    }
    bytes
}
