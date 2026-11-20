//! Releasing the durable records of sandboxes that ended (ADR 0055).
//!
//! A destroyed sandbox, and a launch that failed after it was admitted, leave a terminal record in
//! the state root. Nothing removed one, so at benchmark volume the 2G tmpfs on host-04 filled and
//! every create after that failed its first write (SOMA#127).
//!
//! The rule: a terminal record is released once it has been terminal for [`TERMINAL_RETENTION`],
//! counted from the first sweep that saw it terminal. Ten minutes is how long the runner answers a
//! repeated destroy from its own memory, and the loopback service replays an exact repeated
//! operation from the record for at least as long. A record an earlier process left is first seen
//! by this process's first sweep, so after a restart it is released one window later, not at once.
//! A record that is not terminal is never released, and one that cannot be decoded is counted and
//! kept for an operator.
//!
//! The sweep leases its facades from the same pool requests use, one lease for the census and one
//! for each bounded batch of releases, so a backlog never holds a facade for long, and a pool too
//! busy to lend one skips the sweep: requests come first, and the next sweep finds what this one
//! did not.

use std::{
    collections::HashMap,
    io,
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use soma::{InstanceId, MachineRelease, ManagedFailure, TerminalCensus};

use crate::facade::SandboxFacade;

/// How long a terminal record is kept after a sweep first saw it terminal.
pub const TERMINAL_RETENTION: Duration = Duration::from_mins(10);

/// How often the store is swept.
pub const SWEEP_INTERVAL: Duration = Duration::from_mins(1);

/// The most releases made on one facade lease.
const RELEASES_PER_LEASE: usize = 256;

/// The two facade calls a sweep makes, so a sweep can be exercised without a whole facade.
pub trait TerminalRecords {
    /// See [`SandboxFacade::terminal_census`].
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn terminal_census(&mut self) -> Result<TerminalCensus, ManagedFailure>;

    /// See [`SandboxFacade::release`].
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn release(&mut self, instance_id: &InstanceId) -> Result<MachineRelease, ManagedFailure>;
}

impl<F: SandboxFacade + ?Sized> TerminalRecords for F {
    fn terminal_census(&mut self) -> Result<TerminalCensus, ManagedFailure> {
        SandboxFacade::terminal_census(self)
    }

    fn release(&mut self, instance_id: &InstanceId) -> Result<MachineRelease, ManagedFailure> {
        SandboxFacade::release(self, instance_id)
    }
}

/// When each terminal record was first seen terminal, and so which are due for release.
#[derive(Debug)]
pub struct Retention {
    window: Duration,
    first_seen: HashMap<InstanceId, Instant>,
}

impl Retention {
    #[must_use]
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            first_seen: HashMap::new(),
        }
    }

    /// Takes the Instances that are terminal now and returns those that have been terminal for a
    /// whole window.
    ///
    /// An Instance missing from `terminal` is forgotten: it was released, here or by another
    /// process, and an identity created again later starts a new window.
    pub fn due(&mut self, terminal: Vec<InstanceId>, now: Instant) -> Vec<InstanceId> {
        let mut seen = HashMap::with_capacity(terminal.len());
        let mut due = Vec::new();
        for instance_id in terminal {
            let first = self.first_seen.get(&instance_id).copied().unwrap_or(now);
            if now.saturating_duration_since(first) >= self.window {
                due.push(instance_id.clone());
            }
            seen.insert(instance_id, first);
        }
        self.first_seen = seen;
        due
    }
}

/// What one sweep found and did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sweep {
    /// The records the store holds after this sweep's releases.
    pub records: usize,
    /// The terminal records the census found, released or not.
    pub terminal: usize,
    /// The records this sweep released.
    pub released: usize,
}

/// Why a sweep stopped before it finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SweepFailure {
    /// No facade could be leased.
    Busy,
    /// The census could not read the store.
    Census(ManagedFailure),
}

/// Runs one sweep: a census, then the release of every terminal record that is due.
///
/// A release that fails leaves its record due, so the next sweep tries it again.
///
/// # Errors
///
/// Returns [`SweepFailure::Busy`] when no facade could be leased for the census, or the census's
/// own failure. A pool that runs dry part way through the releases ends the sweep early and is
/// reported as what was released by then.
pub fn sweep<R: TerminalRecords + ?Sized>(
    retention: &mut Retention,
    open: &dyn Fn() -> Option<Box<R>>,
    now: Instant,
) -> Result<Sweep, SweepFailure> {
    let census = open()
        .ok_or(SweepFailure::Busy)?
        .terminal_census()
        .map_err(SweepFailure::Census)?;
    let records = census.records();
    let terminal = census.terminal().len();
    let due = retention.due(census.into_terminal(), now);
    let mut released = 0_usize;
    for batch in due.chunks(RELEASES_PER_LEASE) {
        let Some(mut facade) = open() else {
            break;
        };
        for instance_id in batch {
            match facade.release(instance_id) {
                Ok(MachineRelease::Released) => released = released.saturating_add(1),
                Ok(MachineRelease::Absent | MachineRelease::Kept) => {}
                Err(failure) => eprintln!(
                    "soma-api: the record sweep could not release {}: {failure:?}",
                    instance_id.as_str()
                ),
            }
        }
    }
    Ok(Sweep {
        records: records.saturating_sub(released),
        terminal,
        released,
    })
}

/// The last finished sweep, for the runner's `/healthz`.
static LAST_SWEEP: Mutex<Option<Sweep>> = Mutex::new(None);

/// What the last finished sweep in this process found, or nothing before the first one.
#[must_use]
pub fn last_sweep() -> Option<Sweep> {
    *LAST_SWEEP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Starts the sweep on its own thread, once every [`SWEEP_INTERVAL`] for the life of the process.
///
/// # Errors
///
/// Returns the failure to start the thread.
pub fn spawn(
    open: impl Fn() -> Option<Box<dyn SandboxFacade>> + Send + 'static,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("soma-record-sweep".to_owned())
        .spawn(move || {
            let mut retention = Retention::new(TERMINAL_RETENTION);
            loop {
                thread::sleep(SWEEP_INTERVAL);
                match sweep(&mut retention, &open, Instant::now()) {
                    Ok(done) => {
                        if done.released > 0 {
                            eprintln!(
                                "soma-api: released {} terminal sandbox record(s); the state \
                                 store holds {}",
                                done.released, done.records
                            );
                        }
                        *LAST_SWEEP
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(done);
                    }
                    Err(SweepFailure::Busy) => {}
                    Err(SweepFailure::Census(failure)) => {
                        eprintln!(
                            "soma-api: the record sweep could not read the store: {failure:?}"
                        );
                    }
                }
            }
        })
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod tests;
