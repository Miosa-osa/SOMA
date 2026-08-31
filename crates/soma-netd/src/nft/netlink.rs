//! Direct `nf_tables` netlink for the broker's read-only table questions.
//!
//! Presence and listing are asked of `NETLINK_NETFILTER` with `NFT_MSG_GETTABLE` instead of
//! being read out of a pinned tool's standard output. A read carries no transaction, so it
//! costs neither a process spawn nor an `nf_tables` commit, which is what the two probes on the
//! activation path and the two on the release path were paying for.
//!
//! Only reads live here. Applying a ruleset stays on `nft` because the measured cost of an
//! application is the kernel's own commit rather than the tool: on the eval host a state
//! changing transaction costs about fourteen milliseconds whoever submits it, against about
//! one and a half milliseconds of tool startup and parsing, so encoding every expression over
//! netlink would buy a small fraction of one application and cost a large mechanism.
//!
//! The socket is opened per call and therefore belongs to the calling thread's namespace,
//! which is how a probe run inside [`crate::namespace::NetNamespace::within`] asks about the
//! sandbox namespace and the same call outside it asks about the host.

#![allow(unsafe_code)]
// Kernel ABI values are fixed-width by definition; the casts below convert `libc` constants
// and syscall lengths whose ranges are bounded by the buffer sizes declared in this module.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use crate::{Error, Step};

const NLMSG_HDRLEN: usize = 16;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 0x001;
const NLM_F_DUMP: u16 = 0x300;
const NFNL_SUBSYS_NFTABLES: u16 = 10;
const NFT_MSG_NEWTABLE: u16 = 0;
const NFT_MSG_GETTABLE: u16 = 1;
const NFTA_TABLE_NAME: u16 = 1;
const NFGENMSG_LEN: usize = 4;
const NFPROTO_INET: u8 = 1;
const NFNETLINK_V0: u8 = 0;
const ATTRIBUTE_KIND: u16 = 0x3fff;

/// One receive buffer; a dump that needs more is read again rather than truncated.
const RECEIVE: usize = 32 * 1024;

/// Receives one dump may take before the broker declares the answer unbounded.
///
/// A namespace the broker owns holds one table per bundle, so a dump that outruns this many
/// full buffers is not an answer worth acting on; refusing is safer than the silent shortening
/// a captured standard output would have produced.
const RECEIVES: usize = 64;

/// Reports whether one `inet` table exists in the calling thread's namespace.
pub(crate) fn table_exists(name: &str) -> Result<bool, Error> {
    let socket = open()?;
    send(&socket, &request(NLM_F_REQUEST, Some(name)))?;
    let mut buffer = vec![0_u8; RECEIVE];
    let received = receive(&socket, &mut buffer)?;
    presence(&buffer[..received])
}

/// The exact presence decision, separated from the exchange that produced the reply.
///
/// The kernel answers a named table request with the table itself or with `ENOENT`; any other
/// error is the operation's failure rather than an absence, so a probe can never report a
/// table gone because the question could not be asked.
fn presence(reply: &[u8]) -> Result<bool, Error> {
    let message = split(reply)?;
    match message.kind {
        NLMSG_ERROR => match errno(message.body)? {
            0 => Ok(true),
            errno if errno == libc::ENOENT => Ok(false),
            errno => Err(Error::Kernel {
                step: Step::Netlink,
                errno,
            }),
        },
        kind if kind == kind_of(NFT_MSG_NEWTABLE) => Ok(true),
        _ => Err(Error::Protocol("nftables reply kind")),
    }
}

/// Lists the `inet` table names of the calling thread's namespace, sorted.
pub(crate) fn list_tables() -> Result<Vec<String>, Error> {
    let socket = open()?;
    send(&socket, &request(NLM_F_REQUEST | NLM_F_DUMP, None))?;
    let mut names = Vec::new();
    let mut buffer = vec![0_u8; RECEIVE];
    for _ in 0..RECEIVES {
        let received = receive(&socket, &mut buffer)?;
        if collect(&buffer[..received], &mut names)? {
            names.sort_unstable();
            return Ok(names);
        }
    }
    Err(Error::Protocol("nftables dump unbounded"))
}

/// Encodes one `NFT_MSG_GETTABLE` request, for one named table or for every `inet` table.
fn request(flags: u16, name: Option<&str>) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(NLMSG_HDRLEN + NFGENMSG_LEN + 32);
    bytes.extend_from_slice(&0_u32.to_ne_bytes());
    bytes.extend_from_slice(&kind_of(NFT_MSG_GETTABLE).to_ne_bytes());
    bytes.extend_from_slice(&flags.to_ne_bytes());
    bytes.extend_from_slice(&1_u32.to_ne_bytes());
    bytes.extend_from_slice(&0_u32.to_ne_bytes());
    bytes.push(NFPROTO_INET);
    bytes.push(NFNETLINK_V0);
    // The nfnetlink resource identifier is big endian on the wire and unused for tables.
    bytes.extend_from_slice(&0_u16.to_be_bytes());
    if let Some(name) = name {
        let mut payload = name.as_bytes().to_vec();
        payload.push(0);
        let length = u16::try_from(4 + payload.len()).unwrap_or(u16::MAX);
        bytes.extend_from_slice(&length.to_ne_bytes());
        bytes.extend_from_slice(&NFTA_TABLE_NAME.to_ne_bytes());
        bytes.extend_from_slice(&payload);
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
    }
    let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    bytes[..4].copy_from_slice(&length.to_ne_bytes());
    bytes
}

