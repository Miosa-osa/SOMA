//! The supervisor's end of one jailed process's control socket.
//!
//! A jailed process may not create, bind, listen on, or accept a socket: all four are in the
//! documented denial surface, and the compiled filter kills them in both phases. Its one
//! conversation therefore has to exist before it does. The launcher seals one end of a
//! connected `SOCK_SEQPACKET` pair into the manifest's control slot; this is the other end.
//!
//! `SOCK_SEQPACKET` is what makes the exchange framing-free: one packet out is one request and
//! one packet in is one reply, so neither side has a length prefix to get wrong and an
//! oversized packet is truncated by the kernel and then refused by the decoder rather than
//! being read as a shorter valid one.

#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::time::Duration;

/// Why a control exchange did not complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    /// The pair could not be created.
    Unavailable(i32),
    /// The whole packet did not leave, so the peer never saw a complete request.
    Truncated,
    /// The peer sent nothing before the deadline, or is gone.
    Silent,
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(errno) => write!(formatter, "control socket unavailable: {errno}"),
            Self::Truncated => write!(formatter, "control packet was not sent whole"),
            Self::Silent => write!(formatter, "no control packet arrived"),
        }
    }
}

impl std::error::Error for ControlError {}

/// One end of a connected control socket.
#[derive(Debug)]
pub struct ControlSocket {
    socket: OwnedFd,
}

impl ControlSocket {
    /// Creates a connected pair: the supervisor's end and the end sealed into the manifest.
    ///
    /// # Errors
    ///
    /// Returns [`ControlError::Unavailable`] with the `socketpair` errno.
    pub fn pair() -> Result<(Self, OwnedFd), ControlError> {
        let mut fds = [0 as libc::c_int; 2];
        let kind = libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC;
        // SAFETY: `fds` is valid storage for two descriptors.
        if unsafe { libc::socketpair(libc::AF_UNIX, kind, 0, fds.as_mut_ptr()) } != 0 {
            return Err(ControlError::Unavailable(errno()));
        }
        // SAFETY: both descriptors were just created and nothing else owns them.
        let (supervisor, worker) =
            unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        Ok((Self { socket: supervisor }, worker))
    }

    /// Adopts one already-connected end.
    #[must_use]
    pub const fn adopt(socket: OwnedFd) -> Self {
        Self { socket }
    }

    /// Sends one whole packet.
    ///
    /// A peer that has already gone is reported rather than signalled: `MSG_NOSIGNAL` keeps a
    /// dead worker from killing the supervisor that was talking to it.
    ///
    /// # Errors
    ///
    /// Returns [`ControlError::Truncated`] when the packet did not leave whole.
    pub fn send(&self, text: &str) -> Result<(), ControlError> {
        // SAFETY: the pointer and length describe a live string slice.
        let sent = unsafe {
            libc::send(
                self.socket.as_raw_fd(),
                text.as_ptr().cast(),
                text.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if usize::try_from(sent).is_ok_and(|count| count == text.len()) {
            Ok(())
        } else {
            Err(ControlError::Truncated)
        }
    }

    /// Receives one packet of at most `capacity` bytes, waiting at most `within`.
    ///
    /// # Errors
    ///
    /// Returns [`ControlError::Silent`] when the deadline passes first or the peer is gone.
    pub fn receive(&self, capacity: usize, within: Duration) -> Result<String, ControlError> {
        let mut poll = libc::pollfd {
            fd: self.socket.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = libc::c_int::try_from(within.as_millis()).unwrap_or(libc::c_int::MAX);
        // SAFETY: `poll` receives one valid `pollfd` and its count.
        if unsafe { libc::poll(&raw mut poll, 1, millis) } <= 0 {
            return Err(ControlError::Silent);
        }
        let mut buffer = vec![0_u8; capacity];
        // SAFETY: the pointer and length describe valid writable storage.
        let received = unsafe {
            libc::recv(
                self.socket.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                0,
            )
        };
        let count = usize::try_from(received)
            .ok()
            .filter(|count| *count > 0)
            .ok_or(ControlError::Silent)?;
        buffer.truncate(count);
        String::from_utf8(buffer).map_err(|_| ControlError::Silent)
    }
}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}
