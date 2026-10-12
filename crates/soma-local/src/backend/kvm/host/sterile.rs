//! Machine-host processes prepared before a launch asks for one.
//!
//! A generic host is an already-created process blocked on its private launch socket.
//! A primed host additionally owns an identity-free stopped VM built from verified artifacts.
//! Claiming either preserves the one-process-per-machine boundary, while a primed host also removes
//! restore and device construction from TTI.
//!
//! What the pool keeps is a desired set (ADR 0052): a list of shapes with a count each, plus a
//! number of unprimed spares. The background refill builds what is missing, one machine at a time,
//! and a shape that cannot be built backs off without holding up the others.

mod refill;
mod retire;
mod snapshot;
mod spawn;
#[cfg(test)]
mod tests;

use std::{
    io::BufReader,
    os::unix::net::UnixStream,
    process::{Child, ChildStdout},
    sync::{Arc, Mutex, OnceLock, atomic::AtomicBool},
    time::Instant,
};

use soma::BackendFailureKind;

use crate::backend::kvm::prepared::PreparedGeneration;
use refill::schedule_refill;
use retire::retire_surplus;
use spawn::{SpawnFailure, spawn, spawn_explained};

pub(crate) use snapshot::snapshot;

pub(super) struct SterileHost {
    pub(super) child: Child,
    pub(super) handoff: UnixStream,
    pub(super) output: Option<BufReader<ChildStdout>>,
    primed: Option<PrimedKey>,
}

#[derive(Clone)]
pub(crate) struct PrewarmPlan {
    prepared: PreparedGeneration,
    memory_mib: u64,
}

impl PrewarmPlan {
    pub(crate) const fn new(prepared: PreparedGeneration, memory_mib: u64) -> Self {
        Self {
            prepared,
            memory_mib,
        }
    }

    /// The boot memory the machines for this plan are built with, in MiB.
    pub(crate) const fn memory_mib(&self) -> u64 {
        self.memory_mib
    }

