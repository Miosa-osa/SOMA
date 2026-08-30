//! The minimal broker daemon: prepared bundles served over one `SOCK_SEQPACKET` socket.
//!
//! One request frame yields one reply frame; a successful claim additionally sends the TAP
//! descriptor with its typed transfer header on the same connection, and returns the
//! assignment's single-use activation challenge.
//! Activation requires the receipt the repaired guest session minted from that challenge; a
//! rejected, replayed, or failed activation releases the assignment instead of retrying.
//!
//! Every connection is authenticated by [`crate::ControlListener`] before a byte is read, every
//! request needs the [`crate::Capability`] its operation requires, and an assignment can only be
//! activated or released by the exact peer identity that claimed it, so the transferred
//! descriptor and every later reply stay bound to one authenticated owner.
//! The daemon is single-threaded; the library is the deliverable and the daemon is the smallest
//! honest composition of it.

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
    fs::File,
    io::Read,
    os::fd::{AsFd, AsRawFd, OwnedFd},
    path::Path,
};

use crate::{
    Accepted, Assigned, Broker, BundleId, Capability, CleanupGeneration, ControlAuthority,
    ControlListener, Error, InstanceId, MAX_FRAME, NetworkIntent, OperationId, PeerIdentity, Reply,
    Request, Step, SterileBundle, TransferHeader, activate, bundle::AssignFailure, error_code,
    reconcile, release, release_record, release_sterile, send_tap,
};

/// One assignment and the authenticated peer identity that claimed it.
struct Owned {
    peer: u32,
    assigned: Assigned,
}

struct State {
    broker: Broker,
    pool: VecDeque<SterileBundle>,
    assigned: BTreeMap<(BundleId, CleanupGeneration), Owned>,
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
        let bytes = reply.encode();
        // SAFETY: `bytes` is a valid buffer for its full length.
        let sent = unsafe {
            libc::send(
                connection.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if sent < 0 {
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
            let Some(mut owned) = state.assigned.remove(&(bundle, generation)) else {
                return Reply::Failed(error_code(&Error::NotAssigned));
            };
            if owned.peer != peer.uid() {
                state.assigned.insert((bundle, generation), owned);
                return Reply::Failed(error_code(&Error::Unauthorized("assignment owner")));
            }
            match activate(&mut owned.assigned, &receipt) {
                Ok(_) => {
                    state.assigned.insert((bundle, generation), owned);
                    Reply::Activated
                }
                Err(error) => {
                    let _ = release(&state.broker, owned.assigned);
                    Reply::Failed(error_code(&error))
                }
            }
        }
        Request::Release { bundle, generation } => {
            let result = match state.assigned.remove(&(bundle, generation)) {
                Some(owned) if owned.peer != peer.uid() => {
                    state.assigned.insert((bundle, generation), owned);
                    Err(Error::Unauthorized("assignment owner"))
                }
                Some(owned) => release(&state.broker, owned.assigned),
                None => match state.broker.ledger().lookup(bundle, generation) {
                    Ok(entry) => release_record(&state.broker, &entry.record),
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

/// The identities one claim binds: Instance, Launch operation, vsock CID, and admitted intent.
type ClaimRequest<'a> = (InstanceId, OperationId, u32, &'a NetworkIntent);

fn claim(
    state: &mut State,
    connection: &OwnedFd,
    peer: PeerIdentity,
    request: ClaimRequest<'_>,
) -> Reply {
    let (instance, operation, vsock_cid, intent) = request;
    let bundle = match state.pool.pop_front() {
        Some(bundle) => bundle,
        None => match fresh_id().and_then(|id| state.broker.prepare(id)) {
            Ok(bundle) => bundle,
            Err(error) => return Reply::Failed(error_code(&error)),
        },
    };
    let assigned = match state
        .broker
        .assign(bundle, instance, operation, intent, vsock_cid)
    {
        Ok(assigned) => assigned,
        Err(AssignFailure { bundle, error }) => {
            let _ = release_sterile(&state.broker, *bundle, Vec::new());
            return Reply::Failed(error_code(&error));
        }
    };
    let header = TransferHeader {
        bundle: assigned.record().bundle,
        generation: assigned.record().generation,
        intent: assigned.record().intent_digest,
    };
    let launch = launch_bytes(&assigned);
    let key = (assigned.record().bundle, assigned.record().generation);
    let Some(activation) = assigned.activation_challenge().cloned() else {
        let _ = release(&state.broker, assigned);
        return Reply::Failed(error_code(&Error::InvalidState("activation")));
    };
    if let Err(error) = send_tap(connection.as_fd(), &header, assigned.bundle().tap().as_fd()) {
        let _ = release(&state.broker, assigned);
        return Reply::Failed(error_code(&error));
    }
    state.assigned.insert(
        key,
        Owned {
            peer: peer.uid(),
            assigned,
        },
    );
    Reply::Claimed {
        bundle: key.0,
        generation: key.1,
        launch,
        activation,
    }
}

fn launch_bytes(assigned: &Assigned) -> [u8; 35] {
    let launch = assigned.launch();
    let mut out = [0; 35];
    out[..4].copy_from_slice(&launch.vsock_cid().to_be_bytes());
    out[4..8].copy_from_slice(&launch.generation().to_be_bytes());
    out[8..14].copy_from_slice(&launch.mac());
    out[14..18].copy_from_slice(&launch.address());
    out[18] = launch.prefix_length();
    out[19..23].copy_from_slice(&launch.gateway());
    out[23..27].copy_from_slice(&launch.resolver());
    out[27..35].copy_from_slice(&launch.time_sample_nanos().to_be_bytes());
    out
}

fn fresh_id() -> Result<BundleId, Error> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| Error::io(Step::OpenTun, &error))?;
    BundleId::new(bytes)
}
