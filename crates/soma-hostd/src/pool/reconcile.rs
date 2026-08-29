//! Restart reconciliation: every nonterminal ledger entry without a live slot is suspect,
//! verified against the launcher and the resource brokers, and then terminated, released,
//! or retained before the pool may replenish.

use std::fmt;

use crate::{
    LedgerError, Liveness, Phase, Pool, Record, RecordKind, ResourceBroker, ResourceLiveness,
    Running, Slot, Worker, WorkerId, WorkerLauncher,
    pool::{Owned, OwnedWorker, release::absent},
};

/// What reconciliation decided for one suspect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReconcileDisposition {
    /// The process was alive or unknown and was terminated; resources were released.
    Terminated = 1,
    /// The process was gone; resources were released.
    Released = 2,
    /// A running Instance was alive and is retained under this pool's ownership.
    Retained = 3,
}

impl ReconcileDisposition {
    /// Decodes one disposition.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Terminated),
            2 => Some(Self::Released),
            3 => Some(Self::Retained),
            _ => None,
        }
    }
}

/// One reconciled suspect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconcileFinding {
    /// The worker.
    pub worker: WorkerId,
    /// The phase the ledger held.
    pub phase: Phase,
    /// What the launcher found, when an identity was recorded.
    pub liveness: Option<Liveness>,
    /// What the brokers found.
    pub resources: ResourceLiveness,
    /// The decision.
    pub disposition: ReconcileDisposition,
    /// Whether teardown and release both reported completion.
    pub complete: bool,
    /// Whether the committed capacity of a retained Instance was reserved again.
    pub capacity_restored: bool,
}

/// The complete reconciliation pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconcileReport {
    /// Entries marked suspect.
    pub suspects: usize,
    /// One finding per suspect.
    pub findings: Vec<ReconcileFinding>,
}

impl ReconcileReport {
    /// Counts terminated, released, and retained findings.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize) {
        let count = |disposition| {
            self.findings
                .iter()
                .filter(|finding| finding.disposition == disposition)
                .count()
        };
        (
            count(ReconcileDisposition::Terminated),
            count(ReconcileDisposition::Released),
            count(ReconcileDisposition::Retained),
        )
    }
}

impl fmt::Display for ReconcileReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (terminated, released, retained) = self.counts();
        write!(
            formatter,
            "{} suspects: {terminated} terminated, {released} released, {retained} retained",
            self.suspects
        )
    }
}

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// Reconciles every nonterminal ledger entry that this process does not hold a slot for.
    ///
    /// Passes are serialized, so two callers can never adopt the same running Instance twice
    /// and leave an unowned slot counting against the pool maximum forever.
    ///
    /// # Errors
    ///
    /// Returns a ledger failure; the pool stays unreconciled and will not replenish.
    pub fn reconcile(&self) -> Result<ReconcileReport, LedgerError> {
        let gate = self
            .reconcile_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entries = self.ledger().entries()?;
        let mut report = ReconcileReport::default();
        for entry in entries.values() {
            if !entry.phase.is_nonterminal() || self.find_slot(entry.worker).is_some() {
                continue;
            }
            report.suspects += 1;
            let key = self.digest();
            self.record(
                &Record::new(
                    RecordKind::Suspect,
                    entry.worker,
                    entry.lease_generation,
                    key,
                )
                .detail(entry.phase.code()),
            )?;
            let liveness = entry
                .identity
                .map(|identity| self.launcher().probe(identity));
            let resources = self.broker().verify(&entry.resources);
            let retain = entry.phase == Phase::Running && liveness == Some(Liveness::Alive);
            let mut capacity_restored = false;
            let (disposition, complete) = if retain {
                let (fitted, reserved) = self.retain(entry);
                capacity_restored = reserved;
                (ReconcileDisposition::Retained, fitted)
            } else {
                let destroyed = match (entry.identity, liveness) {
                    (Some(identity), Some(Liveness::Alive | Liveness::Unknown)) => {
                        Some(self.launcher().terminate(identity))
                    }
                    _ => None,
                };
                let released = self.broker().release(&entry.resources);
                let disposition = if destroyed.is_some() {
                    ReconcileDisposition::Terminated
                } else {
                    ReconcileDisposition::Released
                };
                let complete =
                    destroyed.is_none_or(|outcome| outcome.complete) && released.complete;
                (disposition, complete)
            };
            self.record(
                &Record::new(
                    RecordKind::Reconciled,
                    entry.worker,
                    entry.lease_generation,
                    key,
                )
                .detail(disposition as u8)
                .identity(entry.identity.unwrap_or(absent()))
                .resources(entry.resources),
            )?;
            report.findings.push(ReconcileFinding {
                worker: entry.worker,
                phase: entry.phase,
                liveness,
                resources,
                disposition,
                complete,
                capacity_restored,
            });
        }
        self.reconciled
            .store(true, std::sync::atomic::Ordering::Release);
        drop(gate);
        Ok(report)
    }

    /// Adopts a running Instance without a handle.
    ///
    /// Returns whether it fit the table and whether its committed capacity was reserved
    /// again, so a restart rebuilds the host usage of every Instance it keeps.
    fn retain(&self, entry: &crate::WorkerLedgerEntry) -> (bool, bool) {
        let slot = Slot::restore(
            entry.worker,
            entry.key,
            Phase::Running,
            entry.lease_generation,
        );
        let Ok(slot) = self.add_slot(slot) else {
            return (false, false);
        };
        let Some(worker) = Worker::<Running>::attach(slot) else {
            return (false, false);
        };
        let (Some(instance), Some(operation), Some(identity)) =
            (entry.instance, entry.operation, entry.identity)
        else {
            return (false, false);
        };
        let mut reservation = self.reserve_capacity().ok();
        self.capacity_launched(&mut reservation);
        let reserved = reservation.is_some();
        self.owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                entry.worker,
                Owned {
                    worker: OwnedWorker::Running(worker),
                    handle: None,
                    starting: false,
                    identity,
                    refs: entry.resources,
                    instance,
                    operation,
                    reservation,
                },
            );
        (true, reserved)
    }
}
