//! The exact ownership and permission decision for the socket directory and socket node.
//!
//! Reading a node is separated from judging it, so every drift class is decided by one pure
//! function the tests exercise directly.

#![allow(unsafe_code)]

use std::{
    ffi::CString,
    os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
};

use crate::{ControlAuthority, Error, Step};

pub(super) const DIRECTORY_MODE: u32 = 0o750;
pub(super) const SOCKET_MODE: u32 = 0o660;

#[derive(Clone, Copy)]
pub(super) enum Node {
    Directory,
    Socket,
}

impl Node {
    pub(super) const fn format(self) -> u32 {
        match self {
            Self::Directory => libc::S_IFDIR,
            Self::Socket => libc::S_IFSOCK,
        }
    }

    pub(super) const fn mode(self) -> u32 {
        match self {
            Self::Directory => DIRECTORY_MODE,
            Self::Socket => SOCKET_MODE,
        }
    }

    pub(super) const fn refusal(self, field: Field) -> Error {
        Error::Unauthorized(match (self, field) {
            (Self::Directory, Field::Format) => "socket directory type",
            (Self::Directory, Field::Owner) => "socket directory owner",
            (Self::Directory, Field::Mode) => "socket directory mode",
            (Self::Socket, Field::Format) => "socket type",
            (Self::Socket, Field::Owner) => "socket owner",
            (Self::Socket, Field::Mode) => "socket mode",
        })
    }
}

#[derive(Clone, Copy)]
pub(super) enum Field {
    Format,
    Owner,
    Mode,
}

/// The exact ownership decision, separated from the kernel call that reads the node.
pub(super) fn require_facts(
    node: Node,
    facts: (u32, u32, u32),
    authority: &ControlAuthority,
) -> Result<(), Error> {
    let (mode, uid, gid) = facts;
    if mode & libc::S_IFMT != node.format() {
        return Err(node.refusal(Field::Format));
    }
    if uid != authority.owner() || gid != authority.group() {
        return Err(node.refusal(Field::Owner));
    }
    if mode & 0o7777 != node.mode() {
        return Err(node.refusal(Field::Mode));
    }
    Ok(())
}

pub(super) fn require(
    node: Node,
    path: &CString,
    authority: &ControlAuthority,
) -> Result<(), Error> {
    let facts = facts(path)?.ok_or_else(|| node.refusal(Field::Format))?;
    require_facts(node, facts, authority)
}

/// Opens one path as a directory, refusing a final symlink instead of following it.
///
/// Every ownership decision and every ownership change is then made on this descriptor, so no
/// privileged mutation can ever reach a node the broker has not already inspected.
pub(super) fn open_directory(path: &CString) -> Result<OwnedFd, Error> {
    // SAFETY: the path is a valid NUL-terminated string; no output buffer is passed.
    let raw = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_RDONLY,
        )
    };
    if raw < 0 {
        return Err(Node::Directory.refusal(Field::Format));
    }
    // SAFETY: `raw` is a freshly opened descriptor owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// Reads the mode, owner, and group of one already-open node.
pub(super) fn facts_of(node: BorrowedFd<'_>) -> Result<(u32, u32, u32), Error> {
    // SAFETY: `stat` is a plain C aggregate for which all-zero bytes are valid.
    let mut read: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: the descriptor is open and `read` is a valid writable buffer.
    if unsafe { libc::fstat(node.as_raw_fd(), &raw mut read) } != 0 {
        return Err(Error::kernel(Step::OpenNamespace));
    }
    Ok((read.st_mode, read.st_uid, read.st_gid))
}

/// Gives one already-open node the authority's owner, group, and mode.
pub(super) fn own_descriptor(
    node: BorrowedFd<'_>,
    authority: &ControlAuthority,
    mode: u32,
) -> Result<(), Error> {
    // SAFETY: the descriptor is open; `fchown` has no memory preconditions.
    if unsafe { libc::fchown(node.as_raw_fd(), authority.owner(), authority.group()) } != 0 {
        return Err(Error::kernel(Step::Bind));
    }
    // SAFETY: the descriptor is open; `fchmod` has no memory preconditions.
    if unsafe { libc::fchmod(node.as_raw_fd(), mode) } != 0 {
        return Err(Error::kernel(Step::Bind));
    }
    Ok(())
}

pub(super) fn facts(path: &CString) -> Result<Option<(u32, u32, u32)>, Error> {
    // SAFETY: `stat` is a plain C aggregate for which all-zero bytes are valid.
    let mut node: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: the path is a valid NUL-terminated string and `node` is a valid writable buffer.
    if unsafe { libc::lstat(path.as_ptr(), &raw mut node) } != 0 {
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ENOENT) => Ok(None),
            _ => Err(Error::kernel(Step::OpenNamespace)),
        };
    }
    Ok(Some((node.st_mode, node.st_uid, node.st_gid)))
}
