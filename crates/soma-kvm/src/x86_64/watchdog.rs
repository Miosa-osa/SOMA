//! Runs the vCPU on one dedicated OS thread under a hard deadline.
//!
//! If the guest neither stops nor faults before the deadline, the watchdog kicks the vCPU thread
//! out of `KVM_RUN`. If the thread still cannot be joined within a bounded grace period the
//! process aborts, because releasing guest memory under a live vCPU is never acceptable.
//! [`VcpuRun`] separates starting the thread from waiting for it so a sandbox can drive its
//! control session while the guest runs and still reclaim the vCPU through the same path.

use std::{
    sync::{
        Mutex, MutexGuard,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use kvm_ioctls::VcpuFd;

use super::{
    error::{MachineError, MachineErrorKind, Phase},
    kick::{self, HandlerGuard, RunMaskGuard},
    mmio::MmioDispatch,
    ports::PortBus,
    run::{self, GuestExit},
};

const STARTUP_GRACE: Duration = Duration::from_secs(2);
const CANCELLATION_GRACE: Duration = Duration::from_secs(2);
const JOIN_POLL: Duration = Duration::from_millis(1);
static PROCESS_HANDLER_LOCK: Mutex<()> = Mutex::new(());

enum WorkerEvent {
    Ready,
    Finished(
        Box<PortBus>,
        Option<Box<MmioDispatch>>,
        Result<GuestExit, MachineError>,
    ),
}

/// The run result together with the port bus and MMIO dispatcher, which are returned even when
/// the run failed so callers can retain the captured console and counters for diagnosis.
pub(crate) struct RunReport {
    pub(crate) bus: Option<Box<PortBus>>,
    pub(crate) mmio: Option<Box<MmioDispatch>>,
    pub(crate) result: Result<GuestExit, MachineError>,
}

impl RunReport {
    fn lost(phase: Phase) -> Self {
        Self {
            bus: None,
            mmio: None,
            result: Err(MachineError::new(phase, MachineErrorKind::WorkerLost)),
        }
    }

    fn failed(bus: PortBus, mmio: Option<MmioDispatch>, error: MachineError) -> Self {
        Self {
            bus: Some(Box::new(bus)),
            mmio: mmio.map(Box::new),
            result: Err(error),
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
        let worker = match thread::Builder::new()
            .name("soma-kvm-vcpu-0".to_owned())
            .spawn(move || worker_main(vcpu, bus, mmio, sentinel.as_deref(), signal, &sender))
        {
            Ok(worker) => worker,
            Err(error) => {
                return Err(RunReport {
                    bus: None,
                    mmio: None,
                    result: Err(MachineError::io(Phase::Run, &error)),
                });
            }
        };
        match receiver.recv_timeout(STARTUP_GRACE) {
            Ok(WorkerEvent::Ready) => Ok(Self {
                worker,
                receiver,
                signal,
                _handler: handler,
                _lock: lock,
            }),
            Ok(WorkerEvent::Finished(bus, mmio, result)) => Err(finish(worker, bus, mmio, result)),
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
            _handler,
            _lock,
        } = self;
        match receiver.recv_timeout(timeout) {
            Ok(WorkerEvent::Finished(bus, mmio, result)) => finish(worker, bus, mmio, result),
            Ok(WorkerEvent::Ready) => std::process::abort(),
            Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Run)),
            Err(RecvTimeoutError::Timeout) => cancel(worker, &receiver, signal),
        }
    }
}

fn worker_main(
    mut vcpu: VcpuFd,
    mut bus: Box<PortBus>,
    mut mmio: Option<Box<MmioDispatch>>,
    sentinel: Option<&[u8]>,
    signal: libc::c_int,
    sender: &SyncSender<WorkerEvent>,
) {
    let result = match RunMaskGuard::install(&vcpu, signal) {
        Ok(mask) => {
            let result = if sender.send(WorkerEvent::Ready).is_ok() {
                run::run(&mut vcpu, &mut bus, mmio.as_deref_mut(), sentinel)
            } else {
                Err(MachineError::new(Phase::Run, MachineErrorKind::WorkerLost))
            };
            drop(mask);
            result
        }
        Err(error) => Err(error),
    };
    drop(vcpu);
    if let Some(dispatch) = mmio.as_deref() {
        dispatch.finish();
    }
    let _ignored = sender.send(WorkerEvent::Finished(bus, mmio, result));
}

fn cancel(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent>,
    signal: libc::c_int,
) -> RunReport {
    let kick_error = kick::kick(&worker, signal).err();
    match receiver.recv_timeout(CANCELLATION_GRACE) {
        Ok(WorkerEvent::Finished(bus, mmio, result)) => {
            let result = match (kick_error, result) {
                (Some(error), _) => Err(error),
                (None, Ok(exit)) => Ok(exit),
                (None, Err(_)) => Err(MachineError::new(Phase::Run, MachineErrorKind::Timeout)),
            };
            finish(worker, bus, mmio, result)
        }
        // A kicked worker that neither finishes nor disconnects may still own a live vCPU.
        Ok(WorkerEvent::Ready) | Err(RecvTimeoutError::Timeout) => std::process::abort(),
        Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Join)),
    }
}

fn finish(
    worker: JoinHandle<()>,
    bus: Box<PortBus>,
    mmio: Option<Box<MmioDispatch>>,
    result: Result<GuestExit, MachineError>,
) -> RunReport {
    join_then(
        worker,
        RunReport {
            bus: Some(bus),
            mmio,
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
            mmio: report.mmio,
            result: Err(MachineError::new(Phase::Join, MachineErrorKind::WorkerLost)),
        };
    }
    report
}
