//! Exclusive socket binding with explicit `IPV6_V6ONLY`.

#![allow(unsafe_code)]
// Socket ABI values are fixed-width by definition; the casts below convert `libc` constants
// and structure sizes whose ranges are bounded by the kernel structures they describe.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment
)]

use std::{
    mem,
    net::SocketAddr,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

use crate::{Error, Step};

/// Binds one socket exclusively and returns it with the port actually held.
pub(super) fn bind_exclusive(
    address: SocketAddr,
    v6_only: Option<bool>,
    stream: bool,
) -> Result<(OwnedFd, u16), Error> {
    let (domain, storage, length) = encode(address);
    let kind = if stream {
        libc::SOCK_STREAM
    } else {
        libc::SOCK_DGRAM
    };
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before ownership
    // is taken.
    let raw = unsafe { libc::socket(domain, kind | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(Error::kernel(Step::Socket));
    }
    // SAFETY: `raw` is a freshly created descriptor owned by nothing else.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    if let Some(v6_only) = v6_only {
        let value: libc::c_int = i32::from(v6_only);
        // SAFETY: the option value is one valid `c_int` and its exact size is passed.
        let result = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::IPPROTO_IPV6,
                libc::IPV6_V6ONLY,
                (&raw const value).cast(),
                mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if result != 0 {
            return Err(Error::kernel(Step::Socket));
        }
    }
    // SAFETY: `storage` is a fully initialised `sockaddr_storage` and `length` is the size of
    // the family-specific prefix that was written into it.
    let result = unsafe { libc::bind(socket.as_raw_fd(), (&raw const storage).cast(), length) };
    if result != 0 {
        return Err(Error::PortUnavailable);
    }
    if stream {
        // SAFETY: `listen` only reads the descriptor and backlog.
        if unsafe { libc::listen(socket.as_raw_fd(), 1) } != 0 {
            return Err(Error::PortUnavailable);
        }
    }
    // SAFETY: `sockaddr_storage` is a plain C aggregate for which all-zero bytes are valid.
    let mut bound: libc::sockaddr_storage = unsafe { mem::zeroed() };
    let mut bound_len = mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    // SAFETY: `getsockname` writes at most `bound_len` bytes into the zeroed storage and
    // updates the length in place.
    let result = unsafe {
        libc::getsockname(
            socket.as_raw_fd(),
            (&raw mut bound).cast(),
            &raw mut bound_len,
        )
    };
    if result != 0 {
        return Err(Error::kernel(Step::Bind));
    }
    Ok((socket, port_of(&bound)))
}

fn encode(address: SocketAddr) -> (libc::c_int, libc::sockaddr_storage, libc::socklen_t) {
    // SAFETY: `sockaddr_storage` is a plain C aggregate for which all-zero bytes are valid.
    let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
    match address {
        SocketAddr::V4(v4) => {
            let sin = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: v4.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(v4.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            // SAFETY: `sockaddr_in` is smaller than `sockaddr_storage` and both are plain C
            // aggregates, so a byte copy into the storage prefix is valid.
            unsafe {
                (&raw mut storage).cast::<libc::sockaddr_in>().write(sin);
            }
            (
                libc::AF_INET,
                storage,
                mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        }
        SocketAddr::V6(v6) => {
            let sin6 = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: 0,
                sin6_addr: libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                },
                sin6_scope_id: v6.scope_id(),
            };
            // SAFETY: as above for `sockaddr_in6`.
            unsafe {
                (&raw mut storage).cast::<libc::sockaddr_in6>().write(sin6);
            }
            (
                libc::AF_INET6,
                storage,
                mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
            )
        }
    }
}

fn port_of(storage: &libc::sockaddr_storage) -> u16 {
    // The port occupies bytes 2 and 3 in network order for both `sockaddr_in` and
    // `sockaddr_in6`, so it is read without reinterpreting the whole structure.
    // SAFETY: `sockaddr_storage` is at least four bytes long and fully initialised.
    let bytes: [u8; 4] =
        unsafe { *std::ptr::from_ref::<libc::sockaddr_storage>(storage).cast::<[u8; 4]>() };
    u16::from_be_bytes([bytes[2], bytes[3]])
}
