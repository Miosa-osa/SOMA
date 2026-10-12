//! The minimal broker daemon: prepared bundles served over one `SOCK_SEQPACKET` socket.
//!
//! One request frame yields one reply frame; a successful claim additionally sends the TAP
//! descriptor with its typed transfer header on the same connection, and returns the
//! assignment's single-use activation challenge.
//! Every lifecycle mutation commits before its reply is delivered, and delivery is complete or
//! terminal: [`delivery`] states the exact recovery each undelivered reply leaves, and
//! [`claim`] makes a claim idempotent under its Instance and Launch operation so an uncertain
//! delivery is replayed rather than turned into a second lease.
//! Activation requires the receipt this assignment's challenge authenticates; the receipt that
//! succeeded is retained, so a peer that lost its `Activated` reply replays the same request
//! and is answered from that record instead of having its running Machine torn down.
//! A receipt that fails to authenticate still releases the assignment.
//! An operation identity stays reserved until its release actually completes, so a teardown
//! that left kernel objects behind can never be answered with a second lease.
//! The claiming peer identity is also recorded durably, so releasing a bundle that only the
//! ledger still names - every live Machine after a broker restart - is refused to any peer but
//! the one that claimed it.
//!
//! Every connection is authenticated by [`crate::ControlListener`] before a byte is read, every
//! request needs the [`crate::Capability`] its operation requires, and an assignment can only be
//! activated or released by the exact peer identity that claimed it, so the transferred
//! descriptor and every later reply stay bound to one authenticated owner.
//! Every admitted connection is served on its own thread and carries the listener's receive
//! deadline, so a peer that connects and stays silent disconnects itself instead of holding a
//! thread. Lifecycle requests run concurrently: the shared maps are locked only to look an
//! assignment up or to move it in and out, and every kernel operation runs outside them.
//! Reconciliation takes the exclusive side of the lifecycle gate, so it never observes a claim
//! half-built.
//! A pool of prepared bundles is kept topped up by background workers, so a claim takes a
//! bundle that is already built instead of building one on the request path.
//! The library is the deliverable and the daemon is the smallest honest composition of it.

