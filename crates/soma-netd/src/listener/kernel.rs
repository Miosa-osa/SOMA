//! Every single kernel call the control listener makes: creating and owning the socket
//! directory, clearing a stale path, creating and binding the socket, and reading one accepted
//! connection's receive deadline and peer credential.
//!
//! `ControlListener` owns the order these happen in, and the order is what fails closed. This
//! file owns what each call does and which failure it reports, so the two can be read and
//! changed apart. Judging a node once it has been read is a third thing again, in `ownership`.

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
    path::Path,
    time::Duration,
};

use super::ownership::{
    DIRECTORY_MODE, Node, facts, facts_of, open_directory, own_descriptor, require_facts,
};
use crate::{ControlAuthority, Error, PeerIdentity, Step};

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
pub(super) fn prepare_directory(
    directory: &Path,
    authority: &ControlAuthority,
) -> Result<(), Error> {
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

pub(super) fn own_node(
    path: &CString,
    authority: &ControlAuthority,
    mode: u32,
) -> Result<(), Error> {
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

pub(super) fn clear_stale(path: &CString, authority: &ControlAuthority) -> Result<(), Error> {
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

pub(super) fn socket() -> Result<OwnedFd, Error> {
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before ownership
    // is taken.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(Error::kernel(Step::Socket));
    }
    // SAFETY: `raw` is a freshly created descriptor owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

pub(super) fn bind_socket(listener: &OwnedFd, path: &CString) -> Result<(), Error> {
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
pub(super) fn bound_receive(connection: &OwnedFd, idle: Duration) -> Result<(), Error> {
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

pub(super) fn peer_identity(connection: &OwnedFd) -> Result<PeerIdentity, Error> {
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

pub(super) fn c_path(path: &OsStr) -> Result<CString, Error> {
    let encoded = CString::new(path.as_bytes()).map_err(|_| Error::InvalidState("socket path"))?;
    if encoded.as_bytes().len() >= 108 {
        return Err(Error::InvalidState("socket path length"));
    }
    Ok(encoded)
}
