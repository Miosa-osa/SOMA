//! The in-process worker launcher with per-step fault injection.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use super::table::{Process, ProcessTable};
use crate::{
    ConstructFault, DestroyOutcome, Liveness, PoolKey, Removal, StartFault, StepAck, TransferFault,
    TransferFrame, TransferStep, WorkerHandle, WorkerId, WorkerIdentity, WorkerLauncher,
};

/// How a step fails.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InjectedFault {
    /// The worker rejects the frame.
    Rejected,
    /// The worker never acknowledges.
    Timeout,
    /// The worker acknowledges the wrong step.
    PartialAck,
    /// The channel closes.
    Closed,
    /// The worker stalls for the duration before acknowledging.
    Stall(Duration),
    /// The allocator process dies mid-transfer: the step panics, so no teardown runs and the
    /// worker, its process, and its assigned resources are left exactly as a crash leaves
    /// them. Drive it under [`std::panic::catch_unwind`].
    Abandon,
}

/// What the launcher and its handles should fail.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FaultPlan {
    /// Fail construction.
    pub construct: Option<ConstructFault>,
    /// Sleep this long inside construction.
    pub construct_delay: Duration,
    /// Fail one transfer step.
    pub transfer: Option<(TransferStep, InjectedFault)>,
    /// Fail start.
    pub start: Option<StartFault>,
}

/// The in-process launcher.
#[derive(Debug)]
pub struct InProcessLauncher {
    table: Arc<ProcessTable>,
    plan: Mutex<FaultPlan>,
    concurrent: AtomicUsize,
    peak: AtomicUsize,
    constructed: AtomicUsize,
}

impl InProcessLauncher {
    /// A launcher over `table`.
    #[must_use]
    pub fn new(table: Arc<ProcessTable>) -> Self {
        Self {
            table,
            plan: Mutex::new(FaultPlan::default()),
            concurrent: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            constructed: AtomicUsize::new(0),
        }
    }

    /// Replaces the fault plan for every later construction.
    pub fn set_plan(&self, plan: FaultPlan) {
        *self
            .plan
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = plan;
    }

    /// The most constructions that ever ran at once.
    #[must_use]
    pub fn peak_concurrency(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }

    /// Successful constructions.
    #[must_use]
    pub fn constructed(&self) -> usize {
        self.constructed.load(Ordering::Acquire)
    }

    /// The process table.
    #[must_use]
    pub const fn table(&self) -> &Arc<ProcessTable> {
        &self.table
    }
}

impl WorkerLauncher for InProcessLauncher {
    type Handle = InProcessHandle;

    fn construct(
        &self,
        _key: &PoolKey,
        worker: WorkerId,
        _budget: Duration,
    ) -> Result<Self::Handle, ConstructFault> {
        let plan = *self
            .plan
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let running = self.concurrent.fetch_add(1, Ordering::AcqRel) + 1;
        self.peak.fetch_max(running, Ordering::AcqRel);
        if !plan.construct_delay.is_zero() {
            thread::sleep(plan.construct_delay);
        }
        self.concurrent.fetch_sub(1, Ordering::AcqRel);
        if let Some(fault) = plan.construct {
            return Err(fault);
        }
        let pid = self.table.next.fetch_add(1, Ordering::AcqRel);
        let mut token = [0; 16];
        token[..8].copy_from_slice(&pid.to_be_bytes());
        token[8..].copy_from_slice(&worker.as_bytes()[..8]);
        self.table.lock().insert(
            pid,
            Process {
                worker,
                alive: true,
                started: false,
                received: Vec::new(),
                descriptors: 0,
            },
        );
        self.constructed.fetch_add(1, Ordering::AcqRel);
        Ok(InProcessHandle {
            table: Arc::clone(&self.table),
            identity: WorkerIdentity {
                process: pid,
                token,
            },
            plan,
            expected: TransferStep::Identity,
        })
    }

    fn probe(&self, identity: WorkerIdentity) -> Liveness {
        match self.table.process(identity.process) {
            Some(process) if process.alive => Liveness::Alive,
            Some(_) => Liveness::Gone,
            None => Liveness::Unknown,
        }
    }

    fn terminate(&self, identity: WorkerIdentity) -> DestroyOutcome {
        terminate(&self.table, identity.process)
    }
}

fn terminate(table: &ProcessTable, pid: u64) -> DestroyOutcome {
    let mut processes = table.lock();
    let process = match processes.get_mut(&pid) {
        Some(process) if process.alive => {
            process.alive = false;
            Removal::Removed
        }
        Some(_) => Removal::AlreadyAbsent,
        None => Removal::Unknown,
    };
    DestroyOutcome {
        process,
        cgroup: process,
        complete: process != Removal::Unknown,
    }
}

/// The in-process handle.
#[derive(Debug)]
pub struct InProcessHandle {
    table: Arc<ProcessTable>,
    identity: WorkerIdentity,
    plan: FaultPlan,
    expected: TransferStep,
}

impl WorkerHandle for InProcessHandle {
    fn identity(&self) -> WorkerIdentity {
        self.identity
    }

    fn deliver(&mut self, frame: TransferFrame) -> Result<StepAck, TransferFault> {
        let step = frame.step();
        if !self
            .table
            .process(self.identity.process)
            .is_some_and(|p| p.alive)
        {
            return Err(TransferFault::Closed);
        }
        if step != self.expected {
            return Err(TransferFault::PartialAck);
        }
        if let Some((at, fault)) = self.plan.transfer
            && at == step
        {
            match fault {
                InjectedFault::Rejected => return Err(TransferFault::Rejected),
                InjectedFault::Timeout => return Err(TransferFault::Timeout),
                InjectedFault::PartialAck => return Err(TransferFault::PartialAck),
                InjectedFault::Closed => {
                    terminate(&self.table, self.identity.process);
                    return Err(TransferFault::Closed);
                }
                InjectedFault::Stall(duration) => thread::sleep(duration),
                InjectedFault::Abandon => panic!("simulated allocator crash at {step:?}"),
            }
        }
        let descriptors = match frame {
            TransferFrame::Disk(_) | TransferFrame::Network(_) | TransferFrame::Control { .. } => 1,
            _ => 0,
        };
        if let Some(process) = self.table.lock().get_mut(&self.identity.process) {
            process.received.push(step);
            process.descriptors += descriptors;
        }
        self.expected = TransferStep::from_code(step.code() + 1).unwrap_or(TransferStep::Commit);
        Ok(StepAck::Accepted)
    }

    fn start(&mut self) -> Result<(), StartFault> {
        if let Some(fault) = self.plan.start {
            return Err(fault);
        }
        match self.table.lock().get_mut(&self.identity.process) {
            Some(process) if process.alive => {
                process.started = true;
                Ok(())
            }
            _ => Err(StartFault::Closed),
        }
    }

    fn destroy(self) -> DestroyOutcome {
        terminate(&self.table, self.identity.process)
    }
}
