//! One bounded pool of sterile workers and resource bundles for exactly one [`PoolKey`].
//!
//! The struct holds the slot table, the prepared payloads of sterile workers, the owned
//! handles of assigned workers, the idempotent claim registry, the durable ledger, and the
//! replenishment threads; each submodule adds one policy or mechanism to it.

pub mod backpressure;
pub mod capacity;
pub mod claim;
mod inspect;
pub mod key;
pub mod launcher;
pub mod ledger;
pub mod reconcile;
pub mod release;
pub mod replenish;
pub mod resources;
pub mod state;
pub mod transfer;

use std::{
    collections::BTreeMap,
    fmt,
    path::Path,
    sync::{
        Arc, Condvar, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
};

use sha2::{Digest, Sha256};

use capacity::PoolAdmission;
use claim::Registry;
use ledger::now_nanos;

use crate::{
    Assigned, InstanceId, LeaseGeneration, Ledger, LedgerError, Limits, LimitsError, OperationId,
    OverloadGate, Overloaded, Phase, PoolKey, PoolKeyDigest, Record, Reservation, ResourceBroker,
    ResourceRefs, Running, Slot, Worker, WorkerHandle, WorkerId, WorkerIdentity, WorkerLauncher,
};

/// Why a pool could not open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoolError {
    /// The limits are invalid.
    Limits(LimitsError),
    /// The ledger is unusable.
    Ledger(LedgerError),
}

impl fmt::Display for PoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limits(error) => write!(formatter, "invalid limits: {error}"),
            Self::Ledger(error) => write!(formatter, "ledger: {error}"),
        }
    }
}

impl std::error::Error for PoolError {}

/// The prepared payloads of every sterile worker.
type PreparedTable<L, R> =
    BTreeMap<WorkerId, Prepared<<L as WorkerLauncher>::Handle, <R as ResourceBroker>::Sterile>>;

/// The payload a sterile worker carries until it is claimed.
pub(crate) struct Prepared<H, S> {
    pub(crate) handle: H,
    pub(crate) sterile: S,
    pub(crate) refs: ResourceRefs,
    pub(crate) identity: WorkerIdentity,
}

/// The typestate of an owned worker.
pub(crate) enum OwnedWorker {
    Assigned(Worker<Assigned>),
    Running(Worker<Running>),
}

impl OwnedWorker {
    /// The shared slot both typestates act on.
    pub(crate) const fn slot(&self) -> &Arc<Slot> {
        match self {
            Self::Assigned(worker) => worker.slot(),
            Self::Running(worker) => worker.slot(),
        }
    }
}

/// One worker after transfer; `handle` is absent for a worker retained across a restart and
/// while [`Pool::start`] holds it outside the pool-wide lock.
pub(crate) struct Owned<H> {
    pub(crate) worker: OwnedWorker,
    pub(crate) handle: Option<H>,
    /// Whether a start is in flight; the pool still owns the worker, so a release is refused
    /// by phase rather than answered as unknown.
    pub(crate) starting: bool,
    pub(crate) identity: WorkerIdentity,
    pub(crate) refs: ResourceRefs,
    pub(crate) instance: InstanceId,
    pub(crate) operation: OperationId,
    pub(crate) reservation: Option<Reservation>,
}

/// What `inspect` reports about one worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerView {
    /// The worker.
    pub worker: WorkerId,
    /// The phase.
    pub phase: Phase,
    /// The lease generation.
    pub lease_generation: LeaseGeneration,
    /// The claiming operation, once claimed.
    pub operation: Option<OperationId>,
    /// The Instance, once assigned.
    pub instance: Option<InstanceId>,
}

