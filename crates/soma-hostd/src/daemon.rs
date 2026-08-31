//! The minimal Host daemon: one Host Runtime served over one Unix `SOCK_SEQPACKET` socket.
//!
//! One request frame yields one reply frame.
//! The daemon holds the [`Runtime`], which is why an Instance outlives the client that
//! created it: a client owns nothing but its connection, and closing that connection ends
//! only the connection.
//! The skeleton is single-threaded and does not authenticate its peer; the library is the
//! deliverable and the daemon is the smallest honest composition of it.

#![allow(unsafe_code)]
// Socket ABI values are fixed-width by definition; the casts below convert `libc` constants
// and structure sizes whose ranges are bounded by the kernel structures they describe.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

mod dispatch;
mod lifecycle;

pub use dispatch::{handle, launch_bytes};

use std::{
    ffi::CString,
    fmt, fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
    sync::Arc,
};

use crate::{
    FailureCode, MAX_FRAME, Reply, Request, ResourceBroker, Runtime, WorkerLauncher, failure_code,
};

/// Why the daemon could not serve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaemonError {
    /// The socket path is unusable.
    SocketPath,
    /// A socket call failed with the errno.
    Socket(i32),
    /// `bind` failed with the errno.
    Bind(i32),
    /// `listen` failed with the errno.
    Listen(i32),
    /// `accept4` failed with the errno.
    Accept(i32),
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SocketPath => formatter.write_str("socket path is unusable"),
            Self::Socket(errno) => write!(formatter, "socket failed with errno {errno}"),
            Self::Bind(errno) => write!(formatter, "bind failed with errno {errno}"),
            Self::Listen(errno) => write!(formatter, "listen failed with errno {errno}"),
            Self::Accept(errno) => write!(formatter, "accept failed with errno {errno}"),
        }
    }
}

impl std::error::Error for DaemonError {}

/// Serves `runtime` on `socket` until the listener fails.
///
/// # Errors
///
/// Returns the listener failure.
pub fn serve<L: WorkerLauncher, R: ResourceBroker>(
    runtime: &Arc<Runtime<L, R>>,
    socket: &Path,
) -> Result<(), DaemonError> {
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
            return Err(DaemonError::Accept(errno()));
        }
        // SAFETY: `raw` is a freshly accepted descriptor owned by nothing else.
        let connection = unsafe { OwnedFd::from_raw_fd(raw) };
        serve_connection(runtime, &connection);
    }
}

fn serve_connection<L: WorkerLauncher, R: ResourceBroker>(
    runtime: &Arc<Runtime<L, R>>,
    connection: &OwnedFd,
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
            Ok(request) => handle(runtime, request),
            Err(_) => Reply::Failed(failure_code(FailureCode::Protocol)),
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

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn listen(path: &Path) -> Result<OwnedFd, DaemonError> {
    let _ = fs::remove_file(path);
    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| DaemonError::SocketPath)?;
    if c_path.as_bytes().len() >= 108 {
        return Err(DaemonError::SocketPath);
    }
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before ownership
    // is taken.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(DaemonError::Socket(errno()));
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
        return Err(DaemonError::Bind(errno()));
    }
    // SAFETY: `listen` only reads the descriptor and backlog.
    if unsafe { libc::listen(listener.as_raw_fd(), 16) } != 0 {
        return Err(DaemonError::Listen(errno()));
    }
    Ok(listener)
}
