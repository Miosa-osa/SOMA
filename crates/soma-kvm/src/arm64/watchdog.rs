mod signal;

use std::{
    sync::{
        Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use kvm_ioctls::VcpuFd;

use self::signal::{ProcessSignalGuard, WorkerMaskGuard};
use super::{Arm64BootError, Arm64BootEvidence};

const STARTUP_GRACE: Duration = Duration::from_secs(2);
const CANCELLATION_GRACE: Duration = Duration::from_secs(2);
const JOIN_POLL: Duration = Duration::from_millis(1);
static SIGNAL_LOCK: Mutex<()> = Mutex::new(());

enum WorkerEvent {
    Ready,
    Finished(Result<Arm64BootEvidence, Arm64BootError>),
}

pub(crate) fn run(
    vcpu: VcpuFd,
    expected_sentinel: &'static [u8],
    timeout: Duration,
) -> Result<Arm64BootEvidence, Arm64BootError> {
    let _lock = SIGNAL_LOCK
        .try_lock()
        .map_err(|error| Arm64BootError::at("acquire exclusive ARM64 boot watchdog", error))?;
    let guard = ProcessSignalGuard::install()?;
    let result = run_worker(vcpu, expected_sentinel, timeout, guard.number());
    guard.restore().and(result)
}

fn run_worker(
    vcpu: VcpuFd,
    expected_sentinel: &'static [u8],
    timeout: Duration,
    signal_number: libc::c_int,
) -> Result<Arm64BootEvidence, Arm64BootError> {
    let (sender, receiver) = mpsc::sync_channel(2);
    let worker = thread::Builder::new()
        .name("soma-kvm-vcpu-0".to_owned())
        .spawn(move || worker_main(vcpu, expected_sentinel, signal_number, &sender))
        .map_err(|error| Arm64BootError::at("spawn vCPU watchdog thread", error))?;

    match receiver.recv_timeout(STARTUP_GRACE) {
        Ok(WorkerEvent::Ready) => wait_for_result(worker, &receiver, timeout, signal_number),
        Ok(WorkerEvent::Finished(result)) => {
            join_or_abort(worker)?;
            result
        }
        Err(RecvTimeoutError::Disconnected) => {
            join_or_abort(worker)?;
            Err(Arm64BootError::message(
                "vCPU worker disconnected during watchdog setup",
            ))
        }
        Err(RecvTimeoutError::Timeout) => {
            // Its KVM_RUN mask may not be installed, so neither return nor kick is safe.
            std::process::abort();
        }
    }
}

fn worker_main(
    vcpu: VcpuFd,
    expected_sentinel: &'static [u8],
    signal_number: libc::c_int,
    sender: &SyncSender<WorkerEvent>,
) {
    let result = prepare_and_run(vcpu, expected_sentinel, signal_number, sender);
    let _ignored = sender.send(WorkerEvent::Finished(result));
}

fn prepare_and_run(
    vcpu: VcpuFd,
    expected_sentinel: &'static [u8],
    signal_number: libc::c_int,
    sender: &SyncSender<WorkerEvent>,
) -> Result<Arm64BootEvidence, Arm64BootError> {
    let mask_guard = WorkerMaskGuard::install(&vcpu, signal_number)?;
    let result = sender
        .send(WorkerEvent::Ready)
        .map_err(|error| Arm64BootError::at("report ready vCPU watchdog", error))
        .and_then(|()| super::run_vcpu(vcpu, expected_sentinel));
    mask_guard.restore().and(result)
}

fn wait_for_result(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent>,
    timeout: Duration,
    signal_number: libc::c_int,
) -> Result<Arm64BootEvidence, Arm64BootError> {
    match receiver.recv_timeout(timeout) {
        Ok(WorkerEvent::Finished(result)) => {
            join_or_abort(worker)?;
            result
        }
        Ok(WorkerEvent::Ready) => std::process::abort(),
        Err(RecvTimeoutError::Disconnected) => {
            join_or_abort(worker)?;
            Err(Arm64BootError::message(
                "vCPU worker disconnected before reporting a result",
            ))
        }
        Err(RecvTimeoutError::Timeout) => cancel(worker, receiver, timeout, signal_number),
    }
}

fn cancel(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent>,
    timeout: Duration,
    signal_number: libc::c_int,
) -> Result<Arm64BootEvidence, Arm64BootError> {
    let kick_error = signal::kick(&worker, signal_number).err();
    match receiver.recv_timeout(CANCELLATION_GRACE) {
        Ok(WorkerEvent::Finished(_result)) => {
            join_or_abort(worker)?;
            if let Some(error) = kick_error {
                return Err(Arm64BootError::at("signal timed-out vCPU", error));
            }
            Err(Arm64BootError::message(format!(
                "ARM64 fixture boot timed out after {} seconds",
                timeout.as_secs_f64()
            )))
        }
        Ok(WorkerEvent::Ready) | Err(RecvTimeoutError::Timeout) => std::process::abort(),
        Err(RecvTimeoutError::Disconnected) => {
            join_or_abort(worker)?;
            Err(Arm64BootError::message(
                "timed-out vCPU stopped without reporting cleanup",
            ))
        }
    }
}

fn join_or_abort(worker: JoinHandle<()>) -> Result<(), Arm64BootError> {
    let started = Instant::now();
    while !worker.is_finished() {
        if started.elapsed() >= CANCELLATION_GRACE {
            std::process::abort();
        }
        thread::park_timeout(JOIN_POLL);
    }
    worker
        .join()
        .map_err(|_| Arm64BootError::message("vCPU watchdog thread panicked"))
}