    fn key(&self) -> PrimedKey {
        PrimedKey {
            reference: self.prepared.reference.clone(),
            generation_id: self.prepared.id.clone(),
            memory_mib: self.memory_mib,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
struct PrimedKey {
    reference: String,
    generation_id: soma::GenerationId,
    memory_mib: u64,
}

/// Builds one primed machine for a target. A closure, so the pool never needs the prepared
/// Generation itself and a test can supply a stand-in.
type Builder = Arc<dyn Fn() -> Result<SterileHost, SpawnFailure> + Send + Sync>;

/// One shape the pool is told to keep prepared, and what has happened to it.
struct Target {
    key: PrimedKey,
    build: Builder,
    count: usize,
    building: bool,
    last_failure: Option<String>,
    /// Consecutive failed builds, which sets how long the next attempt waits.
    failures: u32,
    /// No build is attempted before this moment, so a shape that cannot be built does not spin.
    retry_at: Option<Instant>,
}

/// Launches that wanted one shape: found a prepared machine, or did not.
#[derive(Clone, Copy, Default)]
struct Demand {
    primed: u64,
    miss: u64,
    /// The memory the Generation boots with, which is the only memory a target can name.
    boot_mib: u64,
}

#[derive(Default)]
struct Desired {
    /// Unprimed spares: processes with no VM, claimable by any launch.
    generic: usize,
    targets: Vec<Target>,
    /// Counted per (reference, memory), so a shape nobody asked to prepare still shows its demand.
    demand: std::collections::BTreeMap<(String, u64), Demand>,
    /// Same backoff for the unprimed spares.
    spare_failures: u32,
    spare_retry_at: Option<Instant>,
}

struct Pool {
    available: Mutex<Vec<SterileHost>>,
    desired: Mutex<Desired>,
    replenishing: AtomicBool,
}

/// A target the pool is asked to keep, already resolved to a prepared Generation.
pub(crate) struct TargetPlan {
    pub(crate) plan: PrewarmPlan,
    pub(crate) count: usize,
}

/// The target as the pool holds it: its key, its builder, how many.
struct Wanted {
    key: PrimedKey,
    build: Builder,
    count: usize,
}

impl From<TargetPlan> for Wanted {
    fn from(target: TargetPlan) -> Self {
        let key = target.plan.key();
        let plan = target.plan;
        Self {
            key,
            build: Arc::new(move || spawn_explained(Some(&plan))),
            count: target.count,
        }
    }
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        available: Mutex::new(Vec::new()),
        desired: Mutex::new(Desired::default()),
        replenishing: AtomicBool::new(false),
    })
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Creates `target` sterile hosts synchronously before a service accepts traffic.
///
/// With a plan every host is primed for it, as it always was; without one they are unprimed spares.
pub(crate) fn prewarm(target: usize, plan: Option<&PrewarmPlan>) -> Result<(), BackendFailureKind> {
    let targets = plan
        .cloned()
        .map(|plan| TargetPlan {
            plan,
            count: target,
        })
        .into_iter()
        .collect();
    install_sets(targets, if plan.is_some() { 0 } else { target });
    let mut available = locked(&pool().available);
    while available.len() < target {
        available.push(spawn(plan)?);
    }
    Ok(())
}

/// Replaces the desired set: what to keep prepared, per shape, and how many unprimed spares.
///
/// Surplus machines are retired at once; missing ones are built by the background refill, one at
/// a time, so this returns without waiting for them.
pub(crate) fn install_sets(targets: Vec<TargetPlan>, generic: usize) {
    set_desired(targets.into_iter().map(Wanted::from).collect(), generic);
    retire_surplus();
    schedule_refill();
}

fn set_desired(wanted: Vec<Wanted>, generic: usize) {
    let mut desired = locked(&pool().desired);
    let mut next = Vec::with_capacity(wanted.len());
    for want in wanted {
        // A target that survives keeps its history; a new one starts clean.
        let kept = desired
            .targets
            .iter()
            .position(|existing| existing.key == want.key)
            .map(|index| desired.targets.swap_remove(index));
        next.push(Target {
            key: want.key,
            build: want.build,
            count: want.count,
            building: kept.as_ref().is_some_and(|old| old.building),
            last_failure: kept.as_ref().and_then(|old| old.last_failure.clone()),
            failures: kept.as_ref().map_or(0, |old| old.failures),
            retry_at: kept.and_then(|old| old.retry_at),
        });
    }
    desired.targets = next;
    desired.generic = generic;
}

/// Claims one pre-spawned host, falling back to a synchronous spawn only after depletion.
pub(super) fn claim(
    prepared: &PreparedGeneration,
    memory_mib: u64,
) -> Result<SterileHost, BackendFailureKind> {
    let wanted = PrimedKey {
        reference: prepared.reference.clone(),
        generation_id: prepared.id.clone(),
        memory_mib,
    };
    let claimed = take(&wanted);
    // The machine the launch found is what the planner reads: a primed one is a hit, anything
    // else (an unprimed spare, or none at all) means nothing was prepared for this shape.
    {
        let mut desired = locked(&pool().desired);
        let demand = desired
            .demand
            .entry((wanted.reference.clone(), wanted.memory_mib))
            .or_default();
        demand.boot_mib = prepared.manifest.shape.memory_bytes / (1024 * 1024);
        if claimed.as_ref().is_some_and(|host| host.primed.is_some()) {
            demand.primed += 1;
        } else {
            demand.miss += 1;
        }
    }
    let claimed = claimed.map_or_else(|| spawn(None), Ok)?;
    schedule_refill();
    Ok(claimed)
}

/// Removes a machine for the shape from the pool, primed for it or an unprimed spare.
///
/// The machine leaves the pool under the same lock that `retire_surplus` scans under, so every
/// machine is owned by exactly one of them: a claim that wins owns a whole live machine, and one
/// that loses finds none and builds its own. A machine is never terminated while claimed.
fn take(wanted: &PrimedKey) -> Option<SterileHost> {
    let mut available = locked(&pool().available);
    // A machine primed for this shape first, so the hit counter stays honest when primed machines
    // and unprimed spares coexist; a spare only when none is primed for it.
    let index = available
        .iter()
        .rposition(|host| host.primed.as_ref() == Some(wanted))
        .or_else(|| available.iter().rposition(|host| host.primed.is_none()))?;
    Some(available.swap_remove(index))
}
