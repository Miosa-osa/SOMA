//! Machine-host processes prepared before a launch asks for one.
//!
//! A generic host is an already-created process blocked on its private launch socket.
//! A primed host additionally owns an identity-free stopped VM built from verified artifacts.
//! Claiming either preserves the one-process-per-machine boundary, while a primed host also removes
//! restore and device construction from TTI.

use std::{
    io::BufReader,
    os::fd::{AsFd as _, OwnedFd},
    os::unix::net::UnixStream,
    process::{Child, ChildStdout, Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use soma::BackendFailureKind;

use super::{
    channel,
    wire::{InitialWire, PrewarmReady, PrewarmWire},
};
use crate::backend::kvm::prepared::PreparedGeneration;

/// Refill begins after the first command should have completed, so maintenance cannot inflate TTI.
const REFILL_DELAY: Duration = Duration::from_millis(250);
/// How long a pass that built nothing waits before the next, so a shape that cannot be built is
/// retried slowly instead of spinning. The failure is in the readback meanwhile.
const FAILED_REFILL_DELAY: Duration = Duration::from_secs(5);

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
}

#[derive(Clone, Eq, PartialEq)]
struct PrimedKey {
    reference: String,
    generation_id: soma::GenerationId,
    memory_mib: u64,
}

/// One shape the pool is told to keep prepared, and what has happened to it.
struct Target {
    plan: PrewarmPlan,
    key: PrimedKey,
    count: usize,
    building: bool,
    last_failure: Option<String>,
}

/// Launches that wanted one shape: found a prepared machine, or did not.
#[derive(Clone, Copy, Default)]
struct Demand {
    primed: u64,
    miss: u64,
}

#[derive(Default)]
struct Desired {
    /// Unprimed spares: processes with no VM, claimable by any launch.
    generic: usize,
    targets: Vec<Target>,
    /// Counted per (reference, memory), so a shape nobody asked to prepare still shows its demand.
    demand: std::collections::BTreeMap<(String, u64), Demand>,
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

impl PrewarmPlan {
    /// The boot memory the machines for this plan are built with, in MiB.
    pub(crate) const fn memory_mib(&self) -> u64 {
        self.memory_mib
    }
}

/// What the pool holds and has been asked to hold, for one target.
pub(crate) struct TargetSnapshot {
    pub(crate) reference: String,
    pub(crate) generation_id: String,
    pub(crate) memory_mib: u64,
    pub(crate) target: usize,
    pub(crate) prepared: usize,
    pub(crate) building: bool,
    pub(crate) last_failure: Option<String>,
    pub(crate) claimed_primed: u64,
    pub(crate) claimed_miss: u64,
}

/// Launches that wanted a shape no target covers.
pub(crate) struct DemandSnapshot {
    pub(crate) reference: String,
    pub(crate) memory_mib: u64,
    pub(crate) claimed_primed: u64,
    pub(crate) claimed_miss: u64,
}

pub(crate) struct PoolSnapshot {
    pub(crate) targets: Vec<TargetSnapshot>,
    pub(crate) uncovered_demand: Vec<DemandSnapshot>,
    pub(crate) generic_target: usize,
    pub(crate) generic_prepared: usize,
}

/// Creates `target` sterile hosts synchronously before a service accepts traffic.
///
/// With a plan every host is primed for it, as it always was; without one they are unprimed spares.
pub(crate) fn prewarm(target: usize, plan: Option<&PrewarmPlan>) -> Result<(), BackendFailureKind> {
    let pool = pool();
    install_sets(
        plan.cloned()
            .map(|plan| TargetPlan {
                plan,
                count: target,
            })
            .into_iter()
            .collect(),
        if plan.is_some() { 0 } else { target },
    );
    let mut available = pool
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    set_desired(targets, generic);
    retire_surplus();
    schedule_refill();
}

fn set_desired(targets: Vec<TargetPlan>, generic: usize) {
    {
        let mut desired = pool()
            .desired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = Vec::with_capacity(targets.len());
        for wanted in targets {
            let key = key_of(&wanted.plan);
            // A target that survives keeps its history; a new one starts clean.
            let kept = desired
                .targets
                .iter()
                .position(|existing| existing.key == key)
                .map(|index| desired.targets.swap_remove(index));
            next.push(Target {
                plan: wanted.plan,
                key,
                count: wanted.count,
                building: kept.as_ref().is_some_and(|old| old.building),
                last_failure: kept.and_then(|old| old.last_failure),
            });
        }
        desired.targets = next;
        desired.generic = generic;
    }
}

fn key_of(plan: &PrewarmPlan) -> PrimedKey {
    PrimedKey {
        reference: plan.prepared.reference.clone(),
        generation_id: plan.prepared.id.clone(),
        memory_mib: plan.memory_mib,
    }
}

/// Kills every prepared host the desired set no longer wants. Called after the set changes.
fn retire_surplus() {
    let doomed = {
        let desired = pool()
            .desired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut available = pool()
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut kept_primed: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut kept_generic = 0_usize;
        let mut doomed = Vec::new();
        let mut index = 0;
        while index < available.len() {
            let wanted = match available[index].primed.as_ref() {
                None => {
                    kept_generic += 1;
                    kept_generic <= desired.generic
                }
                Some(key) => desired
                    .targets
                    .iter()
                    .find(|target| &target.key == key)
                    .is_some_and(|target| {
                        let id = format!(
                            "{}\0{}\0{}",
                            key.reference,
                            key.generation_id.as_str(),
                            key.memory_mib
                        );
                        let count = kept_primed.entry(id).or_insert(0);
                        *count += 1;
                        *count <= target.count
                    }),
            };
            if wanted {
                index += 1;
            } else {
                doomed.push(available.swap_remove(index));
            }
        }
        doomed
    };
    for mut host in doomed {
        terminate(&mut host.child);
    }
}

/// Counts what the pool holds against what it was asked to hold.
pub(crate) fn snapshot() -> PoolSnapshot {
    let desired = pool()
        .desired
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let available = pool()
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let targets = desired
        .targets
        .iter()
        .map(|target| {
            let demand = desired
                .demand
                .get(&(target.key.reference.clone(), target.key.memory_mib))
                .copied()
                .unwrap_or_default();
            TargetSnapshot {
                reference: target.key.reference.clone(),
                generation_id: target.key.generation_id.as_str().to_owned(),
                memory_mib: target.key.memory_mib,
                target: target.count,
                prepared: available
                    .iter()
                    .filter(|host| host.primed.as_ref() == Some(&target.key))
                    .count(),
                building: target.building,
                last_failure: target.last_failure.clone(),
                claimed_primed: demand.primed,
                claimed_miss: demand.miss,
            }
        })
        .collect();
    let uncovered_demand = desired
        .demand
        .iter()
        .filter(|((reference, memory), _)| {
            !desired.targets.iter().any(|target| {
                &target.key.reference == reference && target.key.memory_mib == *memory
            })
        })
        .map(|((reference, memory), demand)| DemandSnapshot {
            reference: reference.clone(),
            memory_mib: *memory,
            claimed_primed: demand.primed,
            claimed_miss: demand.miss,
        })
        .collect();
    PoolSnapshot {
        targets,
        uncovered_demand,
        generic_target: desired.generic,
        generic_prepared: available
            .iter()
            .filter(|host| host.primed.is_none())
            .count(),
    }
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
    let claimed = {
        let mut available = pool()
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        available
            .iter()
            .rposition(|host| host.primed.as_ref().is_none_or(|key| key == &wanted))
            .map(|index| available.swap_remove(index))
    };
    // The machine the launch found is what the planner reads: a primed one is a hit, anything
    // else (an unprimed spare, or none at all) means nothing was prepared for this shape.
    {
        let mut desired = pool()
            .desired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let demand = desired
            .demand
            .entry((wanted.reference.clone(), wanted.memory_mib))
            .or_default();
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

fn spawn(plan: Option<&PrewarmPlan>) -> Result<SterileHost, BackendFailureKind> {
    let executable = std::env::current_exe().map_err(|_| BackendFailureKind::Unavailable)?;
    let (handoff, child_input) = UnixStream::pair().map_err(|_| BackendFailureKind::Unavailable)?;
    let child = Command::new(executable)
        .arg("machine-host")
        .stdin(Stdio::from(OwnedFd::from(child_input)))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| BackendFailureKind::Unavailable)?;
    let mut host = SterileHost {
        child,
        handoff,
        output: None,
        primed: None,
    };
    if let Some(plan) = plan
        && let Err(failure) = prime(&mut host, plan)
    {
        terminate(&mut host.child);
        return Err(failure);
    }
    Ok(host)
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn prime(host: &mut SterileHost, plan: &PrewarmPlan) -> Result<(), BackendFailureKind> {
    let (manifest, artifacts) = plan
        .prepared
        .handoff()
        .map_err(|_| BackendFailureKind::Unavailable)?;
    let borrowed = artifacts
        .iter()
        .map(std::fs::File::as_fd)
        .collect::<Vec<_>>();
    soma_supervise::send_descriptors(host.handoff.as_fd(), &borrowed)
        .map_err(|_| BackendFailureKind::Unavailable)?;
    channel::write_line(
        &mut host.handoff,
        &InitialWire::Prewarm(PrewarmWire {
            reference: plan.prepared.reference.clone(),
            generation_id: plan.prepared.id.clone(),
            manifest,
            memory_mib: plan.memory_mib,
        }),
    )
    .map_err(|()| BackendFailureKind::Unavailable)?;
    let output = host
        .child
        .stdout
        .take()
        .ok_or(BackendFailureKind::Unavailable)?;
    let mut output = BufReader::new(output);
    if !matches!(
        channel::read_line(&mut output),
        Some(PrewarmReady::Prepared)
    ) {
        return Err(BackendFailureKind::Unavailable);
    }
    host.output = Some(output);
    host.primed = Some(PrimedKey {
        reference: plan.prepared.reference.clone(),
        generation_id: plan.prepared.id.clone(),
        memory_mib: plan.memory_mib,
    });
    Ok(())
}

fn schedule_refill() {
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
            let progressed = refill();
            pool().replenishing.store(false, Ordering::Release);
            if next_missing().is_some() {
                if !progressed {
                    thread::sleep(FAILED_REFILL_DELAY);
                }
                schedule_refill();
            }
        });
    if started.is_err() {
        sterile_pool.replenishing.store(false, Ordering::Release);
    }
}

/// What the next machine to build is, if any: a primed one for a short target, else an unprimed spare.
enum Missing {
    Primed(Box<PrewarmPlan>, Box<PrimedKey>),
    Spare,
}

fn next_missing() -> Option<Missing> {
    let mut desired = pool()
        .desired
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let available = pool()
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for target in &mut desired.targets {
        let have = available
            .iter()
            .filter(|host| host.primed.as_ref() == Some(&target.key))
            .count();
        if have < target.count {
            target.building = true;
            return Some(Missing::Primed(
                Box::new(target.plan.clone()),
                Box::new(target.key.clone()),
            ));
        }
        target.building = false;
    }
    let spares = available
        .iter()
        .filter(|host| host.primed.is_none())
        .count();
    (spares < desired.generic).then_some(Missing::Spare)
}

/// Builds what is missing, one machine at a time; false when the pass built nothing.
fn refill() -> bool {
    let mut progressed = false;
    loop {
        let Some(missing) = next_missing() else {
            return progressed;
        };
        let built = match &missing {
            Missing::Primed(plan, _) => spawn(Some(plan.as_ref())),
            Missing::Spare => spawn(None),
        };
        match (missing, built) {
            (Missing::Primed(_, key), Ok(host)) => {
                progressed = true;
                note_target(key.as_ref(), None);
                pool()
                    .available
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(host);
            }
            (Missing::Spare, Ok(host)) => {
                progressed = true;
                pool()
                    .available
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(host);
            }
            (Missing::Primed(_, key), Err(failure)) => {
                // Stop this pass, so a shape that cannot be built does not spin; the readback
                // carries the reason and the next claim or change schedules another pass.
                note_target(key.as_ref(), Some(format!("{failure:?}")));
                return progressed;
            }
            (Missing::Spare, Err(_)) => return progressed,
        }
    }
}

/// Records the outcome of building one machine for a target, and that it is no longer building.
fn note_target(key: &PrimedKey, failure: Option<String>) {
    let mut desired = pool()
        .desired
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(target) = desired.targets.iter_mut().find(|target| &target.key == key) {
        target.last_failure = failure;
        target.building = false;
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
