//! The background refill: build what the desired set is short of, one machine at a time.
//!
//! A shape that cannot be built records why and backs off, and the pass moves on to the next
//! shape, so one broken Generation never holds up the others. A machine whose build finishes after
//! the set changed is kept only if the set still wants it.

use std::{
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

use super::{
    Builder, PrimedKey, SterileHost, locked, pool,
    spawn::{SpawnFailure, spawn_explained, terminate},
};

/// Refill begins after the first command should have completed, so maintenance cannot inflate TTI.
const REFILL_DELAY: Duration = Duration::from_millis(250);
/// The first retry of a shape that failed to build; each further failure doubles it.
const FIRST_RETRY: Duration = Duration::from_secs(5);
/// The longest a failing shape waits between attempts.
const LONGEST_RETRY: Duration = Duration::from_secs(60);
/// How often a refill that is only waiting on a backoff looks at the set again.
const WAIT_SLICE: Duration = Duration::from_secs(1);

/// What the next machine to build is.
pub(super) enum Missing {
    Primed(Builder, Box<PrimedKey>),
    Spare,
}

/// What the refill should do now.
pub(super) enum Next {
    Build(Missing),
    /// Everything short is backing off; look again after this long.
    Wait(Duration),
    Done,
}

pub(super) fn schedule_refill() {
    let sterile_pool = pool();
    if sterile_pool
        .replenishing
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let started = thread::Builder::new()
        .name("soma-sterile-refill".to_owned())
        .spawn(|| {
            thread::sleep(REFILL_DELAY);
            loop {
                refill_pass();
                match next_missing(Instant::now()) {
                    Next::Wait(wait) => thread::sleep(wait.min(WAIT_SLICE)),
                    Next::Build(_) => {}
                    Next::Done => {
                        pool().replenishing.store(false, Ordering::Release);
                        // A change that landed between the last look and the flag is not lost.
                        if matches!(next_missing(Instant::now()), Next::Done) {
                            return;
                        }
                        if pool()
                            .replenishing
                            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        });
    if started.is_err() {
        sterile_pool.replenishing.store(false, Ordering::Release);
    }
}

/// The next machine to build, skipping any shape that is backing off.
pub(super) fn next_missing(now: Instant) -> Next {
    let mut desired = locked(&pool().desired);
    let available = locked(&pool().available);
    let mut wait: Option<Duration> = None;
    let mut soonest = |until: Instant| {
        let left = until.saturating_duration_since(now);
        wait = Some(wait.map_or(left, |current| current.min(left)));
    };
    let mut chosen = None;
    for (index, target) in desired.targets.iter().enumerate() {
        let have = available
            .iter()
            .filter(|host| host.primed.as_ref() == Some(&target.key))
            .count();
        if have >= target.count {
            continue;
        }
        match target.retry_at {
            Some(at) if at > now => soonest(at),
            _ => {
                chosen = Some(index);
                break;
            }
        }
    }
    if let Some(index) = chosen {
        let target = &mut desired.targets[index];
        target.building = true;
        return Next::Build(Missing::Primed(
            target.build.clone(),
            Box::new(target.key.clone()),
        ));
    }
    for target in &mut desired.targets {
        target.building = false;
    }
    let spares = available
        .iter()
        .filter(|host| host.primed.is_none())
        .count();
    if spares < desired.generic {
        match desired.spare_retry_at {
            Some(at) if at > now => soonest(at),
            _ => return Next::Build(Missing::Spare),
        }
    }
    wait.map_or(Next::Done, Next::Wait)
}

/// Builds what is missing until nothing is ready to build; returns how many machines were kept.
pub(super) fn refill_pass() -> usize {
    let mut kept = 0;
    while let Next::Build(missing) = next_missing(Instant::now()) {
        let (key, built) = match &missing {
            Missing::Primed(build, key) => (Some(key.as_ref().clone()), build()),
            Missing::Spare => (None, spawn_explained(None)),
        };
        match built {
            Ok(host) => {
                note_built(key.as_ref(), None);
                if admit(host, key.as_ref()) {
                    kept += 1;
                }
            }
            Err(failure) => note_built(key.as_ref(), Some(failure)),
        }
    }
    kept
}

/// Puts a built machine into the pool if the set still wants it; otherwise ends it.
///
/// The set is read again under the same locks that guard the pool, because the build ran outside
/// them and a shrink may have landed meanwhile. Without this a machine finishing after a shrink
/// would sit in the pool, holding memory outside the committed budget, until the next change.
pub(super) fn admit(host: SterileHost, key: Option<&PrimedKey>) -> bool {
    let rejected = {
        let desired = locked(&pool().desired);
        let mut available = locked(&pool().available);
        let wanted = match key {
            Some(key) => desired
                .targets
                .iter()
                .find(|t| &t.key == key)
                .is_some_and(|t| {
                    available
                        .iter()
                        .filter(|held| held.primed.as_ref() == Some(key))
                        .count()
                        < t.count
                }),
            None => {
                available
                    .iter()
                    .filter(|held| held.primed.is_none())
                    .count()
                    < desired.generic
            }
        };
        if wanted {
            available.push(host);
            None
        } else {
            Some(host)
        }
    };
    match rejected {
        None => true,
        Some(mut host) => {
            terminate(&mut host.child);
            false
        }
    }
}

/// Records the outcome of one build: clears the shape's history on success, and on failure keeps
/// the reason and sets when the next attempt may happen.
fn note_built(key: Option<&PrimedKey>, failure: Option<SpawnFailure>) {
    let now = Instant::now();
    let reason = failure.map(|failure| failure.reason);
    let settle = |failures: &mut u32, retry_at: &mut Option<Instant>| {
        if reason.is_some() {
            *failures = failures.saturating_add(1);
            *retry_at = Some(now + backoff(*failures));
        } else {
            *failures = 0;
            *retry_at = None;
        }
    };
    let mut desired = locked(&pool().desired);
    if let Some(key) = key {
        if let Some(target) = desired.targets.iter_mut().find(|t| &t.key == key) {
            settle(&mut target.failures, &mut target.retry_at);
            target.last_failure = reason;
            target.building = false;
        }
    } else {
        let desired = &mut *desired;
        settle(&mut desired.spare_failures, &mut desired.spare_retry_at);
    }
}

fn backoff(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(6);
    FIRST_RETRY
        .saturating_mul(1 << doublings)
        .min(LONGEST_RETRY)
}
