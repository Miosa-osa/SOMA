//! Runs each vCPU of one machine on its own dedicated OS thread under a hard deadline.
//!
//! If a guest neither stops nor faults before the deadline, the watchdog kicks that vCPU thread
//! out of `KVM_RUN`. If a thread still cannot be joined within a bounded grace period the
//! process aborts, because releasing guest memory under a live vCPU is never acceptable.
//!
//! The interrupt handler is process-wide and shared by every active machine. [`RunContext`]
//! holds one reference while a machine is running or resident. [`VcpuRun`] is one worker,
//! and a machine with several vCPUs holds one per processor.

mod vcpu;
mod worker;
pub(crate) use vcpu::VcpuRun;

use std::{
    sync::{
        Arc, Mutex, PoisonError,
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
static PROCESS_HANDLER: Mutex<Option<ProcessHandler>> = Mutex::new(None);

struct ProcessHandler {
    _handler: HandlerGuard,
    signal: libc::c_int,
    owners: usize,
    retain_until_exit: bool,
}

enum WorkerEvent {
    Ready,
    Finished(
        Option<Box<MmioDispatch>>,
        Result<GuestExit, MachineError>,
        Option<VcpuFd>,
    ),
}

/// One machine's ownership of the shared process-wide interrupt handler.
pub(crate) struct RunContext {
    signal: libc::c_int,
}

impl RunContext {
    /// Acquires the same handler for all running or resident machines.
    pub(crate) fn acquire() -> Result<Self, MachineError> {
        let mut active = PROCESS_HANDLER
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(handler) = active.as_mut() {
            handler.owners += 1;
            return Ok(Self {
                signal: handler.signal,
            });
        }
        let signal = kick::signal_number()?;
        *active = Some(ProcessHandler {
            _handler: HandlerGuard::install(signal)?,
            signal,
            owners: 1,
            retain_until_exit: false,
        });
        Ok(Self { signal })
    }
}

impl Drop for RunContext {
    fn drop(&mut self) {
        let mut active = PROCESS_HANDLER
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let handler = active.as_mut().expect("a live context owns the handler");
        handler.owners -= 1;
        if handler.owners == 0 && !handler.retain_until_exit {
            // Restore the previous signal action before another caller may acquire it.
            *active = None;
        }
    }
}

/// Keeps the installed watchdog handler for the lifetime of a dedicated VMM process.
pub(super) fn retain_process_handler() {
    let mut active = PROCESS_HANDLER
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(handler) = active.as_mut() {
        handler.retain_until_exit = true;
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

impl From<MachineError> for RunReport {
    fn from(error: MachineError) -> Self {
        Self::failed(None, error)
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
