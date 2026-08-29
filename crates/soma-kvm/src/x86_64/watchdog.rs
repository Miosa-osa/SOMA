//! Runs the vCPU on one dedicated OS thread under a hard deadline.
//!
//! If the guest neither stops nor faults before the deadline, the watchdog kicks the vCPU thread
//! out of `KVM_RUN`. If the thread still cannot be joined within a bounded grace period the
//! process aborts, because releasing guest memory under a live vCPU is never acceptable.
//! [`VcpuRun`] separates starting the thread from waiting for it so a sandbox can drive its
//! control session while the guest runs and still reclaim the vCPU through the same path.

mod worker;

use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use kvm_ioctls::VcpuFd;

use self::worker::{Control, cancel, finish, join_then, worker_main};
use super::{
    error::{MachineError, MachineErrorKind, Phase},
    kick::{self, HandlerGuard},
    mmio::MmioDispatch,
    ports::PortBus,
    run::GuestExit,
};

const STARTUP_GRACE: Duration = Duration::from_secs(2);
pub(super) const CANCELLATION_GRACE: Duration = Duration::from_secs(2);
static PROCESS_HANDLER_LOCK: Mutex<()> = Mutex::new(());

enum WorkerEvent {
    Ready,
    Finished(
        Box<PortBus>,
        Option<Box<MmioDispatch>>,
        Result<GuestExit, MachineError>,
        Option<VcpuFd>,
    ),
}

/// The run result together with the port bus and MMIO dispatcher, which are returned even when
/// the run failed so callers can retain the captured console and counters for diagnosis.
pub(crate) struct RunReport {
    pub(crate) bus: Option<Box<PortBus>>,
    pub(crate) mmio: Option<Box<MmioDispatch>>,
    pub(crate) result: Result<GuestExit, MachineError>,
    /// vCPU 0, returned only by a pause so its state can be read outside `KVM_RUN`.
    ///
    /// The holder must drop it before the VM and guest memory it belongs to.
    pub(crate) vcpu: Option<VcpuFd>,
}

impl RunReport {
    fn lost(phase: Phase) -> Self {
        Self {
            bus: None,
            mmio: None,
            result: Err(MachineError::new(phase, MachineErrorKind::WorkerLost)),
            vcpu: None,
        }
    }