#![allow(unsafe_code)]
// Socket ABI values are fixed-width by definition; the casts below convert `libc` constants
// and structure sizes whose ranges are bounded by the kernel structures they describe.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment
)]

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    os::fd::{AsFd, AsRawFd, OwnedFd},
    path::Path,
    sync::{
        Condvar, Mutex, MutexGuard, RwLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

mod claim;
mod connect;
mod delivery;

use claim::{claim, fresh_id};
use connect::connect;
use delivery::deliver;

use crate::{
    Accepted, Assigned, Broker, BundleId, Capability, CleanupGeneration, ControlAuthority,
    ControlListener, Error, InstanceId, MAX_FRAME, OperationId, PeerIdentity, Reply, Request,
    SterileBundle, activate, error_code, reconcile, release, release_record,
};

/// Connections served at once; one more is closed so a flood cannot exhaust threads.
const MAX_CONNECTIONS: usize = 1024;
/// Workers that build bundles in parallel; the kernel commit each one waits on is a grace
/// period, so overlapping them is what makes the refill rate keep up with a burst of claims.
const PREPARE_WORKERS: usize = 4;
/// How long the request path must have been idle before background work starts.
const QUIET_MILLIS: u64 = 150;
/// Teardowns allowed to run at once.
const MAX_TEARDOWNS: usize = 8;
/// The longest a release waits for the request path to go quiet before it runs anyway.
const RELEASE_PATIENCE: Duration = Duration::from_millis(500);
/// Pool depth under which a worker builds even while the request path is busy, because a
/// claim that finds the pool empty builds a bundle inline and that is slower than contention.
const LOW_WATER: usize = 8;
/// How long a worker backs off after a failed preparation before it tries again.
const PREPARE_BACKOFF: Duration = Duration::from_secs(1);

/// One assignment and the authenticated peer identity that claimed it.
struct Owned {
    peer: u32,
    assigned: Assigned,
}

/// The bundle slot one assignment occupies.
type Slot = (BundleId, CleanupGeneration);

/// The assignments this broker holds.
#[derive(Default)]
struct Maps {
    assigned: BTreeMap<Slot, Owned>,
    /// The slot each Instance and Launch operation already holds, so a replayed claim finds
    /// its own assignment instead of taking a second one.
    operations: BTreeMap<(InstanceId, OperationId), Slot>,
    /// Claims being built, so two concurrent copies of one claim cannot take two bundles.
    pending: BTreeSet<(InstanceId, OperationId)>,
}

struct State {
    broker: Broker,
    maps: Mutex<Maps>,
    /// Wakes a claim waiting for another copy of itself to be owned or abandoned.
    settled: Condvar,
    pool: Mutex<VecDeque<SterileBundle>>,
    /// Wakes the workers when the pool drops below its target.
    refill: Condvar,
    /// Bundles workers are building right now, counted toward the target.
    building: AtomicUsize,
    target: usize,
    /// Held shared by every lifecycle request and exclusively by reconciliation.
    gate: RwLock<()>,
    /// What the request path is doing, so background work can stay out of its way.
    traffic: Traffic,
    /// Bounds the teardowns running at once.
    teardowns: Permits,
}

/// Claims and activations in flight, and when the last one finished.
///
/// Building and tearing down bundles contend with a claim or an activation for the kernel's
/// network configuration lock, and an activation that waits behind a hundred background builds
/// is a sandbox that is slow to become reachable. Background work therefore waits for the
/// request path to go quiet.
struct Traffic {
    inflight: AtomicUsize,
    last_end_millis: AtomicU64,
    epoch: Instant,
}

impl Traffic {
    fn new() -> Self {
        Self {
            inflight: AtomicUsize::new(0),
            last_end_millis: AtomicU64::new(0),
            epoch: Instant::now(),
        }
    }

    fn now_millis(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Marks one request-path operation in flight until the returned guard drops.
    fn enter(&self) -> InFlight<'_> {
        self.inflight.fetch_add(1, Ordering::AcqRel);
        InFlight(self)
    }

    fn quiet(&self) -> bool {
        self.inflight.load(Ordering::Acquire) == 0
            && self
                .now_millis()
                .saturating_sub(self.last_end_millis.load(Ordering::Acquire))
                >= QUIET_MILLIS
    }

    /// Waits until the request path has been idle for the quiet interval, or `patience` ends.
    fn wait_quiet(&self, patience: Duration) {
        let deadline = Instant::now() + patience;
        while !self.quiet() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// A counting limit on how many teardowns run at once.
///
/// Each teardown spawns privileged tools that wait on kernel locks, and a hundred spawned at
/// once do not finish sooner than eight at a time; they miss the tool deadline instead, which
/// turns a slow release into an incomplete one.
struct Permits {
    free: Mutex<usize>,
    freed: Condvar,
}

impl Permits {
    const fn new(count: usize) -> Self {
        Self {
            free: Mutex::new(count),
            freed: Condvar::new(),
        }
    }

    fn acquire(&self) -> Permit<'_> {
        let mut free = self
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *free == 0 {
            free = self
                .freed
                .wait(free)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *free -= 1;
        Permit(self)
    }
}

struct Permit<'a>(&'a Permits);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self
            .0
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        self.0.freed.notify_one();
    }
}

struct InFlight<'a>(&'a Traffic);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0
            .last_end_millis
            .store(self.0.now_millis(), Ordering::Release);
        self.0.inflight.fetch_sub(1, Ordering::AcqRel);
    }
}

impl State {
    fn maps(&self) -> MutexGuard<'_, Maps> {
        self.maps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn pool(&self) -> MutexGuard<'_, VecDeque<SterileBundle>> {
        self.pool
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Takes one prepared bundle and tells the workers the pool shrank.
    fn take_bundle(&self) -> Option<SterileBundle> {
        let bundle = self.pool().pop_front();
        self.refill.notify_all();
        bundle
    }

    /// Takes ownership of one assignment under both of its identities.
    fn own(&self, owned: Owned) {
        let record = owned.assigned.record();
        let slot = (record.bundle, record.generation);
        let operation = (record.instance, record.operation);
        let mut maps = self.maps();
        maps.pending.remove(&operation);
        maps.operations.insert(operation, slot);
        maps.assigned.insert(slot, owned);
        drop(maps);
        self.settled.notify_all();
    }

    /// Gives up a claim that never became an assignment.
    fn abandon(&self, operation: (InstanceId, OperationId)) {
        self.maps().pending.remove(&operation);
        self.settled.notify_all();
    }

    /// Takes the assignment out of the live map, keeping its operation identity reserved.
    ///
    /// The identity stays reserved until the kernel objects are actually gone, so a replayed
    /// claim can never be answered with a second lease while the first one still exists.
    fn disown(&self, slot: Slot) -> Option<Owned> {
        self.maps().assigned.remove(&slot)
    }

    /// Frees the operation identity of an assignment whose release completed.
    fn forget(&self, operation: (InstanceId, OperationId)) {
        self.maps().operations.remove(&operation);
    }

