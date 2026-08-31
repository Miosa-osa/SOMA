//! The unprivileged client end of the broker's control socket.
//!
//! The broker holds `CAP_NET_ADMIN`; a launcher does not, and must never need it. So a launcher
//! speaks this bounded protocol instead of opening a [`crate::Broker`] in process: one request
//! frame, one reply frame, and for an accepted claim one extra packet carrying the TAP
//! descriptor with `SCM_RIGHTS` and its typed header.
//!
//! One connection belongs to one Instance for its whole life, because the broker binds an
//! assignment to the authenticated peer that claimed it and refuses activation and release to
//! anyone else.

#![allow(unsafe_code)]
// Socket ABI values are fixed-width by definition; these casts convert `libc` constants and the
// return of `recvmsg`, whose range is bounded by the buffer passed to it.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment
)]

use std::{
    mem,
    os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd},
    path::Path,
    ptr,
};

use crate::{MAX_FRAME, MAX_HEADER, Reply, Request, TransferHeader};

/// Why one broker exchange did not produce an answer the caller can use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientError {
    /// No broker is listening at the given path.
    Unreachable,
    /// The exchange was truncated, malformed, or self-contradictory.
    Protocol,
    /// The broker answered with a typed failure code from [`crate::error_code`].
    Refused(u16),
}

/// One authenticated connection to the broker.
#[derive(Debug)]
pub struct BrokerClient {
    socket: OwnedFd,
}

