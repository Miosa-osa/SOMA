//! A minimal rtnetlink encoder for veth creation and link deletion.
//!
//! Only `RTM_NEWLINK` with a peer placed directly into a target namespace and `RTM_DELLINK`
//! by name are encoded; every message is bounded, built from `libc` constants, and confirmed
//! by the kernel's `NLMSG_ERROR` acknowledgement.

#![allow(unsafe_code)]
// Kernel ABI values are fixed-width by definition; the casts below convert `libc` constants
// and syscall lengths whose ranges are bounded by the message sizes asserted in the tests.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use crate::{Error, Step};

const NLMSG_HDRLEN: usize = 16;
const NLMSG_ERROR: u16 = 2;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;
const NLA_F_NESTED: u16 = 0x8000;
const IFLA_IFNAME: u16 = 3;
const IFLA_LINKINFO: u16 = 18;
const IFLA_NET_NS_FD: u16 = 28;
const IFLA_INFO_KIND: u16 = 1;
const IFLA_INFO_DATA: u16 = 2;
const VETH_INFO_PEER: u16 = 1;
const IFINFOMSG_LEN: usize = 16;
const MAX_MESSAGE: usize = 256;
const MAX_REPLY: usize = 4096;

struct Message {
    bytes: Vec<u8>,
}

impl Message {
    fn new(kind: u16, flags: u16, seq: u32) -> Self {
        let mut bytes = Vec::with_capacity(MAX_MESSAGE);
        bytes.extend_from_slice(&0_u32.to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(
            &(libc::NLM_F_REQUEST as u16 | libc::NLM_F_ACK as u16 | flags).to_ne_bytes(),
        );
        bytes.extend_from_slice(&seq.to_ne_bytes());
        bytes.extend_from_slice(&0_u32.to_ne_bytes());
        bytes.extend_from_slice(&[0; IFINFOMSG_LEN]);
        Self { bytes }
    }

    fn attr(&mut self, kind: u16, payload: &[u8]) {
        let length = u16::try_from(4 + payload.len()).unwrap_or(u16::MAX);
        self.bytes.extend_from_slice(&length.to_ne_bytes());
        self.bytes.extend_from_slice(&kind.to_ne_bytes());
        self.bytes.extend_from_slice(payload);
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
    }

    fn nested(&mut self, kind: u16, build: impl FnOnce(&mut Self)) {
        let start = self.bytes.len();
        self.bytes.extend_from_slice(&[0; 4]);
        build(self);
        let length = u16::try_from(self.bytes.len() - start).unwrap_or(u16::MAX);
        self.bytes[start..start + 2].copy_from_slice(&length.to_ne_bytes());
        self.bytes[start + 2..start + 4].copy_from_slice(&(kind | NLA_F_NESTED).to_ne_bytes());
    }

    fn name(&mut self, name: &str) {
        let mut payload = name.as_bytes()[..name.len().min(15)].to_vec();
        payload.push(0);
        self.attr(IFLA_IFNAME, &payload);
    }

    fn finish(mut self) -> Vec<u8> {
        let length = u32::try_from(self.bytes.len()).unwrap_or(u32::MAX);
        self.bytes[..4].copy_from_slice(&length.to_ne_bytes());
        self.bytes
    }
}

/// Creates a veth pair: `host_name` in the calling thread's namespace and `peer_name` inside
/// the namespace referenced by `peer_ns`.
pub(crate) fn create_veth(
    host_name: &str,
    peer_name: &str,
    peer_ns: BorrowedFd<'_>,
) -> Result<(), Error> {
    let mut message = Message::new(libc::RTM_NEWLINK, NLM_F_CREATE | NLM_F_EXCL, 1);
    message.name(host_name);
    let raw_ns = u32::try_from(peer_ns.as_raw_fd()).map_err(|_| Error::InvalidState("ns fd"))?;
    message.nested(IFLA_LINKINFO, |message| {
        message.attr(IFLA_INFO_KIND, b"veth\0");
        message.nested(IFLA_INFO_DATA, |message| {
            message.nested(VETH_INFO_PEER, |message| {
                message.bytes.extend_from_slice(&[0; IFINFOMSG_LEN]);
                message.name(peer_name);
                message.attr(IFLA_NET_NS_FD, &raw_ns.to_ne_bytes());
            });
        });
    });
    transact(&message.finish())
}

/// Deletes one link by name in the calling thread's namespace.
///
/// Returns `Ok(false)` when the link was already absent.
pub(crate) fn delete_link(name: &str) -> Result<bool, Error> {
    let mut message = Message::new(libc::RTM_DELLINK, 0, 2);
    message.name(name);
    match transact(&message.finish()) {
        Ok(()) => Ok(true),
        Err(Error::Kernel { errno, .. }) if errno == libc::ENODEV => Ok(false),
        Err(error) => Err(error),
    }
}

fn transact(message: &[u8]) -> Result<(), Error> {
    let socket = route_socket()?;
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
        return Err(Error::kernel(Step::Netlink));
    }
    let mut reply = [0_u8; MAX_REPLY];
    // SAFETY: `reply` is a valid writable buffer of exactly the passed length.
    let received = unsafe {
        libc::recv(
            socket.as_raw_fd(),
            reply.as_mut_ptr().cast(),
            reply.len(),
            0,
        )
    };
    if received < 0 {
        return Err(Error::kernel(Step::Netlink));
    }
    parse_ack(&reply[..received as usize])
}