/// One bounded pool.
pub struct Pool<L: WorkerLauncher, R: ResourceBroker> {
    key: PoolKey,
    digest: PoolKeyDigest,
    limits: Limits,
    launcher: L,
    broker: R,
    capacity: PoolAdmission,
    ledger: Ledger,
    slots: RwLock<Vec<Arc<Slot>>>,
    pub(crate) prepared: Mutex<PreparedTable<L, R>>,
    pub(crate) owned: Mutex<BTreeMap<WorkerId, Owned<L::Handle>>>,
    pub(crate) registry: Mutex<Registry>,
    pub(crate) registry_changed: Condvar,
    pub(crate) in_flight: AtomicUsize,
    pub(crate) replenish_gate: Mutex<()>,
    pub(crate) threads: Mutex<Vec<JoinHandle<()>>>,
    pub(crate) reconciled: AtomicBool,
    counter: AtomicU64,
}

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// Opens a pool over `ledger_root`; a ledger with nonterminal entries must be reconciled
    /// before the pool replenishes, and the idempotency registry is seeded from the claims
    /// the ledger already holds so a replay survives a restart.
    ///
    /// # Errors
    ///
    /// Returns invalid limits or an unusable ledger.
    pub fn open(
        key: PoolKey,
        limits: Limits,
        launcher: L,
        broker: R,
        capacity: PoolAdmission,
        ledger_root: &Path,
    ) -> Result<Self, PoolError> {
        let limits = limits.validate().map_err(PoolError::Limits)?;
        let ledger = Ledger::open(ledger_root).map_err(PoolError::Ledger)?;
        let suspects = ledger
            .entries()
            .map_err(PoolError::Ledger)?
            .values()
            .any(|entry| entry.phase.is_nonterminal());
        let mut registry = Registry::default();
        registry.seed(
            &ledger.claims().map_err(PoolError::Ledger)?,
            limits.binding_limit,
        );
        Ok(Self {
            digest: key.digest(),
            key,
            limits,
            launcher,
            broker,
            capacity,
            ledger,
            slots: RwLock::new(Vec::new()),
            prepared: Mutex::new(BTreeMap::new()),
            owned: Mutex::new(BTreeMap::new()),
            registry: Mutex::new(registry),
            registry_changed: Condvar::new(),
            in_flight: AtomicUsize::new(0),
            replenish_gate: Mutex::new(()),
            threads: Mutex::new(Vec::new()),
            reconciled: AtomicBool::new(!suspects),
            counter: AtomicU64::new(0),
        })
    }

    /// Returns the key.
    #[must_use]
    pub const fn key(&self) -> &PoolKey {
        &self.key
    }

    /// Returns the key digest.
    #[must_use]
    pub const fn digest(&self) -> PoolKeyDigest {
        self.digest
    }

    /// Returns the limits.
    #[must_use]
    pub const fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Returns the ledger.
    #[must_use]
    pub const fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Returns the launcher.
    #[must_use]
    pub const fn launcher(&self) -> &L {
        &self.launcher
    }

    /// Returns the broker.
    #[must_use]
    pub const fn broker(&self) -> &R {
        &self.broker
    }

    /// Returns whether the ledger still holds unreconciled suspects.
    #[must_use]
    pub fn needs_reconcile(&self) -> bool {
        !self.reconciled.load(Ordering::Acquire)
    }

    /// Adds a slot in the given phase, compacting dead slots and enforcing `max`.
    pub(crate) fn add_slot(&self, slot: Arc<Slot>) -> Result<Arc<Slot>, Overloaded> {
        let mut slots = self
            .slots
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots.retain(|slot| slot.observe().phase != Phase::Dead);
        if slots.len() >= self.limits.max {
            return Err(Overloaded {
                gate: OverloadGate::PoolMaximum,
                current: slots.len(),
                limit: self.limits.max,
            });
        }
        slots.push(Arc::clone(&slot));
        Ok(slot)
    }

    pub(crate) fn record(&self, record: &Record) -> Result<u64, LedgerError> {
        self.ledger.append(record)
    }

    /// Derives a fresh worker identity from the pool, process, clock, and a counter.
    pub(crate) fn fresh_worker_id(&self) -> WorkerId {
        loop {
            let counter = self.counter.fetch_add(1, Ordering::Relaxed);
            let mut hasher = Sha256::new();
            hasher.update(b"SOMAWORKER");
            hasher.update(self.digest.as_bytes());
            hasher.update(std::process::id().to_be_bytes());
            hasher.update(now_nanos().to_be_bytes());
            hasher.update(counter.to_be_bytes());
            let digest: [u8; 32] = hasher.finalize().into();
            let mut bytes = [0; 16];
            bytes.copy_from_slice(&digest[..16]);
            if let Ok(id) = WorkerId::new(bytes) {
                return id;
            }
        }
    }

    pub(crate) fn destroy_handle(handle: Option<L::Handle>) -> Option<crate::DestroyOutcome> {
        handle.map(WorkerHandle::destroy)
    }
}
