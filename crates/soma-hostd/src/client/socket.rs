//! The client end of one Unix `SOCK_SEQPACKET` connection to the Host daemon.
//!
//! The transport is deliberately the same one the daemon accepts rather than a stream with a
//! length prefix: one datagram is one frame, so a client can never read half a reply or two
//! replies as one, and the bound on a frame is enforced by the kernel.

#![allow(unsafe_code)]
// Socket ABI values are fixed-width by definition; the casts below convert `libc` constants
// and structure sizes whose ranges are bounded by the kernel structures they describe.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use std::{
    ffi::CString,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
};

use super::ClientError;
use crate::MAX_FRAME;

/// The longest Unix socket path the kernel accepts, including its terminator.
const SUN_PATH_LIMIT: usize = 108;

/// One connected client socket.
pub(super) struct Connection {
    socket: OwnedFd,
}

impl Connection {
    /// Connects to the daemon listening on `path`.
    pub(super) fn open(path: &Path) -> Result<Self, ClientError> {
        let target =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| ClientError::SocketPath)?;
        if target.as_bytes().len() >= SUN_PATH_LIMIT {
            return Err(ClientError::SocketPath);
        }
        // SAFETY: `socket` has no memory preconditions; the descriptor is checked before
        // ownership is taken.
        let raw =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        if raw < 0 {
            return Err(ClientError::Socket(errno()));
        }
        // SAFETY: `raw` is a freshly created descriptor owned by nothing else.
        let socket = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: `sockaddr_un` is a plain C aggregate for which all-zero bytes are valid.
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (slot, byte) in address.sun_path.iter_mut().zip(target.as_bytes()) {
            *slot = *byte as libc::c_char;
        }
        // SAFETY: `address` is fully initialised and its exact size is passed.
        let connected = unsafe {
            libc::connect(
                socket.as_raw_fd(),
                (&raw const address).cast(),
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if connected != 0 {
            return Err(ClientError::Connect(errno()));
        }
        Ok(Self { socket })
    }

    /// Sends one request frame and returns the one reply frame it produced.
    pub(super) fn exchange(&self, request: &[u8]) -> Result<Vec<u8>, ClientError> {
        // SAFETY: `request` is a valid buffer for its full length.
        let sent = unsafe {
            libc::send(
                self.socket.as_raw_fd(),
                request.as_ptr().cast(),
                request.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if sent < 0 || sent as usize != request.len() {
            return Err(ClientError::Send(errno()));
        }
        // The buffer is one byte longer than a legal frame so an oversized reply is truncated
        // by the kernel into a length no decoder accepts, rather than read as a valid frame.
        let mut frame = [0_u8; MAX_FRAME + 1];
        // SAFETY: `frame` is a valid writable buffer of exactly the passed length.
        let received = unsafe {
            libc::recv(
                self.socket.as_raw_fd(),
                frame.as_mut_ptr().cast(),
                frame.len(),
                0,
            )
        };
        // A zero length datagram is how a closed connection reads, and it carries no errno.
        if received <= 0 {
            return Err(ClientError::Receive(if received == 0 {
                0
            } else {
                errno()
            }));
        }
        Ok(frame[..received as usize].to_vec())
    }
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}
