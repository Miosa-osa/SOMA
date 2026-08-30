//! The exact ownership and permission decision for the socket directory and socket node.
//!
//! Reading a node is separated from judging it, so every drift class is decided by one pure
//! function the tests exercise directly.

#![allow(unsafe_code)]

use std::ffi::CString;

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
