use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use soma::{InstanceId, MachineRelease, ManagedFailure, StateStoreFailureKind, TerminalCensus};

use super::{Retention, Sweep, SweepFailure, TERMINAL_RETENTION, TerminalRecords, sweep};

/// A durable store reduced to what a sweep reads: each record, and whether it is terminal.
#[derive(Clone, Default)]
struct Records {
    held: Arc<Mutex<BTreeMap<InstanceId, bool>>>,
    failing_releases: Arc<AtomicUsize>,
    leases: Arc<AtomicUsize>,
}

impl Records {
    fn hold(&self, digit: u32, terminal: bool) -> InstanceId {
        let instance_id = id(digit);
        self.held
            .lock()
            .expect("records")
            .insert(instance_id.clone(), terminal);
        instance_id
    }

    fn holds(&self, instance_id: &InstanceId) -> bool {
        self.held.lock().expect("records").contains_key(instance_id)
    }

    fn opener(&self) -> impl Fn() -> Option<Box<Self>> + use<> {
        let records = self.clone();
        move || {
            records.leases.fetch_add(1, Ordering::SeqCst);
            Some(Box::new(records.clone()))
        }
    }
}

impl TerminalRecords for Records {
    fn terminal_census(&mut self) -> Result<TerminalCensus, ManagedFailure> {
        let held = self.held.lock().expect("records");
        let terminal = held
            .iter()
            .filter(|(_, terminal)| **terminal)
            .map(|(instance_id, _)| instance_id.clone())
            .collect::<Vec<_>>();
        Ok(census(held.len(), terminal))
    }

    fn release(&mut self, instance_id: &InstanceId) -> Result<MachineRelease, ManagedFailure> {
        if self
            .failing_releases
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(ManagedFailure::StateStore(
                StateStoreFailureKind::Unavailable,
            ));
        }
        let mut held = self.held.lock().expect("records");
        Ok(match held.get(instance_id) {
            None => MachineRelease::Absent,
            Some(false) => MachineRelease::Kept,
            Some(true) => {
                held.remove(instance_id);
                MachineRelease::Released
            }
        })
    }
}

fn census(records: usize, terminal: Vec<InstanceId>) -> TerminalCensus {
    TerminalCensus::new(records, terminal)
}

fn id(digit: u32) -> InstanceId {
    InstanceId::new(format!("{digit:032x}")).expect("instance")
}

fn after(start: Instant, elapsed: Duration) -> Instant {
    start + elapsed
}

#[test]
fn a_terminal_record_is_released_only_after_a_whole_window_and_a_live_one_never() {
    let records = Records::default();
    let ended = records.hold(1, true);
    let live = records.hold(2, false);
    let mut retention = Retention::new(TERMINAL_RETENTION);
    let start = Instant::now();
    let open = records.opener();

    let first = sweep(&mut retention, &open, start).expect("first sweep");
    assert_eq!(
        first,
        Sweep {
            records: 2,
            terminal: 1,
            released: 0
        }
    );
    let early = after(start, TERMINAL_RETENTION - Duration::from_secs(1));
    assert_eq!(
        sweep(&mut retention, &open, early).expect("early").released,
        0
    );
    assert!(
        records.holds(&ended),
        "a repeated request may still need it"
    );

    let due = sweep(&mut retention, &open, after(start, TERMINAL_RETENTION)).expect("due");
    assert_eq!(due.released, 1);
    assert_eq!(due.records, 1);
    assert!(!records.holds(&ended));
    let much_later = after(start, TERMINAL_RETENTION * 100);
    assert_eq!(
        sweep(&mut retention, &open, much_later)
            .expect("later")
            .released,
        0
    );
    assert!(records.holds(&live));
}

#[test]
fn after_a_restart_records_an_earlier_process_left_wait_one_whole_window() {
    let records = Records::default();
    for digit in 1..=3 {
        records.hold(digit, true);
    }
    // A new process: nothing remembers when these records ended.
    let mut retention = Retention::new(TERMINAL_RETENTION);
    let start = Instant::now();
    let open = records.opener();

    assert_eq!(
        sweep(&mut retention, &open, start).expect("first").released,
        0
    );
    let due = sweep(&mut retention, &open, after(start, TERMINAL_RETENTION)).expect("due");
    assert_eq!(due.released, 3);
    assert_eq!(due.records, 0);
}

#[test]
fn a_release_that_fails_is_tried_again_by_the_next_sweep() {
    let records = Records::default();
    let ended = records.hold(1, true);
    let mut retention = Retention::new(TERMINAL_RETENTION);
    let start = Instant::now();
    let open = records.opener();
    sweep(&mut retention, &open, start).expect("first");

    records.failing_releases.store(1, Ordering::SeqCst);
    let failed = sweep(&mut retention, &open, after(start, TERMINAL_RETENTION)).expect("failed");
    assert_eq!(failed.released, 0);
    assert!(records.holds(&ended));

    let retried = after(start, TERMINAL_RETENTION + Duration::from_secs(60));
    assert_eq!(
        sweep(&mut retention, &open, retried)
            .expect("retry")
            .released,
        1
    );
}

#[test]
fn an_identity_released_elsewhere_and_ended_again_starts_a_new_window() {
    let records = Records::default();
    let reused = records.hold(1, true);
    let mut retention = Retention::new(TERMINAL_RETENTION);
    let start = Instant::now();
    let open = records.opener();
    sweep(&mut retention, &open, start).expect("first");

    records.held.lock().expect("records").remove(&reused);
    sweep(&mut retention, &open, after(start, Duration::from_mins(5))).expect("gone");
    records.hold(1, true);
    let ended_again = after(start, Duration::from_mins(6));
    sweep(&mut retention, &open, ended_again).expect("seen again");

    let first_window = after(start, TERMINAL_RETENTION);
    assert_eq!(
        sweep(&mut retention, &open, first_window)
            .expect("old")
            .released,
        0
    );
    let second_window = ended_again + TERMINAL_RETENTION;
    assert_eq!(
        sweep(&mut retention, &open, second_window)
            .expect("new")
            .released,
        1
    );
}

#[test]
fn a_backlog_is_released_in_bounded_leases() {
    let records = Records::default();
    for digit in 1..=1_000 {
        records.hold(digit, true);
    }
    let mut retention = Retention::new(TERMINAL_RETENTION);
    let start = Instant::now();
    let open = records.opener();
    sweep(&mut retention, &open, start).expect("first");
    records.leases.store(0, Ordering::SeqCst);

    let due = sweep(&mut retention, &open, after(start, TERMINAL_RETENTION)).expect("due");

    assert_eq!(due.released, 1_000);
    // One lease for the census and one for each batch of 256 releases.
    assert_eq!(records.leases.load(Ordering::SeqCst), 1 + 4);
}

#[test]
fn a_pool_with_no_facade_to_lend_skips_the_sweep() {
    let mut retention = Retention::new(TERMINAL_RETENTION);
    let busy = || None::<Box<Records>>;

    assert_eq!(
        sweep(&mut retention, &busy, Instant::now()),
        Err(SweepFailure::Busy)
    );
}
