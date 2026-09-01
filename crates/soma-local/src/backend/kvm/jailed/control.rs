//! The supervisor's end of one jailed worker's control socket, in the terms this Backend uses.
//!
//! One packet out is one request and one packet in is one reply. Every exchange carries a
//! deadline, because the process on the other end is one this host may not reach any other way:
//! it has no name, no socket of its own, and no path anything can address it by. A worker that
//! stops answering is therefore a machine that is gone, and saying so promptly is the only way
//! a bounded operation stays bounded.

use std::time::Duration;

use soma::BackendFailureKind;
use soma_jail::ControlSocket;
use soma_vmm::control::{MAX_REPLY_BYTES, Outcome, Request};

/// The supervisor's end of the pre-connected control socket.
pub(in crate::backend::kvm) struct Control {
    socket: ControlSocket,
}

impl Control {
    pub(super) const fn adopt(socket: ControlSocket) -> Self {
        Self { socket }
    }

    /// Sends one request and returns the reply the worker answered with.
    ///
    /// # Errors
    ///
    /// Returns [`BackendFailureKind::Unavailable`] when the exchange did not complete, which
    /// includes a worker its own filter killed mid-request: the packet it never sent is
    /// indistinguishable from a closed socket, and both mean the machine is gone.
    pub(super) fn ask(
        &self,
        request: &Request,
        within: Duration,
    ) -> Result<Outcome, BackendFailureKind> {
        self.tell(request)?;
        let text = self.receive(within)?;
        Outcome::decode(&text).map_err(|_| BackendFailureKind::Unavailable)
    }

    /// Sends one request and does not wait for an answer.
    pub(super) fn tell(&self, request: &Request) -> Result<(), BackendFailureKind> {
        self.socket
            .send(&request.encode())
            .map_err(|_| BackendFailureKind::Unavailable)
    }

    /// Reads the packet the worker sends before it serves anything: its own attestation.
    pub(super) fn receive(&self, within: Duration) -> Result<String, BackendFailureKind> {
        self.socket
            .receive(MAX_REPLY_BYTES, within)
            .map_err(|_| BackendFailureKind::Unavailable)
    }
}
