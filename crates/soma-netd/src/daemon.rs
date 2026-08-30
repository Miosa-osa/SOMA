//! The minimal broker daemon: prepared bundles served over one `SOCK_SEQPACKET` socket.
//!
//! One request frame yields one reply frame; a successful claim additionally sends the TAP
//! descriptor with its typed transfer header on the same connection, and returns the
//! assignment's single-use activation challenge.
//! Activation requires the receipt the repaired guest session minted from that challenge; a
//! rejected, replayed, or failed activation releases the assignment instead of retrying.
//! The skeleton is single-threaded and does not yet authenticate the peer; the library is
//! the deliverable and the daemon is the smallest honest composition of it.

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
    ffi::CString,
    fs::{self, File},
    io::Read,
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
};

use crate::{
    Assigned, Broker, BundleId, CleanupGeneration, Error, MAX_FRAME, Reply, Request, Step,
    SterileBundle, TransferHeader, activate, bundle::AssignFailure, error_code, reconcile, release,
    release_record, release_sterile, send_tap,
};

struct State {
    broker: Broker,
    pool: VecDeque<SterileBundle>,
    assigned: BTreeMap<(BundleId, CleanupGeneration), Assigned>,
}

/// Prepares `prepared` bundles and serves requests until the listener fails.
///
/// # Errors
///
/// Returns the first preparation or listener failure.
pub fn serve(broker: Broker, socket: &Path, prepared: usize) -> Result<(), Error> {
    let mut state = State {
        broker,
        pool: VecDeque::with_capacity(prepared),
        assigned: BTreeMap::new(),
    };
    for _ in 0..prepared {
        let bundle = state.broker.prepare(fresh_id()?)?;
        state.pool.push_back(bundle);
    }
    let listener = listen(socket)?;
    loop {
        // SAFETY: `accept4` only reads the listener descriptor; null address arguments are
        // permitted.
        let raw = unsafe {
            libc::accept4(
                listener.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(Error::kernel(Step::Socket));
        }
        // SAFETY: `raw` is a freshly accepted descriptor owned by nothing else.
        let connection = unsafe { OwnedFd::from_raw_fd(raw) };
        serve_connection(&mut state, &connection);
    }
}

fn serve_connection(state: &mut State, connection: &OwnedFd) {
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
            Ok(request) => handle(state, request, connection),
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

fn handle(state: &mut State, request: Request, connection: &OwnedFd) -> Reply {
    match request {
        Request::Claim {
            instance,
            operation,
            vsock_cid,
            intent,
        } => {
            let bundle = match state.pool.pop_front() {
                Some(bundle) => bundle,
                None => match fresh_id().and_then(|id| state.broker.prepare(id)) {
                    Ok(bundle) => bundle,
                    Err(error) => return Reply::Failed(error_code(&error)),
                },
            };
            match state
                .broker
                .assign(bundle, instance, operation, &intent, vsock_cid)
            {
                Ok(assigned) => {
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
                    if let Err(error) =
                        send_tap(connection.as_fd(), &header, assigned.bundle().tap().as_fd())
                    {
                        let _ = release(&state.broker, assigned);
                        return Reply::Failed(error_code(&error));
                    }
                    state.assigned.insert(key, assigned);
                    Reply::Claimed {
                        bundle: key.0,
                        generation: key.1,
                        launch,
                        activation,
                    }
                }
                Err(AssignFailure { bundle, error }) => {
                    let _ = release_sterile(&state.broker, *bundle, Vec::new());
                    Reply::Failed(error_code(&error))
                }
            }
        }
        Request::Activate {
            bundle,
            generation,
            receipt,
        } => {
            let Some(mut assigned) = state.assigned.remove(&(bundle, generation)) else {
                return Reply::Failed(error_code(&Error::NotAssigned));
            };
            match activate(&mut assigned, &receipt) {
                Ok(_) => {
                    state.assigned.insert((bundle, generation), assigned);
                    Reply::Activated
                }
                Err(error) => {
                    let _ = release(&state.broker, assigned);
                    Reply::Failed(error_code(&error))
                }
            }
        }
        Request::Release { bundle, generation } => {
            let result = match state.assigned.remove(&(bundle, generation)) {
                Some(assigned) => release(&state.broker, assigned),
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

fn listen(path: &Path) -> Result<OwnedFd, Error> {
    let _ = fs::remove_file(path);
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::InvalidState("socket path"))?;
    if c_path.as_bytes().len() >= 108 {
        return Err(Error::InvalidState("socket path length"));
    }
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before ownership
    // is taken.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(Error::kernel(Step::Socket));
    }
    // SAFETY: `raw` is a freshly created descriptor owned by nothing else.
    let listener = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: `sockaddr_un` is a plain C aggregate for which all-zero bytes are valid.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(c_path.as_bytes()) {
        *slot = *byte as libc::c_char;
    }
    // SAFETY: `address` is fully initialised and its exact size is passed.
    let bound = unsafe {
        libc::bind(
            listener.as_raw_fd(),
            (&raw const address).cast(),
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if bound != 0 {
        return Err(Error::kernel(Step::Bind));
    }
    // SAFETY: `listen` only reads the descriptor and backlog.
    if unsafe { libc::listen(listener.as_raw_fd(), 16) } != 0 {
        return Err(Error::kernel(Step::Bind));
    }
    Ok(listener)
}
