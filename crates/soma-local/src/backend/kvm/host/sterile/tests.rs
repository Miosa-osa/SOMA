//! The pool's behaviour with stand-in machines: a live `sleep` process takes the place of a
//! restored machine host, so ownership and termination are the real thing.

use std::{
    os::unix::net::UnixStream,
    process::{Command, Stdio},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use super::{
    Builder, PrimedKey, SpawnFailure, SterileHost, Wanted, locked, pool,
    refill::{admit, refill_pass},
    retire::retire_surplus,
    set_desired,
    spawn::terminate,
    take,
};

/// The pool is process-global, so tests that touch it run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn key(reference: &str) -> PrimedKey {
    PrimedKey {
        reference: reference.to_owned(),
        generation_id: soma::GenerationId::new(format!("sha256:{}", "ab".repeat(32)))
            .expect("a canonical generation id"),
        memory_mib: 2048,
    }
}

/// A live stand-in process: it is alive until something kills it.
fn host(primed: Option<PrimedKey>) -> SterileHost {
    let (handoff, _peer) = UnixStream::pair().expect("a socket pair");
    let child = Command::new("sleep")
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .expect("sleep starts");
    SterileHost {
        child,
        handoff,
        output: None,
        primed,
    }
}

/// Starts a test with an empty pool and nothing desired.
fn fresh() -> std::sync::MutexGuard<'static, ()> {
    let guard = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for mut stale in std::mem::take(&mut *locked(&pool().available)) {
        terminate(&mut stale.child);
    }
    *locked(&pool().desired) = super::Desired::default();
    guard
}

fn wanted(reference: &str, count: usize, build: Builder) -> Wanted {
    Wanted {
        key: key(reference),
        build,
        count,
    }
}

fn building(reference: &'static str) -> Builder {
    Arc::new(move || Ok(host(Some(key(reference)))))
}

fn prepared(reference: &str) -> usize {
    let wanted = key(reference);
    locked(&pool().available)
        .iter()
        .filter(|held| held.primed.as_ref() == Some(&wanted))
        .count()
}

/// A claim racing a shrink either wins a whole live machine or finds none and builds its own.
/// Every machine is accounted for exactly once: claimed, or retired, and a claimed one is never
/// terminated.
#[test]
fn a_claim_racing_a_shrink_wins_a_live_machine_or_gets_a_miss() {
    const MACHINES: usize = 24;
    const CLAIMERS: usize = 6;
    let _serial = fresh();
    for _round in 0..20 {
        // Nothing is desired, so a shrink retires every prepared machine.
        locked(&pool().available).extend((0..MACHINES).map(|_| host(Some(key("node:22")))));

        let start = Arc::new(Barrier::new(CLAIMERS + 1));
        let claimers: Vec<_> = (0..CLAIMERS)
            .map(|_| {
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    let mut won = Vec::new();
                    for _ in 0..MACHINES {
                        if let Some(host) = take(&key("node:22")) {
                            won.push(host);
                        }
                    }
                    won
                })
            })
            .collect();
        let retirer = {
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                retire_surplus()
            })
        };

        let ended = retirer.join().expect("the retirer finishes");
        let mut claimed = 0;
        for claimer in claimers {
            for mut host in claimer.join().expect("a claimer finishes") {
                assert!(
                    host.child.try_wait().expect("a status").is_none(),
                    "a claimed machine was terminated under its claimer"
                );
                claimed += 1;
                terminate(&mut host.child);
            }
        }
        assert_eq!(
            claimed + ended,
            MACHINES,
            "every machine is either claimed or retired, exactly once"
        );
        assert!(locked(&pool().available).is_empty());
    }
}

