//! Runs the vCPU on one dedicated OS thread under a hard deadline.
//!
//! If the guest neither halts nor faults before the deadline, the watchdog kicks the vCPU thread
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
    error::{HaltGuestError, HaltGuestErrorKind, Phase},
    kick::{self, HandlerGuard, RunMaskGuard},
    run::{self, RunOutcome},
};

const STARTUP_GRACE: Duration = Duration::from_secs(2);
const CANCELLATION_GRACE: Duration = Duration::from_secs(2);
const JOIN_POLL: Duration = Duration::from_millis(1);
static PROCESS_HANDLER_LOCK: Mutex<()> = Mutex::new(());

enum WorkerEvent {
    Ready,
    Finished(Result<RunOutcome, HaltGuestError>),
}

/// Runs `vcpu` to completion on a new thread, interrupting it after `timeout`.
///
/// The vCPU descriptor is consumed and dropped on the worker thread before this returns, so the
/// caller can release the VM and guest memory afterwards.
pub(crate) fn run_with_deadline(
    vcpu: VcpuFd,
    timeout: Duration,
) -> Result<RunOutcome, HaltGuestError> {
    if timeout.is_zero() {
        return Err(HaltGuestError::invalid(
            Phase::Run,
            "deadline must be positive",
        ));
    }
    let _lock = PROCESS_HANDLER_LOCK
        .try_lock()
        .map_err(|_| HaltGuestError::invalid(Phase::Run, "another halt-guest proof is running"))?;
    let signal = kick::signal_number()?;
    let handler = HandlerGuard::install(signal)?;
    let result = run_worker(vcpu, timeout, signal);
    drop(handler);
    result
}

fn run_worker(
    vcpu: VcpuFd,
    timeout: Duration,
    signal: libc::c_int,
) -> Result<RunOutcome, HaltGuestError> {
    let (sender, receiver) = mpsc::sync_channel(2);
    let worker = thread::Builder::new()
        .name("soma-kvm-vcpu-0".to_owned())
        .spawn(move || worker_main(vcpu, signal, &sender))
        .map_err(|error| HaltGuestError::new(Phase::Run, kind_from_io(&error)))?;

    match receiver.recv_timeout(STARTUP_GRACE) {
        Ok(WorkerEvent::Ready) => wait_for_result(worker, &receiver, timeout, signal),
        Ok(WorkerEvent::Finished(result)) => {
            join_or_abort(worker)?;
            result
        }
        Err(RecvTimeoutError::Disconnected) => {
            join_or_abort(worker)?;
            Err(HaltGuestError::new(
                Phase::Run,
                HaltGuestErrorKind::WorkerLost,
            ))
        }
        // The worker's KVM_RUN mask may not be installed, so neither a kick nor a return is safe.
        Err(RecvTimeoutError::Timeout) => std::process::abort(),
    }
}

fn worker_main(mut vcpu: VcpuFd, signal: libc::c_int, sender: &SyncSender<WorkerEvent>) {
    let result = match RunMaskGuard::install(&vcpu, signal) {
        Ok(mask) => {
            let result = if sender.send(WorkerEvent::Ready).is_ok() {
                run::run(&mut vcpu)
            } else {
                Err(HaltGuestError::new(
                    Phase::Run,
                    HaltGuestErrorKind::WorkerLost,
                ))
            };
            drop(mask);
            result
        }
        Err(error) => Err(error),
    };
    drop(vcpu);
    let _ignored = sender.send(WorkerEvent::Finished(result));
}

fn wait_for_result(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent>,
    timeout: Duration,
    signal: libc::c_int,
) -> Result<RunOutcome, HaltGuestError> {
    match receiver.recv_timeout(timeout) {
        Ok(WorkerEvent::Finished(result)) => {
            join_or_abort(worker)?;
            result
        }
        Ok(WorkerEvent::Ready) => std::process::abort(),
        Err(RecvTimeoutError::Disconnected) => {
            join_or_abort(worker)?;
            Err(HaltGuestError::new(
                Phase::Run,
                HaltGuestErrorKind::WorkerLost,
            ))
        }
        Err(RecvTimeoutError::Timeout) => cancel(worker, receiver, signal),
    }
}

fn cancel(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent>,
    signal: libc::c_int,
) -> Result<RunOutcome, HaltGuestError> {
    let kick_error = kick::kick(&worker, signal).err();
    match receiver.recv_timeout(CANCELLATION_GRACE) {
        Ok(WorkerEvent::Finished(result)) => {
            join_or_abort(worker)?;
            if let Some(error) = kick_error {
                return Err(error);
            }
            match result {
                Ok(outcome) => Ok(outcome),
                Err(_) => Err(HaltGuestError::new(Phase::Run, HaltGuestErrorKind::Timeout)),
            }
        }
        // A kicked worker that neither finishes nor disconnects may still own a live vCPU.
        Ok(WorkerEvent::Ready) | Err(RecvTimeoutError::Timeout) => std::process::abort(),
        Err(RecvTimeoutError::Disconnected) => {
            join_or_abort(worker)?;
            Err(HaltGuestError::new(
                Phase::Join,
                HaltGuestErrorKind::WorkerLost,
            ))
        }
    }
}

fn join_or_abort(worker: JoinHandle<()>) -> Result<(), HaltGuestError> {
    let started = Instant::now();
    while !worker.is_finished() {
        if started.elapsed() >= CANCELLATION_GRACE {
            std::process::abort();
        }
        thread::park_timeout(JOIN_POLL);
    }
    worker
        .join()
        .map_err(|_| HaltGuestError::new(Phase::Join, HaltGuestErrorKind::WorkerLost))
}

fn kind_from_io(error: &std::io::Error) -> HaltGuestErrorKind {
    HaltGuestErrorKind::Os(error.raw_os_error().unwrap_or(0))
}
