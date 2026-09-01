//! The owned control socket: an explicitly owned directory, a verified socket node, and one
//! kernel-derived identity for every connection.
//!
//! The directory is opened without following a final symlink and judged through that
//! descriptor before any privileged change reaches it, so a planted symlink is refused rather
//! than chowned, and a directory that already existed is accepted as it is or refused rather
//! than taken over.
//! The socket is created, given its exact owner, group, and mode, and then verified by reading
//! it back.
//! Both are verified again before every accept, so ownership drift or a permission change made
//! after startup fails closed instead of widening reach.
//! A stale path is removed only when it is a socket already owned by this broker; anything else
//! is a refusal rather than an unlink.
//! Every accepted connection carries the peer credential the kernel stamped, and a peer the
//! authority does not admit is closed before any frame is read.
//! Every accepted connection also carries [`IDLE_TIMEOUT`] as its receive deadline, so an
//! admitted peer that connects and then stays silent disconnects itself instead of wedging the
//! single-threaded broker for every other peer.

#![allow(unsafe_code)]
// Socket and file ABI values are fixed-width by definition; the casts below convert `libc`
// constants and structure sizes whose ranges are bounded by the structures they describe.
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

use std::{
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{ControlAuthority, Error, PeerIdentity, Step};

pub use kernel::broker_owner;

// Every single kernel call this listener makes lives beside it. `ControlListener` owns the order
// they must happen in, which is the part that fails closed; that file owns what each one does
// and which failure it reports.
use kernel::{
    bind_socket, bound_receive, c_path, clear_stale, own_node, peer_identity, prepare_directory,
    socket,
};

mod kernel;
mod ownership;

use ownership::{Node, SOCKET_MODE, require};

const BACKLOG: i32 = 16;

/// How long one accepted connection may stay silent before the broker closes it.
///
/// The broker is single-threaded and serves one connection at a time, so an admitted peer that
/// connects and never sends would otherwise deny every lifecycle and reconcile request to every
/// other admitted peer. A peer whose connection is closed this way loses nothing: it reconnects
/// and replays the same Instance and Launch operation.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// The outcome of one accept.
#[derive(Debug)]
pub enum Accepted {
    /// An admitted peer and the kernel-derived identity every later reply is bound to.
    Authorized(OwnedFd, PeerIdentity),
    /// A peer the authority does not admit; its connection was closed before any decode.
    Rejected(PeerIdentity),
}

/// One bound, owned, verified control socket.
#[derive(Debug)]
pub struct ControlListener {
    listener: OwnedFd,
    path: PathBuf,
    authority: ControlAuthority,
}

impl ControlListener {
    /// Creates the owned directory, binds the socket, sets and verifies both, then listens.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unauthorized`] when a directory, stale path, owner, group, or mode does
    /// not match the authority, or the first kernel failure.
    pub fn bind(path: &Path, authority: ControlAuthority) -> Result<Self, Error> {
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or(Error::InvalidState("socket directory"))?;
        prepare_directory(directory, &authority)?;
        let target = c_path(path.as_os_str())?;
        clear_stale(&target, &authority)?;
        let listener = socket()?;
        bind_socket(&listener, &target)?;
        own_node(&target, &authority, SOCKET_MODE)?;
        require(Node::Socket, &target, &authority)?;
        // SAFETY: `listen` only reads the descriptor and backlog.
        if unsafe { libc::listen(listener.as_raw_fd(), BACKLOG) } != 0 {
            return Err(Error::kernel(Step::Bind));
        }
        Ok(Self {
            listener,
            path: path.to_path_buf(),
            authority,
        })
    }

    /// Returns the enforced authority.
    #[must_use]
    pub const fn authority(&self) -> &ControlAuthority {
        &self.authority
    }

    /// Reverifies ownership and mode, then accepts and authenticates one connection.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unauthorized`] on ownership or permission drift, or the first kernel
    /// failure.
    pub fn accept(&self) -> Result<Accepted, Error> {
        let directory = self
            .path
            .parent()
            .ok_or(Error::InvalidState("socket directory"))?;
        require(
            Node::Directory,
            &c_path(directory.as_os_str())?,
            &self.authority,
        )?;
        require(
            Node::Socket,
            &c_path(self.path.as_os_str())?,
            &self.authority,
        )?;
        // SAFETY: `accept4` only reads the listener descriptor; null address arguments are
        // permitted.
        let raw = unsafe {
            libc::accept4(
                self.listener.as_raw_fd(),
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
        let peer = peer_identity(&connection)?;
        bound_receive(&connection, IDLE_TIMEOUT)?;
        if self.authority.admits(&peer) {
            Ok(Accepted::Authorized(connection, peer))
        } else {
            drop(connection);
            Ok(Accepted::Rejected(peer))
        }
    }
}

#[cfg(test)]
mod tests;