/// A shape that cannot be built records why and backs off; the shapes after it are still built.
#[test]
fn a_failing_shape_does_not_starve_the_shapes_after_it() {
    let _serial = fresh();
    let attempts = Arc::new(AtomicUsize::new(0));
    let failing: Builder = {
        let attempts = Arc::clone(&attempts);
        Arc::new(move || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(SpawnFailure::at(
                "the machine host refused to restore this Generation",
            ))
        })
    };
    set_desired(
        vec![
            wanted("broken:1", 2, failing),
            wanted("node:22", 3, building("node:22")),
        ],
        0,
    );

    assert_eq!(
        refill_pass(),
        3,
        "the healthy shape is built in the same pass"
    );
    assert_eq!(prepared("node:22"), 3);
    assert_eq!(prepared("broken:1"), 0);
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "the broken shape was tried once"
    );

    {
        let desired = locked(&pool().desired);
        let broken = desired
            .targets
            .iter()
            .find(|target| target.key.reference == "broken:1")
            .expect("the broken target");
        assert_eq!(
            broken.last_failure.as_deref(),
            Some("the machine host refused to restore this Generation")
        );
        assert!(broken.retry_at.is_some_and(|at| at > Instant::now()));
        assert_eq!(broken.failures, 1);
    }

    // Still backing off: another pass neither retries it nor builds anything.
    assert_eq!(refill_pass(), 0);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);

    // Once the backoff has passed it is tried again, and a second failure waits longer.
    locked(&pool().desired).targets[0].retry_at = Some(Instant::now());
    assert_eq!(refill_pass(), 0);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(locked(&pool().desired).targets[0].failures, 2);

    for mut host in std::mem::take(&mut *locked(&pool().available)) {
        terminate(&mut host.child);
    }
}

/// A build that finishes after a shrink is ended, not left in the pool outside the budget.
#[test]
fn a_build_finishing_after_a_shrink_is_ended() {
    let _serial = fresh();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let pid = Arc::new(Mutex::new(None));
    let slow: Builder = {
        let pid = Arc::clone(&pid);
        let started_tx = Mutex::new(started_tx);
        Arc::new(move || {
            let built = host(Some(key("node:22")));
            *pid.lock().expect("pid") = Some(built.child.id());
            started_tx
                .lock()
                .expect("sender")
                .send(())
                .expect("started");
            release_rx
                .lock()
                .expect("receiver")
                .recv()
                .expect("released");
            Ok(built)
        })
    };
    set_desired(vec![wanted("node:22", 1, slow)], 0);

    let pass = thread::spawn(refill_pass);
    started_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the build started");
    // The shrink lands while the machine is still being built.
    set_desired(vec![wanted("node:22", 0, building("node:22"))], 0);
    assert_eq!(retire_surplus(), 0, "nothing prepared yet to retire");
    release_tx.send(()).expect("release the build");

    assert_eq!(
        pass.join().expect("the pass finishes"),
        0,
        "the late machine is not kept"
    );
    assert_eq!(prepared("node:22"), 0);
    let pid = pid.lock().expect("pid").expect("the build ran");
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "the late machine was terminated and reaped"
    );
}

/// A machine built for a target that has since gone, or that is already full, is refused.
#[test]
fn admit_keeps_a_machine_only_while_the_set_wants_it() {
    let _serial = fresh();
    set_desired(vec![wanted("node:22", 1, building("node:22"))], 0);
    assert!(admit(host(Some(key("node:22"))), Some(&key("node:22"))));
    assert!(
        !admit(host(Some(key("node:22"))), Some(&key("node:22"))),
        "the target is full"
    );
    assert!(
        !admit(host(Some(key("gone:1"))), Some(&key("gone:1"))),
        "no such target"
    );
    assert!(!admit(host(None), None), "no spares are wanted");
    assert_eq!(prepared("node:22"), 1);
    for mut host in std::mem::take(&mut *locked(&pool().available)) {
        terminate(&mut host.child);
    }
}

/// A claim takes a machine primed for its shape before an unprimed spare, whichever was added last,
/// and never one primed for another shape.
#[test]
fn a_claim_prefers_a_primed_machine_over_a_spare() {
    let _serial = fresh();
    {
        let mut available = locked(&pool().available);
        available.push(host(Some(key("node:22"))));
        available.push(host(None));
        available.push(host(Some(key("other:1"))));
    }
    let first = take(&key("node:22")).expect("a machine");
    assert!(
        first.primed == Some(key("node:22")),
        "the primed one, not the spare"
    );
    let second = take(&key("node:22")).expect("the spare");
    assert!(second.primed.is_none());
    assert!(
        take(&key("node:22")).is_none(),
        "the other shape's machine is not taken"
    );
    for mut left in [first, second] {
        terminate(&mut left.child);
    }
    for mut host in std::mem::take(&mut *locked(&pool().available)) {
        terminate(&mut host.child);
    }
}
