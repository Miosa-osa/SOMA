//! Runs each vCPU of one machine on its own dedicated OS thread under a hard deadline.
//!
//! If a guest neither stops nor faults before the deadline, the watchdog kicks that vCPU thread
//! out of `KVM_RUN`. If a thread still cannot be joined within a bounded grace period the
//! process aborts, because releasing guest memory under a live vCPU is never acceptable.
//!
//! The interrupt handler is process-wide, so it is installed once per machine and every worker
//! thread shares it: a second installation would replace the first and leave the earlier threads
//! unkickable. [`RunContext`] holds that handler and the process-wide serialization guard;
//! [`VcpuRun`] is one worker, and a machine with several vCPUs holds one per processor.

mod worker;

use std::{
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
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
    exits::ExitLedger,
    kick::{self, HandlerGuard},
    mmio::{MmioCounters, MmioDispatch},
    ports::PortBusHandle,
    run::GuestExit,
};

const STARTUP_GRACE: Duration = Duration::from_secs(2);
pub(super) const CANCELLATION_GRACE: Duration = Duration::from_secs(2);
static PROCESS_HANDLER_LOCK: Mutex<()> = Mutex::new(());

enum WorkerEvent {
    Ready,
    Finished(
        Option<Box<MmioDispatch>>,
        Result<GuestExit, MachineError>,
        Option<VcpuFd>,
    ),
}

/// The process-wide interrupt handler and serialization guard every worker of one machine shares.
pub(crate) struct RunContext {
    signal: libc::c_int,
    _handler: HandlerGuard,
    _lock: MutexGuard<'static, ()>,
}

impl RunContext {
    /// Installs the handler once and holds the process-wide guard for the whole run.
    ///
    /// A poisoned lock only means a previous proof panicked after installing its handler; the
    /// guard restored it, so the lock is safe to reuse.
    ///
    /// # Errors
    ///
    /// Returns the signal-selection or handler-installation failure.
    pub(crate) fn acquire() -> Result<Self, MachineError> {
        let lock = PROCESS_HANDLER_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let signal = kick::signal_number()?;
        let handler = HandlerGuard::install(signal)?;
        Ok(Self {
            signal,
            _handler: handler,
            _lock: lock,
        })
    }
}

/// The run result together with the MMIO dispatcher, which is returned even when the run failed
/// so callers can retain the counters for diagnosis.
pub(crate) struct RunReport {
    pub(crate) mmio: Option<Box<MmioDispatch>>,
    pub(crate) result: Result<GuestExit, MachineError>,
    /// The vCPU, returned only by a pause so its state can be read outside `KVM_RUN`.
    ///
    /// The holder must drop it before the VM and guest memory it belongs to.
    pub(crate) vcpu: Option<VcpuFd>,
}

impl RunReport {
    fn lost(phase: Phase) -> Self {
        Self {
            mmio: None,
            result: Err(MachineError::new(phase, MachineErrorKind::WorkerLost)),
            vcpu: None,
        }
    }

    fn failed(mmio: Option<MmioDispatch>, error: MachineError) -> Self {
        Self {
            mmio: mmio.map(Box::new),
            result: Err(error),
            vcpu: None,
        }
    }
}

/// Sums the MMIO counters over every dispatcher a set of reports returned.
#[must_use]
pub(crate) fn total_mmio(reports: &[RunReport]) -> MmioCounters {
    reports
        .iter()
        .filter_map(|report| report.mmio.as_deref())
        .fold(MmioCounters::default(), |total, dispatch| {
            let counters = dispatch.counters();
            MmioCounters {
                reads: total.reads.saturating_add(counters.reads),
                writes: total.writes.saturating_add(counters.writes),
                transport_violations: total
                    .transport_violations
                    .saturating_add(counters.transport_violations),
                notify_exits: total.notify_exits.saturating_add(counters.notify_exits),
            }
        })
}