impl BrokerClient {
    /// Connects to the broker socket at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Unreachable`] when no broker is listening there, which is the only
    /// condition a caller may read as "this host has no network broker".
    pub fn connect(path: &Path) -> Result<Self, ClientError> {
        let address = sockaddr(path).ok_or(ClientError::Unreachable)?;
        // SAFETY: `socket` has no memory preconditions and the result is checked before it is
        // owned.
        let raw =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        if raw < 0 {
            return Err(ClientError::Unreachable);
        }
        // SAFETY: `raw` is a freshly created descriptor owned by nothing else.
        let socket = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: `address` is a live, fully initialised `sockaddr_un` of the passed length.
        let connected = unsafe {
            libc::connect(
                socket.as_raw_fd(),
                ptr::from_ref(&address).cast(),
                mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if connected != 0 {
            return Err(ClientError::Unreachable);
        }
        Ok(Self { socket })
    }

    /// Sends one request and returns its reply, with any descriptor the broker transferred.
    ///
    /// A transferred TAP arrives non-blocking: a VMM device thread reads it inline, so a
    /// blocking descriptor would stall every other device behind one absent frame, and leaving
    /// that to each caller would make the stall a matter of who remembered.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Protocol`] for a truncated or self-contradictory exchange and
    /// [`ClientError::Refused`] carrying the broker's own failure code.
    pub fn call(&self, request: &Request) -> Result<(Reply, Option<OwnedFd>), ClientError> {
        let frame = request.encode();
        // SAFETY: `frame` is a valid buffer for its full length; `MSG_NOSIGNAL` keeps a closed
        // broker from raising `SIGPIPE` in the caller's process.
        let sent = unsafe {
            libc::send(
                self.socket.as_raw_fd(),
                frame.as_ptr().cast(),
                frame.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if usize::try_from(sent) != Ok(frame.len()) {
            return Err(ClientError::Protocol);
        }
        // A claim the broker accepted arrives as the descriptor packet first and the reply
        // second; every other outcome, a refused claim included, is the reply alone. So the
        // first packet is classified by whether it carried a descriptor rather than by what was
        // asked for, and a refusal is never mistaken for a transfer.
        let (payload, descriptor) = self.receive()?;
        let (payload, descriptor) = match descriptor {
            Some(descriptor) => {
                let header = TransferHeader::decode(&payload).map_err(|_| ClientError::Protocol)?;
                let (reply, stray) = self.receive()?;
                if stray.is_some() {
                    return Err(ClientError::Protocol);
                }
                names_the_same_assignment(&reply, &header)?;
                nonblocking(&descriptor)?;
                (reply, Some(descriptor))
            }
            None => (payload, None),
        };
        match Reply::decode(&payload).map_err(|_| ClientError::Protocol)? {
            Reply::Failed(code) => Err(ClientError::Refused(code)),
            reply => Ok((reply, descriptor)),
        }
    }

    /// Receives one packet, and the single descriptor it may carry.
    fn receive(&self) -> Result<(Vec<u8>, Option<OwnedFd>), ClientError> {
        let mut payload = [0_u8; MAX_FRAME + MAX_HEADER + 1];
        let mut control = [0_u8; 128];
        let mut iov = libc::iovec {
            iov_base: payload.as_mut_ptr().cast(),
            iov_len: payload.len(),
        };
        // SAFETY: `msghdr` is a plain C aggregate for which all-zero bytes are valid.
        let mut message: libc::msghdr = unsafe { mem::zeroed() };
        message.msg_iov = &raw mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = control.len();
        // SAFETY: every pointer inside `message` references live locals sized as declared.
        let received = unsafe {
            libc::recvmsg(
                self.socket.as_raw_fd(),
                &raw mut message,
                libc::MSG_CMSG_CLOEXEC,
            )
        };
        if received <= 0 {
            return Err(ClientError::Protocol);
        }
        let descriptors = collect(&message);
        // A control message the kernel had to truncate may have dropped a descriptor this
        // process would then never close, so the packet is refused rather than trusted.
        if message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0 || descriptors.len() > 1 {
            return Err(ClientError::Protocol);
        }
        Ok((
            payload[..received as usize].to_vec(),
            descriptors.into_iter().next(),
        ))
    }
}

/// Requires the claim reply to name the exact assignment the descriptor packet named.
fn names_the_same_assignment(reply: &[u8], header: &TransferHeader) -> Result<(), ClientError> {
    match Reply::decode(reply).map_err(|_| ClientError::Protocol)? {
        Reply::Claimed {
            bundle, generation, ..
        } if bundle == header.bundle && generation == header.generation => Ok(()),
        Reply::Failed(code) => Err(ClientError::Refused(code)),
        // A descriptor that arrived under one assignment and a reply describing another cannot
        // both be true, and taking either would attach a frame path to the wrong lease.
        _ => Err(ClientError::Protocol),
    }
}

/// Collects every descriptor the packet carried, so none is left open on a rejected packet.
fn collect(message: &libc::msghdr) -> Vec<OwnedFd> {
    let mut owned = Vec::new();
    // SAFETY: `CMSG_FIRSTHDR` and `CMSG_NXTHDR` walk only inside the control buffer bounded by
    // `msg_controllen`, and each descriptor read stays within the header's declared length.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(message);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(cmsg);
                let bytes = (*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                for index in 0..bytes / mem::size_of::<libc::c_int>() {
                    let raw = ptr::read_unaligned(data.cast::<libc::c_int>().add(index));
                    owned.push(OwnedFd::from_raw_fd(raw));
                }
            }
            cmsg = libc::CMSG_NXTHDR(message, cmsg);
        }
    }
    owned
}

/// Puts the transferred descriptor in non-blocking mode.
fn nonblocking(descriptor: &OwnedFd) -> Result<(), ClientError> {
    let fd = descriptor.as_raw_fd();
    // SAFETY: `fcntl` with these two commands takes no pointer argument.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: as above; `flags` is the value the kernel just reported for this descriptor.
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(ClientError::Protocol);
    }
    Ok(())
}

/// Encodes one filesystem socket address, refusing a path the address cannot hold.
fn sockaddr(path: &Path) -> Option<libc::sockaddr_un> {
    use std::os::unix::ffi::OsStrExt as _;
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: `sockaddr_un` is a plain C aggregate for which all-zero bytes are valid.
    let mut address: libc::sockaddr_un = unsafe { mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    // One byte is reserved for the terminator the kernel expects on a filesystem path.
    if bytes.is_empty() || bytes.len() >= address.sun_path.len() {
        return None;
    }
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    Some(address)
}

#[cfg(test)]
#[path = "client/tests.rs"]
mod tests;
