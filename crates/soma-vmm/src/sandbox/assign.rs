//! The prepared half of a Session: parking a sterile machine, and assigning one Instance to it.
//!
//! A prepared machine reaches its session by exactly the path a restored one does, so the mint,
//! the broker activation, and the link gate are the same exchange [`Session::launch`] makes. The
//! only difference is when the machine was built, which is the whole point of preparing it.

use std::sync::mpsc::channel;
use std::time::Duration;

use soma_guest::ActivationReceipt;

use super::session::{
    ACTIVATION_PATIENCE, Activation, BOOT_DEADLINE, EXIT_GRACE, Outcome, Request, Response,
    Session, SessionError,
};
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
            pending: None,
            head_reads: Vec::new(),
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
        activate: &mut dyn FnMut(&ActivationReceipt) -> Result<Activation, SessionError>,
    ) -> Result<(), SessionError> {
        self.requests
            .send(Request::Assign(Box::new(assignment)))
            .map_err(|_| self.poison(SessionError::Gone))?;
        match self.await_response(BOOT_DEADLINE + EXIT_GRACE) {
            Ok(Response::Ready { head_reads }) => {
                self.head_reads = head_reads;
                Ok(())
            }
            Ok(Response::Minted(receipt)) => self.raise_the_link(&receipt, activate),
            Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
            Ok(_) => Err(self.poison(SessionError::Boot)),
        }
    }

    /// The mint, activation, and link exchange, over a session this caller already holds.
    pub(super) fn raise_the_link(
        &mut self,
        receipt: &ActivationReceipt,
        activate: &mut dyn FnMut(&ActivationReceipt) -> Result<Activation, SessionError>,
    ) -> Result<(), SessionError> {
        let job = match activate(receipt).map_err(|error| self.poison(error))? {
            Activation::Done => None,
            Activation::Deferred(job) => Some(job),
        };
        if let Some(job) = job {
            return self.activate_in_background(job);
        }
        self.requests
            .send(Request::RaiseLink)
            .map_err(|_| self.poison(SessionError::Gone))?;
        match self.await_response(BOOT_DEADLINE) {
            Ok(Response::Ready { head_reads }) => {
                self.head_reads = head_reads;
                Ok(())
            }
            Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
            Ok(_) => Err(self.poison(SessionError::Boot)),
        }
    }

    /// Asks the broker off the launch path, and raises the link only once it has answered.
    ///
    /// The launch returns here; the broker's answer, the link, and the machine's Ready arrive
    /// behind it. Nothing may use the network before they do, so every operation that can
    /// (`execute`, `file`, `pty`, a shutdown) first calls [`Self::settle`], which waits for the
    /// answer a bounded time and ends the session with [`SessionError::Network`] when it is no.
    fn activate_in_background(
        &mut self,
        job: super::session::ActivationJob,
    ) -> Result<(), SessionError> {
        let outcome = std::sync::Arc::new(Outcome::default());
        let report = std::sync::Arc::clone(&outcome);
        let raise = self.requests.clone();
        std::thread::Builder::new()
            .name("soma-activate".to_owned())
            .spawn(move || {
                let result = job().and_then(|()| {
                    raise
                        .send(Request::RaiseLink)
                        .map_err(|_| SessionError::Gone)
                });
                report.set(result);
            })
            .map_err(|_| self.poison(SessionError::Network))?;
        self.pending = Some(outcome);
        Ok(())
    }

    /// Waits for a background activation to finish, then for the machine to report Ready.
    ///
    /// A session whose activation already settled, or never had one, returns at once.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Network`] when the broker refused or did not answer in time, or
    /// the failure that ended the machine; the session is poisoned either way, because a
    /// sandbox promised a network it does not have must not be handed to a caller.
    pub fn settle(&mut self) -> Result<(), SessionError> {
        let Some(outcome) = self.pending.take() else {
            return Ok(());
        };
        match outcome.wait(ACTIVATION_PATIENCE) {
            Some(Ok(())) => match self.await_response(BOOT_DEADLINE) {
                Ok(Response::Ready { head_reads }) => {
                    self.head_reads = head_reads;
                    Ok(())
                }
                Ok(Response::Failed(error)) | Err(error) => Err(self.poison(error)),
                Ok(_) => Err(self.poison(SessionError::Boot)),
            },
            Some(Err(error)) => Err(self.poison(error)),
            None => Err(self.poison(SessionError::Network)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc::channel};

    use super::*;

    /// A session whose machine thread is a stand-in that answers `RaiseLink` with Ready.
    fn session_with_stand_in() -> (Session, std::thread::JoinHandle<Vec<&'static str>>) {
        let (request_tx, request_rx) = channel::<Request>();
        let (response_tx, response_rx) = channel();
        let stand_in = std::thread::spawn(move || {
            let mut seen = Vec::new();
            while let Ok(request) = request_rx.recv() {
                match request {
                    Request::RaiseLink => {
                        seen.push("raise");
                        let _ = response_tx.send(Response::Ready {
                            head_reads: Vec::new(),
                        });
                    }
                    Request::Execute(_) => seen.push("execute"),
                    _ => seen.push("other"),
                }
            }
            seen
        });
        let session = Session {
            requests: request_tx,
            responses: response_rx,
            thread: None,
            poisoned: false,
            pending: None,
            head_reads: Vec::new(),
        };
        (session, stand_in)
    }

    /// The link is raised only after the broker's answer, and settle returns once it is up.
    #[test]
    fn a_deferred_activation_raises_the_link_only_after_the_broker_accepts() {
        let (mut session, stand_in) = session_with_stand_in();
        let accepted = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&accepted);
        session
            .activate_in_background(Box::new(move || {
                std::thread::sleep(Duration::from_millis(50));
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }))
            .expect("started");
        assert!(
            !accepted.load(Ordering::SeqCst),
            "the launch must return before the broker has answered"
        );
        session.settle().expect("settled");
        assert!(accepted.load(Ordering::SeqCst));
        assert!(session.is_usable());
        assert!(session.settle().is_ok(), "settling twice is a no-op");
        drop(session);
        assert_eq!(stand_in.join().expect("stand-in"), vec!["raise"]);
    }

    /// A refused activation ends the session, and the link is never raised.
    #[test]
    fn a_refused_deferred_activation_poisons_the_session_and_never_raises_the_link() {
        let (mut session, stand_in) = session_with_stand_in();
        session
            .activate_in_background(Box::new(|| Err(SessionError::Network)))
            .expect("started");
        assert_eq!(session.settle(), Err(SessionError::Network));
        assert!(!session.is_usable());
        assert_eq!(session.settle(), Ok(()), "nothing left to wait for");
        drop(session);
        assert!(stand_in.join().expect("stand-in").is_empty());
    }

    /// An answer that never comes is a bounded wait, not a hang.
    #[test]
    fn waiting_for_an_answer_that_never_comes_gives_up() {
        let outcome = Outcome::default();
        assert!(outcome.wait(Duration::from_millis(20)).is_none());
        outcome.set(Ok(()));
        assert_eq!(outcome.wait(Duration::from_millis(20)), Some(Ok(())));
    }
}
