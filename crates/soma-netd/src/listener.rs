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
    ffi::{CString, OsStr},
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{ControlAuthority, Error, PeerIdentity, Step};

mod ownership;

use ownership::{
    DIRECTORY_MODE, Node, SOCKET_MODE, facts, facts_of, open_directory, own_descriptor, require,
    require_facts,
};

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

/// Returns the user identity this broker runs as, which must own the socket and directory.
#[must_use]
pub fn broker_owner() -> u32 {
    // SAFETY: `getuid` reads this process's identity and has no preconditions.
    unsafe { libc::getuid() }
}

/// Creates the socket directory and proves it before any privileged change touches it.
///
/// The directory is opened without following a final symlink and judged through that
/// descriptor, so a planted symlink is refused rather than chowned. Ownership and mode are set
/// only on a directory this call just created; a directory that already existed is accepted
/// exactly as it is or refused, never taken over.
fn prepare_directory(directory: &Path, authority: &ControlAuthority) -> Result<(), Error> {
    let path = c_path(directory.as_os_str())?;
    // SAFETY: the path is a valid NUL-terminated string.
    let created = unsafe { libc::mkdir(path.as_ptr(), DIRECTORY_MODE) } == 0;
    if !created && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        return Err(Error::kernel(Step::Bind));
    }
    let node = open_directory(&path)?;
    if created {
        if facts_of(node.as_fd())?.1 != broker_owner() {
            return Err(Error::Unauthorized("socket directory owner"));
        }
        own_descriptor(node.as_fd(), authority, DIRECTORY_MODE)?;
    }
    require_facts(Node::Directory, facts_of(node.as_fd())?, authority)
}

fn own_node(path: &CString, authority: &ControlAuthority, mode: u32) -> Result<(), Error> {
    // SAFETY: the path is a valid NUL-terminated string.
    if unsafe { libc::chown(path.as_ptr(), authority.owner(), authority.group()) } != 0 {
        return Err(Error::kernel(Step::Bind));
    }
    // SAFETY: the path is a valid NUL-terminated string.
    if unsafe { libc::chmod(path.as_ptr(), mode) } != 0 {
        return Err(Error::kernel(Step::Bind));
    }
    Ok(())
}

fn clear_stale(path: &CString, authority: &ControlAuthority) -> Result<(), Error> {
    let Some((mode, uid, _)) = facts(path)? else {
        return Ok(());
    };
    if mode & libc::S_IFMT != libc::S_IFSOCK || uid != authority.owner() {
        return Err(Error::Unauthorized("stale socket path"));
    }
    // SAFETY: the path is a valid NUL-terminated string.
    if unsafe { libc::unlink(path.as_ptr()) } != 0 {
        return Err(Error::kernel(Step::Unlink));
    }
    Ok(())
}

fn socket() -> Result<OwnedFd, Error> {
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before ownership
    // is taken.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(Error::kernel(Step::Socket));
    }
    // SAFETY: `raw` is a freshly created descriptor owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn bind_socket(listener: &OwnedFd, path: &CString) -> Result<(), Error> {
    // SAFETY: `sockaddr_un` is a plain C aggregate for which all-zero bytes are valid.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(path.as_bytes()) {
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
    if bound == 0 {
        Ok(())
    } else {
        Err(Error::kernel(Step::Bind))
    }
}

/// Gives one accepted connection its receive deadline, so a silent peer disconnects itself.
fn bound_receive(connection: &OwnedFd, idle: Duration) -> Result<(), Error> {
    let timeout = libc::timeval {
        tv_sec: idle.as_secs() as libc::time_t,
        tv_usec: libc::suseconds_t::from(idle.subsec_micros()),
    };
    // SAFETY: the descriptor is open and the option buffer matches the passed length.
    let set = unsafe {
        libc::setsockopt(
            connection.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&raw const timeout).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if set == 0 {
        Ok(())
    } else {
        Err(Error::kernel(Step::Socket))
    }
}

fn peer_identity(connection: &OwnedFd) -> Result<PeerIdentity, Error> {
    // SAFETY: `ucred` is a plain C aggregate for which all-zero bytes are valid.
    let mut credential: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the descriptor is open and the output buffer matches the passed length.
    let read = unsafe {
        libc::getsockopt(
            connection.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credential).cast(),
            &raw mut length,
        )
    };
    if read != 0 || length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(Error::Unauthorized("peer credential"));
    }
    Ok(PeerIdentity::new(
        credential.uid,
        credential.gid,
        credential.pid,
    ))
}

fn c_path(path: &OsStr) -> Result<CString, Error> {
    let encoded = CString::new(path.as_bytes()).map_err(|_| Error::InvalidState("socket path"))?;
    if encoded.as_bytes().len() >= 108 {
        return Err(Error::InvalidState("socket path length"));
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests;