const fn kind_of(kind: u16) -> u16 {
    (NFNL_SUBSYS_NFTABLES << 8) | kind
}

/// One netlink message taken off the front of a reply buffer.
struct Framed<'a> {
    kind: u16,
    body: &'a [u8],
    rest: &'a [u8],
}

/// Splits one message off the front of a buffer.
///
/// A header that claims more bytes than the buffer holds is refused rather than clamped, so a
/// short read can never be parsed as a complete answer.
fn split(reply: &[u8]) -> Result<Framed<'_>, Error> {
    if reply.len() < NLMSG_HDRLEN {
        return Err(Error::Protocol("netlink reply short"));
    }
    let length = u32::from_ne_bytes([reply[0], reply[1], reply[2], reply[3]]) as usize;
    if length < NLMSG_HDRLEN || length > reply.len() {
        return Err(Error::Protocol("netlink reply length"));
    }
    Ok(Framed {
        kind: u16::from_ne_bytes([reply[4], reply[5]]),
        body: &reply[NLMSG_HDRLEN..length],
        rest: &reply[length..],
    })
}

/// Reads the error number out of an `NLMSG_ERROR` body, which carries it negated.
fn errno(body: &[u8]) -> Result<i32, Error> {
    if body.len() < 4 {
        return Err(Error::Protocol("netlink error short"));
    }
    Ok(-i32::from_ne_bytes([body[0], body[1], body[2], body[3]]))
}

/// Appends the table names in one dump buffer and reports whether the dump ended.
fn collect(reply: &[u8], names: &mut Vec<String>) -> Result<bool, Error> {
    let mut rest = reply;
    while !rest.is_empty() {
        let message = split(rest)?;
        rest = message.rest;
        match message.kind {
            NLMSG_DONE => return Ok(true),
            NLMSG_ERROR => match errno(message.body)? {
                0 => return Ok(true),
                errno => {
                    return Err(Error::Kernel {
                        step: Step::Netlink,
                        errno,
                    });
                }
            },
            kind if kind == kind_of(NFT_MSG_NEWTABLE) => {
                if let Some(name) = table_name(message.body) {
                    names.push(name);
                }
            }
            _ => return Err(Error::Protocol("nftables reply kind")),
        }
    }
    Ok(false)
}

/// Reads `NFTA_TABLE_NAME` out of one `NFT_MSG_NEWTABLE` body.
fn table_name(body: &[u8]) -> Option<String> {
    let mut rest = body.get(NFGENMSG_LEN..)?;
    while rest.len() >= 4 {
        let length = u16::from_ne_bytes([rest[0], rest[1]]) as usize;
        let kind = u16::from_ne_bytes([rest[2], rest[3]]) & ATTRIBUTE_KIND;
        if length < 4 || length > rest.len() {
            return None;
        }
        if kind == NFTA_TABLE_NAME {
            let payload = &rest[4..length];
            let end = payload.iter().position(|byte| *byte == 0)?;
            return String::from_utf8(payload[..end].to_vec()).ok();
        }
        rest = rest.get(length.next_multiple_of(4)..)?;
    }
    None
}

fn open() -> Result<OwnedFd, Error> {
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before ownership
    // is taken.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_NETFILTER,
        )
    };
    if fd < 0 {
        return Err(Error::kernel(Step::Socket));
    }
    // SAFETY: `fd` is a freshly created descriptor owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn send(socket: &OwnedFd, message: &[u8]) -> Result<(), Error> {
    // SAFETY: `message` is a valid bounded buffer for its full length.
    let sent = unsafe {
        libc::send(
            socket.as_raw_fd(),
            message.as_ptr().cast(),
            message.len(),
            0,
        )
    };
    if sent < 0 || sent as usize != message.len() {
        return Err(Error::kernel(Step::SendMsg));
    }
    Ok(())
}

fn receive(socket: &OwnedFd, buffer: &mut [u8]) -> Result<usize, Error> {
    // SAFETY: `buffer` is a valid writable buffer of exactly the passed length.
    let received = unsafe {
        libc::recv(
            socket.as_raw_fd(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            0,
        )
    };
    if received < 0 {
        return Err(Error::kernel(Step::RecvMsg));
    }
    Ok(received as usize)
}

#[cfg(test)]
mod tests;
