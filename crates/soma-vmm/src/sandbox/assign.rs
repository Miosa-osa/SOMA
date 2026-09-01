//! The prepared half of a Session: parking a sterile machine, and assigning one Instance to it.
//!
//! A prepared machine reaches its session by exactly the path a restored one does, so the mint,
//! the broker activation, and the link gate are the same exchange [`Session::launch`] makes. The
//! only difference is when the machine was built, which is the whole point of preparing it.

use std::sync::mpsc::channel;
use std::time::Duration;

use soma_guest::ActivationReceipt;

use super::session::{BOOT_DEADLINE, EXIT_GRACE, Request, Response, Session, SessionError};
use super::sterile::{self, Assignment, SterileSpec};

/// How long restoring one sterile machine has before the worker is considered lost.
///
/// A restore is milliseconds of work, so this is a liveness bound on a wedged host rather than
/// a budget: a preparation that takes this long has already failed at whatever it was doing.
const PREPARE_DEADLINE: Duration = Duration::from_secs(60);

impl Session {
    /// Restores one sterile machine on its own thread and returns once it is parked.
    ///
    /// The returned session owns a machine that has paid for everything a restore costs except
    /// the authority one Instance owns. Dropping it destroys the machine, which is what a
    /// single-use worker must do rather than return to a pool.
    ///
    /// # Errors
    ///
    /// Returns the [`SessionError`] the sandbox thread reported, or [`SessionError::Gone`]
    /// when no machine exists behind the session it would have returned.
    pub fn prepare(spec: SterileSpec) -> Result<Self, SessionError> {
        let (request_tx, request_rx) = channel();
        let (response_tx, response_rx) = channel();
        let thread = std::thread::Builder::new()
            .name("soma-kvm-prepared".to_owned())
            .spawn(move || sterile::serve(spec, &request_rx, &response_tx))
            .map_err(|_| SessionError::Create)?;
        let mut session = Self {
            requests: request_tx,
            responses: response_rx,
            thread: Some(thread),
            poisoned: false,
        };
        match session.await_response(PREPARE_DEADLINE) {
            Ok(Response::Prepared) => Ok(session),
            // Anything else means no sterile machine exists behind this session, so the caller
            // must not be handed one that looks claimable.
            Ok(Response::Failed(error)) | Err(error) => Err(error),
            Ok(_) => Err(SessionError::Create),
        }
    }

    /// Transfers fresh Instance authority into a parked sterile machine and waits for Ready.
    ///
    /// This is the same exchange [`Self::launch`] makes once the machine exists, including the
    /// mint, the broker activation, and the link gate, because a prepared machine reaches its
    /// session by exactly the path a restored one does. A failure poisons the session, which
    /// stops its thread and releases the machine, because a transfer that did not certainly
    /// complete leaves authority nobody can describe.
    ///
    /// # Errors
    ///
    /// Returns the [`SessionError`] that ended the transfer; the session is poisoned and its
    /// machine released before the failure is returned.
    pub fn assign(
        &mut self,
        assignment: Assignment,
        activate: &mut dyn FnMut(&ActivationReceipt) -> Result<(), SessionError>,
    ) -> Result<(), SessionError> {
        self.requests
            .send(Request::Assign(Box::new(assignment)))
            .map_err(|_| self.poison(SessionError::Gone))?;
        match self.await_response(BOOT_DEADLINE + EXIT_GRACE) {
            Ok(Response::Ready) => Ok(()),
            Ok(Response::Minted(receipt)) => self.raise_the_link(&receipt, activate),
            Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
            Ok(_) => Err(self.poison(SessionError::Boot)),
        }
    }

    /// The mint, activation, and link exchange, over a session this caller already holds.
    pub(super) fn raise_the_link(
        &mut self,
        receipt: &ActivationReceipt,
        activate: &mut dyn FnMut(&ActivationReceipt) -> Result<(), SessionError>,
    ) -> Result<(), SessionError> {
        activate(receipt).map_err(|error| self.poison(error))?;
        self.requests
            .send(Request::RaiseLink)
            .map_err(|_| self.poison(SessionError::Gone))?;
        match self.await_response(BOOT_DEADLINE) {
            Ok(Response::Ready) => Ok(()),
            Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
            Ok(_) => Err(self.poison(SessionError::Boot)),
        }
    }
}