    fn failed(bus: PortBus, mmio: Option<MmioDispatch>, error: MachineError) -> Self {
        Self {
            bus: Some(Box::new(bus)),
            mmio: mmio.map(Box::new),
            result: Err(error),
            vcpu: None,
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
    mmio: Option<MmioDispatch>,
    sentinel: Option<Vec<u8>>,
    timeout: Duration,
) -> RunReport {
    if timeout.is_zero() {
        return RunReport::failed(
            bus,
            mmio,
            MachineError::invalid(Phase::Run, "deadline must be positive"),
        );
    }
    match VcpuRun::start(vcpu, bus, mmio, sentinel) {
        Ok(run) => run.wait(timeout),
        Err(report) => report,
    }
}

/// A vCPU thread that has entered `KVM_RUN` with its interrupt mask installed.
pub(crate) struct VcpuRun {
    worker: JoinHandle<()>,
    receiver: Receiver<WorkerEvent>,
    signal: libc::c_int,
    pause: Arc<AtomicBool>,
    _handler: HandlerGuard,
    _lock: MutexGuard<'static, ()>,
}

impl VcpuRun {
    /// Starts the worker and waits until it has entered its run mask.
    ///
    /// # Errors
    ///
    /// Returns a report carrying the bus and dispatcher when the thread could not start; a
    /// worker that neither reports readiness nor finishes within the startup grace aborts
    /// the process because its run mask state is unknown.
    pub(crate) fn start(
        vcpu: VcpuFd,
        bus: PortBus,
        mmio: Option<MmioDispatch>,
        sentinel: Option<Vec<u8>>,
    ) -> Result<Self, RunReport> {
        // The interrupt handler is process-wide, so concurrent proofs in one process serialize
        // here instead of failing. A poisoned lock only means a previous proof panicked after
        // installing its handler; the guard restored it, so the lock is safe to reuse.
        let lock = PROCESS_HANDLER_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let signal = match kick::signal_number() {
            Ok(signal) => signal,
            Err(error) => return Err(RunReport::failed(bus, mmio, error)),
        };
        let handler = match HandlerGuard::install(signal) {
            Ok(handler) => handler,
            Err(error) => return Err(RunReport::failed(bus, mmio, error)),
        };
        let (sender, receiver) = mpsc::sync_channel(2);
        let bus = Box::new(bus);
        let mmio = mmio.map(Box::new);
        let pause = Arc::new(AtomicBool::new(false));
        let worker_pause = Arc::clone(&pause);
        let worker = match thread::Builder::new()
            .name("soma-kvm-vcpu-0".to_owned())
            .spawn(move || {
                worker_main(
                    vcpu,
                    bus,
                    mmio,
                    sentinel.as_deref(),
                    &Control {
                        signal,
                        pause: &worker_pause,
                        sender: &sender,
                    },
                );
            }) {
            Ok(worker) => worker,
            Err(error) => {
                return Err(RunReport {
                    bus: None,
                    mmio: None,
                    result: Err(MachineError::io(Phase::Run, &error)),
                    vcpu: None,
                });
            }
        };
        match receiver.recv_timeout(STARTUP_GRACE) {
            Ok(WorkerEvent::Ready) => Ok(Self {
                worker,
                receiver,
                signal,
                pause,
                _handler: handler,
                _lock: lock,
            }),
            Ok(WorkerEvent::Finished(bus, mmio, result, vcpu)) => {
                Err(finish(worker, bus, mmio, result, vcpu))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(join_then(worker, RunReport::lost(Phase::Run)))
            }
            // The worker's KVM_RUN mask may not be installed, so neither a kick nor a return is safe.
            Err(RecvTimeoutError::Timeout) => std::process::abort(),
        }
    }

    /// Waits for the guest to stop, kicking the vCPU out of `KVM_RUN` after `timeout`.
    pub(crate) fn wait(self, timeout: Duration) -> RunReport {
        let Self {
            worker,
            receiver,
            signal,
            pause: _pause,
            _handler,
            _lock,
        } = self;
        match receiver.recv_timeout(timeout) {
            Ok(WorkerEvent::Finished(bus, mmio, result, vcpu)) => {
                finish(worker, bus, mmio, result, vcpu)
            }
            Ok(WorkerEvent::Ready) => std::process::abort(),
            Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Run)),
            Err(RecvTimeoutError::Timeout) => cancel(worker, &receiver, signal),
        }
    }

    /// Kicks the vCPU out of `KVM_RUN` at a safe point and reclaims its descriptor.
    ///
    /// The guest is not stopped: KVM has already saved every architectural register, so the
    /// returned [`RunReport::vcpu`] can be read with `KVM_GET_*` while nothing runs it.
    ///
    /// A worker that neither reports nor disconnects within `grace` may still own a live
    /// vCPU, so the process aborts rather than releasing guest memory underneath it.
    pub(crate) fn pause(self, grace: Duration) -> RunReport {
        let Self {
            worker,
            receiver,
            signal,
            pause,
            _handler,
            _lock,
        } = self;
        pause.store(true, Ordering::Release);
        if let Err(error) = kick::kick(&worker, signal) {
            return join_then(
                worker,
                RunReport {
                    bus: None,
                    mmio: None,
                    result: Err(error),
                    vcpu: None,
                },
            );
        }
        match receiver.recv_timeout(grace) {
            Ok(WorkerEvent::Finished(bus, mmio, result, vcpu)) => {
                finish(worker, bus, mmio, result, vcpu)
            }
            Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Join)),
            Ok(WorkerEvent::Ready) | Err(RecvTimeoutError::Timeout) => std::process::abort(),
        }
    }
}
