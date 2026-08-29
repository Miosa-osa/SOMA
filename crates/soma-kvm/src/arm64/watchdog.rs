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

enum WorkerEvent<T> {
    Ready,
    Armed(Instant),
    Finished(Result<T, Arm64BootError>),
}

pub(crate) struct DeadlineArm<T> {
    sender: SyncSender<WorkerEvent<T>>,
    armed: bool,
}

impl<T> DeadlineArm<T> {
    pub(crate) fn arm(&mut self, timeout: Duration) -> Result<(), Arm64BootError> {
        if self.armed || timeout.is_zero() {
            return Err(Arm64BootError::message(
                "command watchdog deadline was armed invalidly",
            ));
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Arm64BootError::message("command watchdog deadline overflow"))?;
        self.sender
            .send(WorkerEvent::Armed(deadline))
            .map_err(|error| Arm64BootError::at("arm command watchdog deadline", error))?;
        self.armed = true;
        Ok(())
    }
}

pub(crate) enum TaskOutcome<T> {
    Finished(T),
    TimedOut,
}

pub(crate) fn run(
    vcpu: VcpuFd,
    expected_sentinel: &'static [u8],
    timeout: Duration,
) -> Result<Arm64BootEvidence, Arm64BootError> {
    match run_task(vcpu, timeout, move |vcpu, _deadline| {
        super::run_vcpu(vcpu, expected_sentinel)
    })? {
        TaskOutcome::Finished(evidence) => Ok(evidence),
        TaskOutcome::TimedOut => Err(Arm64BootError::message(format!(
            "ARM64 fixture boot timed out after {} seconds",
            timeout.as_secs_f64()
        ))),
    }
}

pub(crate) fn run_task<T, F>(
    vcpu: VcpuFd,
    timeout: Duration,
    task: F,
) -> Result<TaskOutcome<T>, Arm64BootError>
where
    T: Send + 'static,
    F: FnOnce(VcpuFd, &mut DeadlineArm<T>) -> Result<T, Arm64BootError> + Send + 'static,
{
    let _lock = SIGNAL_LOCK
        .try_lock()
        .map_err(|error| Arm64BootError::at("acquire exclusive ARM64 boot watchdog", error))?;
    let guard = ProcessSignalGuard::install()?;
    let result = run_worker(vcpu, timeout, guard.number(), task);
    guard.restore().and(result)
}

fn run_worker<T, F>(
    vcpu: VcpuFd,
    timeout: Duration,
    signal_number: libc::c_int,
    task: F,
) -> Result<TaskOutcome<T>, Arm64BootError>
where
    T: Send + 'static,
    F: FnOnce(VcpuFd, &mut DeadlineArm<T>) -> Result<T, Arm64BootError> + Send + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(2);
    let worker = thread::Builder::new()
        .name("soma-kvm-vcpu-0".to_owned())
        .spawn(move || worker_main(vcpu, signal_number, &sender, task))
        .map_err(|error| Arm64BootError::at("spawn vCPU watchdog thread", error))?;

    match receiver.recv_timeout(STARTUP_GRACE) {
        Ok(WorkerEvent::Ready) => wait_for_result(worker, &receiver, timeout, signal_number),
        Ok(WorkerEvent::Finished(result)) => {
            join_or_abort(worker)?;
            result.map(TaskOutcome::Finished)
        }
        Ok(WorkerEvent::Armed(_)) => std::process::abort(),
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

fn worker_main<T, F>(
    vcpu: VcpuFd,
    signal_number: libc::c_int,
    sender: &SyncSender<WorkerEvent<T>>,
    task: F,
) where
    F: FnOnce(VcpuFd, &mut DeadlineArm<T>) -> Result<T, Arm64BootError>,
{
    let result = prepare_and_run(vcpu, signal_number, sender, task);
    let _ignored = sender.send(WorkerEvent::Finished(result));
}

fn prepare_and_run<T, F>(
    vcpu: VcpuFd,
    signal_number: libc::c_int,
    sender: &SyncSender<WorkerEvent<T>>,
    task: F,
) -> Result<T, Arm64BootError>
where
    F: FnOnce(VcpuFd, &mut DeadlineArm<T>) -> Result<T, Arm64BootError>,
{
    let mask_guard = WorkerMaskGuard::install(&vcpu, signal_number)?;
    let mut deadline = DeadlineArm {
        sender: sender.clone(),
        armed: false,
    };
    let result = sender
        .send(WorkerEvent::Ready)
        .map_err(|error| Arm64BootError::at("report ready vCPU watchdog", error))
        .and_then(|()| task(vcpu, &mut deadline));
    mask_guard.restore().and(result)
}

fn wait_for_result<T>(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent<T>>,
    timeout: Duration,
    signal_number: libc::c_int,
) -> Result<TaskOutcome<T>, Arm64BootError> {
    let mut deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| Arm64BootError::message("initial watchdog deadline overflow"))?;
    let mut armed = false;
    loop {
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(WorkerEvent::Finished(result)) => {
                join_or_abort(worker)?;
                return result.map(TaskOutcome::Finished);
            }
            Ok(WorkerEvent::Armed(next_deadline)) if !armed => {
                armed = true;
                deadline = next_deadline;
            }
            Ok(WorkerEvent::Ready | WorkerEvent::Armed(_)) => std::process::abort(),
            Err(RecvTimeoutError::Disconnected) => {
                join_or_abort(worker)?;
                return Err(Arm64BootError::message(
                    "vCPU worker disconnected before reporting a result",
                ));
            }
            Err(RecvTimeoutError::Timeout) => {
                return cancel(worker, receiver, signal_number);
            }
        }
    }
}

fn cancel<T>(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent<T>>,
    signal_number: libc::c_int,
) -> Result<TaskOutcome<T>, Arm64BootError> {
    let kick_error = signal::kick(&worker, signal_number).err();
    let started = Instant::now();
    loop {
        let Some(remaining) = CANCELLATION_GRACE.checked_sub(started.elapsed()) else {
            std::process::abort();
        };
        match receiver.recv_timeout(remaining) {
            Ok(WorkerEvent::Finished(_result)) => {
                join_or_abort(worker)?;
                if let Some(error) = kick_error {
                    return Err(Arm64BootError::at("signal timed-out vCPU", error));
                }
                return Ok(TaskOutcome::TimedOut);
            }
            Ok(WorkerEvent::Armed(_)) => {}
            Ok(WorkerEvent::Ready) | Err(RecvTimeoutError::Timeout) => std::process::abort(),
            Err(RecvTimeoutError::Disconnected) => {
                join_or_abort(worker)?;
                return Err(Arm64BootError::message(
                    "timed-out vCPU stopped without reporting cleanup",
                ));
            }
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
