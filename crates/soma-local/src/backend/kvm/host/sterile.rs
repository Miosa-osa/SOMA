//! Machine-host processes prepared before a launch asks for one.
//!
//! A sterile host owns no VM, guest memory, Instance identity, or artifact descriptor.
//! It is only an already-created process blocked on its private launch socket.
//! Claiming one removes process creation from TTI without weakening the one-process-per-machine
//! boundary or pretending that a VM was warm.

use std::{
    os::fd::OwnedFd,
    os::unix::net::UnixStream,
    process::{Child, Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use soma::BackendFailureKind;

/// Refill begins after the first command should have completed, so maintenance cannot inflate TTI.
const REFILL_DELAY: Duration = Duration::from_millis(250);

pub(super) struct SterileHost {
    pub(super) child: Child,
    pub(super) handoff: UnixStream,
}

struct Pool {
    available: Mutex<Vec<SterileHost>>,
    target: AtomicUsize,
    replenishing: AtomicBool,
}

/// Creates `target` sterile hosts synchronously before a service accepts traffic.
pub(crate) fn prewarm(target: usize) -> Result<(), BackendFailureKind> {
    let pool = pool();
    pool.target.store(target, Ordering::Release);
    let mut available = pool
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    while available.len() < target {
        available.push(spawn()?);
    }
    Ok(())
}

/// Claims one pre-spawned host, falling back to a synchronous spawn only after depletion.
pub(super) fn claim() -> Result<SterileHost, BackendFailureKind> {
    let claimed = pool()
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .map_or_else(spawn, Ok)?;
    schedule_refill();
    Ok(claimed)
}

fn spawn() -> Result<SterileHost, BackendFailureKind> {
    let executable = std::env::current_exe().map_err(|_| BackendFailureKind::Unavailable)?;
    let (handoff, child_input) = UnixStream::pair().map_err(|_| BackendFailureKind::Unavailable)?;
    let child = Command::new(executable)
        .arg("machine-host")
        .stdin(Stdio::from(OwnedFd::from(child_input)))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| BackendFailureKind::Unavailable)?;
    Ok(SterileHost { child, handoff })
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
        let Ok(host) = spawn() else {
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
        target: AtomicUsize::new(0),
        replenishing: AtomicBool::new(false),
    })
}