    /// Makes the lease index and zone of a verified release available to the next bundle.
    ///
    /// Only the first completed release of an assignment of this generation may recycle: a
    /// replayed release finds the index possibly held by a newer bundle.
    fn recycle(&self, record: &crate::AssignmentRecord) {
        if record.generation == self.broker.generation() {
            self.broker.recycle(record.lease_index, record.zone);
        }
    }
}

/// The Instance and Launch operation one assignment holds.
fn operation_of(assigned: &Assigned) -> (InstanceId, OperationId) {
    let record = assigned.record();
    (record.instance, record.operation)
}

/// Keeps the pool at its target by building bundles outside every lock.
fn prepare_worker(state: &State) {
    loop {
        {
            let mut pool = state.pool();
            loop {
                let held = pool.len() + state.building.load(Ordering::Acquire);
                if held < state.target {
                    state.building.fetch_add(1, Ordering::AcqRel);
                    break;
                }
                pool = state
                    .refill
                    .wait(pool)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
        if state.pool().len() >= LOW_WATER.min(state.target) {
            state.traffic.wait_quiet(Duration::from_secs(5));
        }
        let built = fresh_id().and_then(|id| state.broker.prepare(id));
        state.building.fetch_sub(1, Ordering::AcqRel);
        match built {
            Ok(bundle) => {
                state.pool().push_back(bundle);
                state.refill.notify_all();
            }
            // A full lease pool or a failing kernel call is retried rather than spun on.
            Err(_) => std::thread::sleep(PREPARE_BACKOFF),
        }
    }
}

/// Prepares `prepared` bundles and serves requests until the listener fails.
///
/// `prepared` is also the pool target the background workers hold the pool at.
///
/// # Errors
///
/// Returns the first preparation or listener failure.
pub fn serve(
    broker: Broker,
    socket: &Path,
    prepared: usize,
    authority: ControlAuthority,
) -> Result<(), Error> {
    let state = State {
        broker,
        maps: Mutex::new(Maps::default()),
        settled: Condvar::new(),
        pool: Mutex::new(VecDeque::with_capacity(prepared)),
        refill: Condvar::new(),
        building: AtomicUsize::new(0),
        target: prepared,
        gate: RwLock::new(()),
        traffic: Traffic::new(),
        teardowns: Permits::new(MAX_TEARDOWNS),
    };
    // The pool a previous process prepared has no ledger owner; remove it before building a new one.
    // A failure here must not stop egress brokering, so it is reported and the daemon starts anyway.
    match crate::reconcile::reap_unassigned(&state.broker) {
        Ok(report) => eprintln!(
            "soma-netd: start reaped {} unassigned bundle(s); {} in use, {} failed",
            report.removed.len(),
            report.in_use.len(),
            report.failed.len()
        ),
        Err(error) => eprintln!("soma-netd: start could not reap unassigned bundles: {error}"),
    }
    let listener = ControlListener::bind(socket, authority)?;
    let live = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..PREPARE_WORKERS {
            scope.spawn(|| prepare_worker(&state));
        }
        loop {
            match listener.accept()? {
                Accepted::Authorized(connection, peer) => {
                    if live.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                        live.fetch_sub(1, Ordering::AcqRel);
                        continue;
                    }
                    let (state, live, authority) = (&state, &live, listener.authority());
                    scope.spawn(move || {
                        serve_connection(state, authority, &connection, peer);
                        live.fetch_sub(1, Ordering::AcqRel);
                    });
                }
                Accepted::Rejected(_) => {}
            }
        }
    })
}

fn serve_connection(
    state: &State,
    authority: &ControlAuthority,
    connection: &OwnedFd,
    peer: PeerIdentity,
) {
    let mut frame = [0_u8; MAX_FRAME + 1];
    loop {
        // SAFETY: `frame` is a valid writable buffer of exactly the passed length.
        let received = unsafe {
            libc::recv(
                connection.as_raw_fd(),
                frame.as_mut_ptr().cast(),
                frame.len(),
                0,
            )
        };
        if received <= 0 {
            return;
        }
        let reply = match Request::decode(&frame[..received as usize]) {
            Ok(request) if authority.permits(&peer, Capability::required_for(&request)) => {
                handle(state, request, connection, peer)
            }
            Ok(_) => Reply::Failed(error_code(&Error::Unauthorized("peer capability"))),
            Err(error) => Reply::Failed(error_code(&error)),
        };
        if deliver(connection.as_fd(), &reply.encode()).is_err() {
            return;
        }
    }
}

