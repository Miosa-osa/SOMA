//! Opening one Instance's network, once its guest session exists.

use std::sync::mpsc::{Receiver, Sender};

use soma_guest::RepairedHostControl;
use soma_kvm::x86_64::SandboxMachine;

use super::super::io::HostIo;
use super::super::network::PendingActivation;
use super::super::session::{Request, Response, SessionError};

/// Mints the activation capability, waits for the owner to spend it, and raises the link.
///
/// The receipt exists only here, because only a repaired session can mint one; the broker will
/// only accept it from the peer that claimed the assignment, which is the owner of this thread.
/// So the two halves are joined by one exchange: this thread mints and waits, the owner
/// activates, and the link gate opens after the broker has enabled forwarding rather than
/// before it.
pub(super) fn open_network(
    machine: &SandboxMachine,
    repaired: &RepairedHostControl<HostIo<'_>>,
    activation: Option<PendingActivation>,
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) -> Result<(), SessionError> {
    let Some(pending) = activation else {
        return Ok(());
    };
    let receipt = repaired
        .network_activation(&pending.challenge, pending.generation, pending.intent)
        .map_err(|_| SessionError::Network)?;
    responses
        .send(Response::Minted(Box::new(receipt)))
        .map_err(|_| SessionError::Gone)?;
    match requests.recv() {
        Ok(Request::RaiseLink) => {
            machine.set_network_link(true);
            Ok(())
        }
        // The owner ended the session rather than activating, so no traffic may ever flow here.
        _ => Err(SessionError::Gone),
    }
}