/// Runs `vcpu` to completion on a new thread, interrupting it after `timeout`.
///
/// The vCPU descriptor is consumed and dropped on the worker thread before this returns, so the
/// caller can release the VM and guest memory afterwards.
pub(crate) fn run_with_deadline(
    vcpu: VcpuFd,
    bus: &PortBusHandle,
    mmio: Option<MmioDispatch>,
    sentinel: Option<Vec<u8>>,
    timeout: Duration,
) -> RunReport {
    if timeout.is_zero() {
        return RunReport::failed(
            mmio,
            MachineError::invalid(Phase::Run, "deadline must be positive"),
        );
    }
    let context = match RunContext::acquire() {
        Ok(context) => context,
        Err(error) => return RunReport::failed(mmio, error),
    };
    match VcpuRun::start(
        &context,
        vcpu,
        bus,
        mmio,
        sentinel,
        &Arc::new(ExitLedger::new()),
    ) {
        Ok(run) => run.wait(timeout),
        Err(report) => report,
    }
}

/// One vCPU thread that has entered `KVM_RUN` with its interrupt mask installed.
pub(crate) struct VcpuRun {
    worker: JoinHandle<()>,
    receiver: Receiver<WorkerEvent>,
    signal: libc::c_int,
    pause: Arc<AtomicBool>,
}

impl VcpuRun {
    /// Starts one worker thread and waits until it has entered its run mask.
    ///
    /// # Errors
    ///
    /// Returns a report carrying the bus and dispatcher when the thread could not start; a
    /// worker that neither reports readiness nor finishes within the startup grace aborts
    /// the process because its run mask state is unknown.
    pub(crate) fn start(
        context: &RunContext,
        vcpu: VcpuFd,
        bus: &PortBusHandle,
        mmio: Option<MmioDispatch>,
        sentinel: Option<Vec<u8>>,
        ledger: &Arc<ExitLedger>,
    ) -> Result<Self, RunReport> {
        let signal = context.signal;
        let (sender, receiver) = mpsc::sync_channel(2);
        let mmio = mmio.map(Box::new);
        let pause = Arc::new(AtomicBool::new(false));
        let worker_pause = Arc::clone(&pause);
        let worker_ledger = Arc::clone(ledger);
        let worker_bus = bus.clone();
        let worker = match thread::Builder::new()
            .name("soma-kvm-vcpu".to_owned())
            .spawn(move || {
                worker_main(
                    vcpu,
                    &worker_bus,
                    mmio,
                    sentinel.as_deref(),
                    &Control {
                        signal,
                        pause: &worker_pause,
                        sender: &sender,
                        ledger: &worker_ledger,
                    },
                );
            }) {
            Ok(worker) => worker,
            Err(error) => {
                return Err(RunReport {
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
            }),
            Ok(WorkerEvent::Finished(mmio, result, vcpu)) => {
                Err(finish(worker, mmio, result, vcpu))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(join_then(worker, RunReport::lost(Phase::Run)))
            }
            // The worker's KVM_RUN mask may not be installed, so neither a kick nor a return is safe.
            Err(RecvTimeoutError::Timeout) => std::process::abort(),
        }
    }

    /// Waits for this vCPU to stop, kicking it out of `KVM_RUN` after `timeout`.
    pub(crate) fn wait(self, timeout: Duration) -> RunReport {
        let Self {
            worker,
            receiver,
            signal,
            pause: _pause,
        } = self;
        match receiver.recv_timeout(timeout) {
            Ok(WorkerEvent::Finished(mmio, result, vcpu)) => finish(worker, mmio, result, vcpu),
            Ok(WorkerEvent::Ready) => std::process::abort(),
            Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Run)),
            Err(RecvTimeoutError::Timeout) => cancel(worker, &receiver, signal),
        }
    }

    /// Kicks this vCPU out of `KVM_RUN` at a safe point and reclaims its descriptor.
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
        } = self;
        pause.store(true, Ordering::Release);
        if let Err(error) = kick::kick(&worker, signal) {
            return join_then(
                worker,
                RunReport {
                    mmio: None,
                    result: Err(error),
                    vcpu: None,
                },
            );
        }
        match receiver.recv_timeout(grace) {
            Ok(WorkerEvent::Finished(mmio, result, vcpu)) => finish(worker, mmio, result, vcpu),
            Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Join)),
            Ok(WorkerEvent::Ready) | Err(RecvTimeoutError::Timeout) => std::process::abort(),
        }
    }
}
