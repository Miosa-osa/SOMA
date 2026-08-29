//! Runs the vCPU on one dedicated OS thread under a hard deadline.
//!
//! If the guest neither stops nor faults before the deadline, the watchdog kicks the vCPU thread
//! out of `KVM_RUN`. If the thread still cannot be joined within a bounded grace period the
//! process aborts, because releasing guest memory under a live vCPU is never acceptable.

use std::{
    sync::{
        Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use kvm_ioctls::VcpuFd;

use super::{
    error::{MachineError, MachineErrorKind, Phase},
    kick::{self, HandlerGuard, RunMaskGuard},
    ports::PortBus,
    run::{self, GuestExit},
};

const STARTUP_GRACE: Duration = Duration::from_secs(2);
const CANCELLATION_GRACE: Duration = Duration::from_secs(2);
const JOIN_POLL: Duration = Duration::from_millis(1);
static PROCESS_HANDLER_LOCK: Mutex<()> = Mutex::new(());

enum WorkerEvent {
    Ready,
    Finished(Box<PortBus>, Result<GuestExit, MachineError>),
}

/// The run result together with the port bus, which is returned even when the run failed so
/// callers can retain the captured console for diagnosis.
pub(crate) struct RunReport {
    pub(crate) bus: Option<Box<PortBus>>,
    pub(crate) result: Result<GuestExit, MachineError>,
}

impl RunReport {
    fn lost(phase: Phase) -> Self {
        Self {
            bus: None,
            result: Err(MachineError::new(phase, MachineErrorKind::WorkerLost)),
        }
    }
}

/// Runs `vcpu` to completion on a new thread, interrupting it after `timeout`.
///
/// The vCPU descriptor is consumed and dropped on the worker thread before this returns, so the
/// caller can release the VM and guest memory afterwards.
pub(crate) fn run_with_deadline(
    vcpu: VcpuFd,
    bus: PortBus,
    sentinel: Option<Vec<u8>>,
    timeout: Duration,
) -> RunReport {
    if timeout.is_zero() {
        return RunReport {
            bus: Some(Box::new(bus)),
            result: Err(MachineError::invalid(
                Phase::Run,
                "deadline must be positive",
            )),
        };
    }
    // The interrupt handler is process-wide, so concurrent proofs in one process serialize here
    // instead of failing. A poisoned lock only means a previous proof panicked after installing
    // its handler; the guard restored it, so the lock is safe to reuse.
    let _lock = PROCESS_HANDLER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let signal = match kick::signal_number() {
        Ok(signal) => signal,
        Err(error) => {
            return RunReport {
                bus: Some(Box::new(bus)),
                result: Err(error),
            };
        }
    };
    let handler = match HandlerGuard::install(signal) {
        Ok(handler) => handler,
        Err(error) => {
            return RunReport {
                bus: Some(Box::new(bus)),
                result: Err(error),
            };
        }
    };
    let report = run_worker(vcpu, Box::new(bus), sentinel, timeout, signal);
    drop(handler);
    report
}

fn run_worker(
    vcpu: VcpuFd,
    bus: Box<PortBus>,
    sentinel: Option<Vec<u8>>,
    timeout: Duration,
    signal: libc::c_int,
) -> RunReport {
    let (sender, receiver) = mpsc::sync_channel(2);
    let worker = match thread::Builder::new()
        .name("soma-kvm-vcpu-0".to_owned())
        .spawn(move || worker_main(vcpu, bus, sentinel.as_deref(), signal, &sender))
    {
        Ok(worker) => worker,
        Err(error) => {
            return RunReport {
                bus: None,
                result: Err(MachineError::io(Phase::Run, &error)),
            };
        }
    };

    match receiver.recv_timeout(STARTUP_GRACE) {
        Ok(WorkerEvent::Ready) => wait_for_result(worker, &receiver, timeout, signal),
        Ok(WorkerEvent::Finished(bus, result)) => finish(worker, bus, result),
        Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Run)),
        // The worker's KVM_RUN mask may not be installed, so neither a kick nor a return is safe.
        Err(RecvTimeoutError::Timeout) => std::process::abort(),
    }
}

fn worker_main(
    mut vcpu: VcpuFd,
    mut bus: Box<PortBus>,
    sentinel: Option<&[u8]>,
    signal: libc::c_int,
    sender: &SyncSender<WorkerEvent>,
) {
    let result = match RunMaskGuard::install(&vcpu, signal) {
        Ok(mask) => {
            let result = if sender.send(WorkerEvent::Ready).is_ok() {
                run::run(&mut vcpu, &mut bus, sentinel)
            } else {
                Err(MachineError::new(Phase::Run, MachineErrorKind::WorkerLost))
            };
            drop(mask);
            result
        }
        Err(error) => Err(error),
    };
    drop(vcpu);
    let _ignored = sender.send(WorkerEvent::Finished(bus, result));
}

fn wait_for_result(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent>,
    timeout: Duration,
    signal: libc::c_int,
) -> RunReport {
    match receiver.recv_timeout(timeout) {
        Ok(WorkerEvent::Finished(bus, result)) => finish(worker, bus, result),
        Ok(WorkerEvent::Ready) => std::process::abort(),
        Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Run)),
        Err(RecvTimeoutError::Timeout) => cancel(worker, receiver, signal),
    }
}

fn cancel(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent>,
    signal: libc::c_int,
) -> RunReport {
    let kick_error = kick::kick(&worker, signal).err();
    match receiver.recv_timeout(CANCELLATION_GRACE) {
        Ok(WorkerEvent::Finished(bus, result)) => {
            let result = match (kick_error, result) {
                (Some(error), _) => Err(error),
                (None, Ok(exit)) => Ok(exit),
                (None, Err(_)) => Err(MachineError::new(Phase::Run, MachineErrorKind::Timeout)),
            };
            finish(worker, bus, result)
        }
        // A kicked worker that neither finishes nor disconnects may still own a live vCPU.
        Ok(WorkerEvent::Ready) | Err(RecvTimeoutError::Timeout) => std::process::abort(),
        Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Join)),
    }
}

fn finish(
    worker: JoinHandle<()>,
    bus: Box<PortBus>,
    result: Result<GuestExit, MachineError>,
) -> RunReport {
    join_then(
        worker,
        RunReport {
            bus: Some(bus),
            result,
        },
    )
}

/// Joins the worker within the grace period; a join failure replaces the pending result.
fn join_then(worker: JoinHandle<()>, report: RunReport) -> RunReport {
    let started = Instant::now();
    while !worker.is_finished() {
        if started.elapsed() >= CANCELLATION_GRACE {
            std::process::abort();
        }
        thread::park_timeout(JOIN_POLL);
    }
    if worker.join().is_err() {
        return RunReport {
            bus: report.bus,
            result: Err(MachineError::new(Phase::Join, MachineErrorKind::WorkerLost)),
        };
    }
    report
}
