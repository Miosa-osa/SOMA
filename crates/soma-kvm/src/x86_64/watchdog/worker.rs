//! The vCPU worker thread body and the join, cancel, and reporting helpers around it.
//!
//! Splitting these out of the watchdog keeps the deadline policy and the thread mechanics in
//! separate files; the ownership rule is unchanged, so a vCPU descriptor is released here
//! unless a pause hands it back for state capture.

use std::{
    sync::{
        Arc,
        atomic::AtomicBool,
        mpsc::{Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use kvm_ioctls::VcpuFd;

use super::{CANCELLATION_GRACE, RunReport, WorkerEvent};
use crate::x86_64::{
    error::{MachineError, MachineErrorKind, Phase},
    exits::ExitLedger,
    kick::{self, RunMaskGuard},
    mmio::MmioDispatch,
    ports::PortBus,
    run::{self, GuestExit},
};

const JOIN_POLL: Duration = Duration::from_millis(1);

/// The channel, signal, pause flag, and exit ledger the watchdog shares with its worker.
pub(super) struct Control<'a> {
    pub(super) signal: libc::c_int,
    pub(super) pause: &'a AtomicBool,
    pub(super) sender: &'a SyncSender<WorkerEvent>,
    pub(super) ledger: &'a Arc<ExitLedger>,
}

pub(super) fn worker_main(
    mut vcpu: VcpuFd,
    mut bus: Box<PortBus>,
    mut mmio: Option<Box<MmioDispatch>>,
    sentinel: Option<&[u8]>,
    control: &Control<'_>,
) {
    let Control {
        signal,
        pause,
        sender,
        ledger,
    } = *control;
    let result = match RunMaskGuard::install(&vcpu, signal) {
        Ok(mask) => {
            let result = if sender.send(WorkerEvent::Ready).is_ok() {
                run::run(
                    &mut vcpu,
                    &mut bus,
                    mmio.as_deref_mut(),
                    sentinel,
                    pause,
                    ledger,
                )
            } else {
                Err(MachineError::new(Phase::Run, MachineErrorKind::WorkerLost))
            };
            drop(mask);
            result
        }
        Err(error) => Err(error),
    };
    // A paused vCPU is handed back so its state can be read; every other outcome releases the
    // descriptor here, before the caller may release the VM and guest memory.
    let carried = if result == Ok(GuestExit::Paused) {
        Some(vcpu)
    } else {
        drop(vcpu);
        None
    };
    if let Some(dispatch) = mmio.as_deref() {
        dispatch.finish();
    }
    let _ignored = sender.send(WorkerEvent::Finished(bus, mmio, result, carried));
}

pub(super) fn cancel(
    worker: JoinHandle<()>,
    receiver: &Receiver<WorkerEvent>,
    signal: libc::c_int,
) -> RunReport {
    let kick_error = kick::kick(&worker, signal).err();
    match receiver.recv_timeout(CANCELLATION_GRACE) {
        Ok(WorkerEvent::Finished(bus, mmio, result, vcpu)) => {
            let result = match (kick_error, result) {
                (Some(error), _) => Err(error),
                (None, Ok(exit)) => Ok(exit),
                (None, Err(_)) => Err(MachineError::new(Phase::Run, MachineErrorKind::Timeout)),
            };
            finish(worker, bus, mmio, result, vcpu)
        }
        // A kicked worker that neither finishes nor disconnects may still own a live vCPU.
        Ok(WorkerEvent::Ready) | Err(RecvTimeoutError::Timeout) => std::process::abort(),
        Err(RecvTimeoutError::Disconnected) => join_then(worker, RunReport::lost(Phase::Join)),
    }
}

pub(super) fn finish(
    worker: JoinHandle<()>,
    bus: Box<PortBus>,
    mmio: Option<Box<MmioDispatch>>,
    result: Result<GuestExit, MachineError>,
    vcpu: Option<VcpuFd>,
) -> RunReport {
    join_then(
        worker,
        RunReport {
            bus: Some(bus),
            mmio,
            result,
            vcpu,
        },
    )
}

/// Joins the worker within the grace period; a join failure replaces the pending result.
pub(super) fn join_then(worker: JoinHandle<()>, report: RunReport) -> RunReport {
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
            vcpu: report.vcpu,
        };
    }
    report
}