fn handle(state: &State, request: Request, connection: &OwnedFd, peer: PeerIdentity) -> Reply {
    match request {
        Request::Reconcile => {
            let _exclusive = state
                .gate
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match reconcile(&state.broker) {
                Ok(report) => {
                    let (consistent, drifted, orphaned) = report.counts();
                    Reply::Reconciled {
                        consistent,
                        drifted,
                        orphaned,
                        unowned: report.unowned(),
                    }
                }
                Err(error) => Reply::Failed(error_code(&error)),
            }
        }
        lifecycle => {
            let _shared = state
                .gate
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            handle_lifecycle(state, lifecycle, connection, peer)
        }
    }
}

fn handle_lifecycle(
    state: &State,
    request: Request,
    connection: &OwnedFd,
    peer: PeerIdentity,
) -> Reply {
    match request {
        Request::Claim {
            instance,
            operation,
            vsock_cid,
            intent,
        } => {
            let _busy = state.traffic.enter();
            claim(
                state,
                connection,
                peer,
                (instance, operation, vsock_cid, &intent),
            )
        }
        Request::Activate {
            bundle,
            generation,
            receipt,
        } => {
            let _busy = state.traffic.enter();
            let Some(mut owned) = state.disown((bundle, generation)) else {
                return Reply::Failed(error_code(&Error::NotAssigned));
            };
            if owned.peer != peer.uid() {
                state.own(owned);
                return Reply::Failed(error_code(&Error::Unauthorized("assignment owner")));
            }
            if owned.assigned.activated_by(&receipt) {
                state.own(owned);
                return Reply::Activated;
            }
            match activate(&mut owned.assigned, &receipt) {
                Ok(_) => {
                    state.own(owned);
                    Reply::Activated
                }
                Err(error) => {
                    let operation = operation_of(&owned.assigned);
                    let record = owned.assigned.record().clone();
                    if release(&state.broker, owned.assigned).complete {
                        state.forget(operation);
                        state.recycle(&record);
                    }
                    Reply::Failed(error_code(&error))
                }
            }
        }
        Request::Release { bundle, generation } => {
            release_request(state, peer, (bundle, generation))
        }
        Request::Connect {
            instance,
            bundle,
            generation,
            port,
        } => connect(
            state,
            connection,
            peer,
            (instance, bundle, generation, port),
        ),
        Request::Reconcile => Reply::Failed(error_code(&Error::InvalidState("reconcile"))),
    }
}

fn release_request(state: &State, peer: PeerIdentity, slot: Slot) -> Reply {
    let (bundle, generation) = slot;
    state.traffic.wait_quiet(RELEASE_PATIENCE);
    let _turn = state.teardowns.acquire();
    let result = match state.disown((bundle, generation)) {
        Some(owned) if owned.peer != peer.uid() => {
            state.own(owned);
            Err(Error::Unauthorized("assignment owner"))
        }
        Some(owned) => {
            let operation = operation_of(&owned.assigned);
            let record = owned.assigned.record().clone();
            let evidence = release(&state.broker, owned.assigned);
            log_release(&record, &evidence);
            if evidence.complete {
                state.forget(operation);
                state.recycle(&record);
            }
            Ok(evidence)
        }
        None => match state.broker.ledger().lookup(bundle, generation) {
            Ok(entry) if entry.record.owner != peer.uid() => {
                Err(Error::Unauthorized("assignment owner"))
            }
            Ok(entry) => {
                let evidence = release_record(&state.broker, &entry.record);
                if evidence.complete {
                    state.forget((entry.record.instance, entry.record.operation));
                    if !entry.released {
                        state.recycle(&entry.record);
                    }
                }
                Ok(evidence)
            }
            Err(error) => Err(error),
        },
    };
    match result {
        Ok(evidence) => Reply::Released {
            complete: evidence.complete,
        },
        Err(error) => Reply::Failed(error_code(&error)),
    }
}

/// Says why a release was not verified complete; reconciliation owns what is left.
fn log_release(record: &crate::AssignmentRecord, evidence: &crate::ReleaseEvidence) {
    if let Some(usage) = evidence.usage {
        eprintln!(
            "soma-netd: usage bundle={} bytes_out={} bytes_in={} packets_out={} packets_in={}",
            record.bundle.short_hex(),
            usage.bytes_out,
            usage.bytes_in,
            usage.packets_out,
            usage.packets_in
        );
    }
    if !evidence.complete {
        eprintln!("soma-netd: release incomplete: {evidence:?}");
    }
}
