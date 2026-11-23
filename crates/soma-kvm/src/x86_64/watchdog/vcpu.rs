//! One vCPU's run thread: start, resume, wait for and pause it under its deadline.

use super::{
    Arc, AtomicBool, Control, Duration, ExitLedger, JoinHandle, MachineError, MmioDispatch,
    Ordering, Phase, PortBusHandle, Receiver, RecvTimeoutError, RunContext, RunReport,
    STARTUP_GRACE, VcpuFd, WorkerEvent, cancel, finish, join_then, kick, mpsc, thread, worker_main,
};

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
        Self::spawn(
            context,
            vcpu,
            bus,
            mmio.map(Box::new),
            sentinel,
            ledger,
            true,
        )
    }

    pub(crate) fn resume(
        context: &RunContext,
        vcpu: VcpuFd,
        bus: &PortBusHandle,
        mmio: Box<MmioDispatch>,
        ledger: &Arc<ExitLedger>,
    ) -> Result<Self, RunReport> {
        Self::spawn(context, vcpu, bus, Some(mmio), None, ledger, false)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "startup and resident vCPUs use the same bounded worker"
    )]
    fn spawn(
        context: &RunContext,
        vcpu: VcpuFd,
        bus: &PortBusHandle,
        mmio: Option<Box<MmioDispatch>>,
        sentinel: Option<Vec<u8>>,
        ledger: &Arc<ExitLedger>,
        configure_kvm_mask: bool,
    ) -> Result<Self, RunReport> {
        let signal = context.signal;
        let (sender, receiver) = mpsc::sync_channel(2);
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
                        configure_kvm_mask,
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
