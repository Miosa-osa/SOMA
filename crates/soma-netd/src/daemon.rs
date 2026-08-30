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
//!
//! Every connection is authenticated by [`crate::ControlListener`] before a byte is read, every
//! request needs the [`crate::Capability`] its operation requires, and an assignment can only be
//! activated or released by the exact peer identity that claimed it, so the transferred
//! descriptor and every later reply stay bound to one authenticated owner.
//! The daemon is single-threaded, so every accepted connection carries the listener's receive
//! deadline: a peer that connects and stays silent disconnects itself instead of denying
//! service to every other admitted peer.
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
    collections::{BTreeMap, VecDeque},
    os::fd::{AsFd, AsRawFd, OwnedFd},
    path::Path,
};

mod claim;
mod delivery;

use claim::{claim, fresh_id};
use delivery::deliver;

use crate::{
    Accepted, Assigned, Broker, BundleId, Capability, CleanupGeneration, ControlAuthority,
    ControlListener, Error, InstanceId, MAX_FRAME, OperationId, PeerIdentity, Reply, Request,
    SterileBundle, activate, error_code, reconcile, release, release_record,
};

/// One assignment and the authenticated peer identity that claimed it.
struct Owned {
    peer: u32,
    assigned: Assigned,
}

/// The bundle slot one assignment occupies.
type Slot = (BundleId, CleanupGeneration);

struct State {
    broker: Broker,
    pool: VecDeque<SterileBundle>,
    assigned: BTreeMap<Slot, Owned>,
    /// The slot each Instance and Launch operation already holds, so a replayed claim finds
    /// its own assignment instead of taking a second one.
    operations: BTreeMap<(InstanceId, OperationId), Slot>,
}

impl State {
    /// Takes ownership of one assignment under both of its identities.
    fn own(&mut self, owned: Owned) {
        let record = owned.assigned.record();
        let slot = (record.bundle, record.generation);
        self.operations
            .insert((record.instance, record.operation), slot);
        self.assigned.insert(slot, owned);
    }

    /// Releases the daemon's hold on one slot under both of its identities.
    fn disown(&mut self, slot: Slot) -> Option<Owned> {
        let owned = self.assigned.remove(&slot)?;
        let record = owned.assigned.record();
        self.operations.remove(&(record.instance, record.operation));
        Some(owned)
    }
}

/// Prepares `prepared` bundles and serves requests until the listener fails.
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
    let mut state = State {
        broker,
        pool: VecDeque::with_capacity(prepared),
        assigned: BTreeMap::new(),
        operations: BTreeMap::new(),
    };
    for _ in 0..prepared {
        let bundle = state.broker.prepare(fresh_id()?)?;
        state.pool.push_back(bundle);
    }
    let listener = ControlListener::bind(socket, authority)?;
    loop {
        match listener.accept()? {
            Accepted::Authorized(connection, peer) => {
                serve_connection(&mut state, listener.authority(), &connection, peer);
            }
            Accepted::Rejected(_) => {}
        }
    }
}

fn serve_connection(
    state: &mut State,
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

fn handle(state: &mut State, request: Request, connection: &OwnedFd, peer: PeerIdentity) -> Reply {
    match request {
        Request::Claim {
            instance,
            operation,
            vsock_cid,
            intent,
        } => claim(
            state,
            connection,
            peer,
            (instance, operation, vsock_cid, &intent),
        ),
        Request::Activate {
            bundle,
            generation,
            receipt,
        } => {
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
                    drop(release(&state.broker, owned.assigned));
                    Reply::Failed(error_code(&error))
                }
            }
        }
        Request::Release { bundle, generation } => {
            let result = match state.disown((bundle, generation)) {
                Some(owned) if owned.peer != peer.uid() => {
                    state.own(owned);
                    Err(Error::Unauthorized("assignment owner"))
                }
                Some(owned) => Ok(release(&state.broker, owned.assigned)),
                None => match state.broker.ledger().lookup(bundle, generation) {
                    Ok(entry) => Ok(release_record(&state.broker, &entry.record)),
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
        Request::Reconcile => match reconcile(&state.broker) {
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
        },
    }
}
