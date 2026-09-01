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
        atomic::{AtomicBool, AtomicUsize, Ordering},
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

struct Pool {
    available: Mutex<Vec<SterileHost>>,
    plan: Mutex<Option<PrewarmPlan>>,
    target: AtomicUsize,
    replenishing: AtomicBool,
}

/// Creates `target` sterile hosts synchronously before a service accepts traffic.
pub(crate) fn prewarm(target: usize, plan: Option<&PrewarmPlan>) -> Result<(), BackendFailureKind> {
    let pool = pool();
    pool.target.store(target, Ordering::Release);
    *pool
        .plan
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = plan.cloned();
    let mut available = pool
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    while available.len() < target {
        available.push(spawn(plan)?);
    }
    Ok(())
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
    }
    .map_or_else(|| spawn(None), Ok)?;
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
            refill();
            pool().replenishing.store(false, Ordering::Release);
            if pool()
                .available
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
                < pool().target.load(Ordering::Acquire)
            {
                schedule_refill();
            }
        });
    if started.is_err() {
        sterile_pool.replenishing.store(false, Ordering::Release);
    }
}

fn refill() {
    loop {
        let missing = {
            let available = pool()
                .available
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pool()
                .target
                .load(Ordering::Acquire)
                .saturating_sub(available.len())
        };
        if missing == 0 {
            return;
        }
        let plan = pool()
            .plan
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Ok(host) = spawn(plan.as_ref()) else {
            return;
        };
        pool()
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(host);
    }
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        available: Mutex::new(Vec::new()),
        plan: Mutex::new(None),
        target: AtomicUsize::new(0),
        replenishing: AtomicBool::new(false),
    })
}
