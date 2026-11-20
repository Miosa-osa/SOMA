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
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

mod claim;
mod connect;
mod delivery;
mod egress;
mod instance_lock;
mod pacing;
mod teardown;

use claim::{claim, fresh_id};
use connect::connect;
use delivery::deliver;
use instance_lock::InstanceLock;
use pacing::{Permits, Traffic};
use teardown::release_request;

use crate::{
    Accepted, Assigned, Broker, BundleId, Capability, CleanupGeneration, ControlAuthority,
    ControlListener, Error, InstanceId, MAX_FRAME, OperationId, PeerIdentity, Reply, Request, Step,
    SterileBundle, activate, error_code, release,
};

/// Connections served at once; one more is closed so a flood cannot exhaust threads.
const MAX_CONNECTIONS: usize = 1024;
/// Workers that build bundles in parallel; the kernel commit each one waits on is a grace
/// period, so overlapping them is what makes the refill rate keep up with a burst of claims.
const PREPARE_WORKERS: usize = 4;
/// Teardowns allowed to run at once.
const MAX_TEARDOWNS: usize = 8;
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
    // One broker per state root. A second process would see the first one's live pool as unowned
    // and reap it, and would unlink the first one's control socket.
    let _only_broker = InstanceLock::acquire(state_root(&state.broker)?)?;
    // The pool a previous process prepared has no ledger owner. What to remove is decided now,
    // before this process builds any bundle, so removal can run beside the listener without being
    // able to name one of its own. A failure to decide must not stop egress brokering, so it is
    // reported and the daemon starts anyway.
    let plan = match crate::reconcile::reap_plan(&state.broker) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("soma-netd: start could not plan a reap of unassigned bundles: {error}");
            crate::reconcile::ReapPlan::default()
        }
    };
    // A window that was open when the previous broker ended has no timer and no record now, and the
    // kernel kept its ruleset. Every bundle claimed with no egress is put back to that base before
    // the first request is served, so a restart can only close, never leave open.
    match crate::recover::restore_base_rulesets(&state.broker) {
        Ok(report) => eprintln!(
            "soma-netd: start restored {} denied bundle(s) to their base; forwarding switched off for [{}]; could not restore [{}]",
            report.restored.len(),
            report.forwarding_off.join(","),
            report.failed.join(",")
        ),
        Err(error) => eprintln!(
            "soma-netd: WARN: start could not read the ledger to restore denied bundles: {error}"
        ),
    }
    let listener = ControlListener::bind(socket, authority)?;
    let live = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let report = plan.execute(&state.broker, crate::reconcile::MAX_REAP_PER_START);
            eprintln!(
                "soma-netd: start reaped {} unassigned bundle(s); in use [{}]; failed [{}]; deferred {}",
                report.removed.len(),
                report.in_use.join(","),
                report.failed.join(","),
                report.deferred.len()
            );
            if !report.foreign.is_empty() {
                eprintln!(
                    "soma-netd: WARN: kernel objects with the broker's naming and no pin of ours were left alone (another broker, or a bundle whose pin is gone): [{}]",
                    report.foreign.join(",")
                );
            }
            if !report.failed.is_empty() {
                eprintln!(
                    "soma-netd: WARN: could not fully remove [{}]; they keep their pin and are retried at the next start",
                    report.failed.join(",")
                );
            }
        });
        for _ in 0..PREPARE_WORKERS {
            scope.spawn(|| prepare_worker(&state));
        }
        if let Some(config) = crate::snapshot::Config::from_env() {
            let broker = &state.broker;
            scope.spawn(move || crate::snapshot::run(broker, &config));
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
            let window_open = |bundle: crate::BundleId, generation: crate::CleanupGeneration| {
                state
                    .maps()
                    .assigned
                    .get(&(bundle, generation))
                    .is_some_and(|owned| owned.assigned.current.egress().forwards())
            };
            match crate::reconcile::reconcile_with(&state.broker, &window_open) {
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
        Request::SetEgress {
            instance,
            bundle,
            generation,
            egress,
            dns,
        } => egress::set_egress(state, peer, (instance, bundle, generation, egress, dns)),
        Request::Reconcile => Reply::Failed(error_code(&Error::InvalidState("reconcile"))),
    }
}

/// The directory that holds this broker's ledger and namespace pins.
fn state_root(broker: &Broker) -> Result<&std::path::Path, Error> {
    broker.namespace_dir().parent().ok_or(Error::Kernel {
        step: Step::LedgerOpen,
        errno: libc::EINVAL,
    })
}
