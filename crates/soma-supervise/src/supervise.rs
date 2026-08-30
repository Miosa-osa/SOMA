//! The one thread that owns a tool's lifetime: deadline, termination, and reaping.
//!
//! Signalling and reaping happen in this thread only and in that exact order, so no signal can
//! ever reach a process-group identifier the caller has already released.
//! The thread is bounded by construction: it waits at most until the deadline, then spends at
//! most the termination and force graces before it reaps the leader and reports.
//! Every exit path forces the group once, including the leader's own normal exit, so a
//! descendant that kept an inherited pipe can never outlive the invocation or block its feed.

use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use super::group::{Group, Signal};

/// One bounded wait between exit polls.
const EXIT_POLL: Duration = Duration::from_millis(5);

/// How one supervised tool ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Supervised {
    /// The exit code, or `None` when the tool was killed or signalled.
    pub(crate) exit_code: Option<i32>,
    /// Whether the supervisor had to terminate the group instead of observing a normal exit.
    pub(crate) terminated: bool,
}

/// A handle to the supervising thread.
pub(crate) struct Supervisor {
    outcome: Receiver<Supervised>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Supervisor {
    /// Takes ownership of the child and supervises it until it ends or the deadline passes.
    ///
    /// `requests` is the cancellation channel: any message on it, from the caller's feed
    /// failure or from a reader that overflowed its capture ceiling, terminates the group.
    pub(crate) fn start(
        child: Child,
        group: Group,
        deadline: Instant,
        graces: (Duration, Duration),
        requests: Receiver<()>,
    ) -> Self {
        let (report, outcome) = mpsc::channel();
        let handle =
            thread::spawn(move || supervise(child, group, deadline, graces, &requests, &report));
        Self {
            outcome,
            handle: Some(handle),
        }
    }

    /// Collects the outcome, waiting no longer than `until`, and joins the thread.
    pub(crate) fn finish(mut self, until: Instant) -> Option<Supervised> {
        let remaining = until.saturating_duration_since(Instant::now());
        let supervised = self.outcome.recv_timeout(remaining).ok();
        if supervised.is_some()
            && let Some(handle) = self.handle.take()
        {
            let _ = handle.join();
        }
        supervised
    }
}

fn supervise(
    mut child: Child,
    group: Group,
    deadline: Instant,
    graces: (Duration, Duration),
    requests: &Receiver<()>,
    report: &Sender<Supervised>,
) {
    let (term_grace, kill_grace) = graces;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // The leader is gone, but a descendant it forked can still hold the inherited
                // pipes; forcing the group once here closes them, so a caller blocked writing
                // standard input fails at once instead of waiting for a process no bound
                // covers. The identifier is still reserved exactly while such a member exists,
                // so this can only reach the tool's own descendants.
                group.signal(Signal::Force);
                let _ = report.send(Supervised {
                    exit_code: status.code(),
                    terminated: false,
                });
                return;
            }
            Err(_) => break,
            Ok(None) => {}
        }
        if !matches!(requests.try_recv(), Err(TryRecvError::Empty)) || Instant::now() >= deadline {
            break;
        }
        thread::sleep(EXIT_POLL);
    }
    group.signal(Signal::Terminate);
    if !await_exit(&mut child, term_grace) {
        group.signal(Signal::Force);
        await_exit(&mut child, kill_grace);
    }
    // A member that outlived its leader is still in the group, so force the whole group once
    // more before the leader is reaped and its identifier is released.
    group.signal(Signal::Force);
    let exit_code = child.wait().ok().and_then(|status| status.code());
    let _ = report.send(Supervised {
        exit_code,
        terminated: true,
    });
}

fn await_exit(child: &mut Child, grace: Duration) -> bool {
    let until = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return true,
            Ok(None) if Instant::now() < until => thread::sleep(EXIT_POLL),
            Ok(None) => return false,
        }
    }
}