fn parse_ack(reply: &[u8]) -> Result<(), Error> {
    if reply.len() < NLMSG_HDRLEN + 4 {
        return Err(Error::Protocol("netlink reply short"));
    }
    let kind = u16::from_ne_bytes([reply[4], reply[5]]);
    if kind != NLMSG_ERROR {
        return Err(Error::Protocol("netlink reply kind"));
    }
    let error = i32::from_ne_bytes([reply[16], reply[17], reply[18], reply[19]]);
    if error == 0 {
        Ok(())
    } else {
        Err(Error::Kernel {
            step: Step::Netlink,
            errno: -error,
        })
    }
}

fn route_socket() -> Result<OwnedFd, Error> {
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before ownership
    // is taken.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        )
    };
    if fd < 0 {
        return Err(Error::kernel(Step::Socket));
    }
    // SAFETY: `fd` is a freshly created descriptor owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_aligned_bounded_and_carry_the_nested_peer() {
        let mut message = Message::new(libc::RTM_NEWLINK, NLM_F_CREATE | NLM_F_EXCL, 1);
        message.name("sv0a0b0c0d");
        message.nested(IFLA_LINKINFO, |message| {
            message.attr(IFLA_INFO_KIND, b"veth\0");
            message.nested(IFLA_INFO_DATA, |message| {
                message.nested(VETH_INFO_PEER, |message| {
                    message.bytes.extend_from_slice(&[0; IFINFOMSG_LEN]);
                    message.name("vs0");
                    message.attr(IFLA_NET_NS_FD, &7_u32.to_ne_bytes());
                });
            });
        });
        let bytes = message.finish();
        assert_eq!(bytes.len() % 4, 0);
        assert!(bytes.len() < MAX_MESSAGE);
        assert_eq!(
            u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
            bytes.len()
        );
        assert_eq!(u16::from_ne_bytes([bytes[4], bytes[5]]), libc::RTM_NEWLINK);
        let flags = u16::from_ne_bytes([bytes[6], bytes[7]]);
        assert_eq!(flags & NLM_F_CREATE, NLM_F_CREATE);
        assert_eq!(flags & NLM_F_EXCL, NLM_F_EXCL);
        let linkinfo = &bytes[NLMSG_HDRLEN + IFINFOMSG_LEN + 16..];
        assert_eq!(
            u16::from_ne_bytes([linkinfo[2], linkinfo[3]]),
            IFLA_LINKINFO | NLA_F_NESTED
        );
        assert!(bytes.windows(5).any(|window| window == b"veth\0"));
        assert!(bytes.windows(4).any(|window| window == b"vs0\0"));
    }

    #[test]
    fn acks_are_parsed_and_errors_carry_errno() {
        let mut ack = vec![0_u8; 20];
        ack[4..6].copy_from_slice(&NLMSG_ERROR.to_ne_bytes());
        assert_eq!(parse_ack(&ack), Ok(()));
        ack[16..20].copy_from_slice(&(-libc::EEXIST).to_ne_bytes());
        assert_eq!(
            parse_ack(&ack),
            Err(Error::Kernel {
                step: Step::Netlink,
                errno: libc::EEXIST
            })
        );
        assert_eq!(
            parse_ack(&ack[..10]),
            Err(Error::Protocol("netlink reply short"))
        );
        assert!(delete_link("soma-none0").is_err() || !delete_link("soma-none0").expect("absent"));
    }
}
