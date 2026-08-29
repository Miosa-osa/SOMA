//! The shared table of simulated worker processes.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, atomic::AtomicU64},
};

use crate::{TransferStep, WorkerId};

/// One simulated process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Process {
    /// The worker the process was built for.
    pub worker: WorkerId,
    /// Whether it is alive.
    pub alive: bool,
    /// Whether it started its Instance.
    pub started: bool,
    /// Steps it acknowledged, in order.
    pub received: Vec<TransferStep>,
    /// Descriptors it received.
    pub descriptors: usize,
}

/// The shared table of simulated processes; survives a simulated allocator restart.
#[derive(Debug, Default)]
pub struct ProcessTable {
    processes: Mutex<BTreeMap<u64, Process>>,
    pub(super) next: AtomicU64,
}

impl ProcessTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            processes: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1000),
        })
    }

    /// Returns one process.
    #[must_use]
    pub fn process(&self, pid: u64) -> Option<Process> {
        self.lock().get(&pid).cloned()
    }

    /// Counts live processes.
    #[must_use]
    pub fn alive(&self) -> usize {
        self.lock().values().filter(|p| p.alive).count()
    }

    /// Counts every process ever built.
    #[must_use]
    pub fn total(&self) -> usize {
        self.lock().len()
    }

    /// Kills every process, as parent death would.
    pub fn kill_all(&self) {
        for process in self.lock().values_mut() {
            process.alive = false;
        }
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, Process>> {
        self.processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
