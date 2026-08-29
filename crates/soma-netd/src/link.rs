//! Per-interface configuration through the classic `ioctl` interface.
//!
//! Every call acts inside the namespace of the calling thread, so callers run these functions
//! through [`crate::namespace::NetNamespace::within`] or directly for the host namespace.
//! Each request is one fixed-layout `ifreq` or `rtentry`; no netlink library is used.

#![allow(unsafe_code)]

use std::{
    fs,
    net::Ipv4Addr,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

use crate::{Cidr, Error, Step};

const IFNAMSIZ: usize = 16;
const IFREQ_DATA: usize = 24;
const ARPHRD_ETHER: u16 = 1;
const AF_INET: u16 = 2;
const IFF_UP: i16 = 0x1;

#[repr(C)]
pub(crate) struct IfReq {
    name: [u8; IFNAMSIZ],
    data: [u8; IFREQ_DATA],
}

#[repr(C)]
struct RouteEntry {
    pad1: u64,
    dst: [u8; 16],
    gateway: [u8; 16],
    genmask: [u8; 16],
    flags: u16,
    pad2: i16,
    pad3: u64,
    tos: u8,
    class: u8,
    pad4: [i16; 3],
    metric: i16,
    dev: *mut libc::c_char,
    mtu: u64,
    window: u64,
    irtt: u16,
}

/// One `AF_INET` datagram control socket.
pub(crate) fn control_socket() -> Result<OwnedFd, Error> {
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before ownership
    // is taken.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(Error::kernel(Step::Socket));
    }
    // SAFETY: `fd` is a freshly created descriptor owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub(crate) fn ifreq(name: &str, data: [u8; IFREQ_DATA]) -> IfReq {
    let mut request = IfReq {
        name: [0; IFNAMSIZ],
        data,
    };
    let bytes = name.as_bytes();
    let length = bytes.len().min(IFNAMSIZ - 1);
    request.name[..length].copy_from_slice(&bytes[..length]);
    request
}

fn inet_data(address: Ipv4Addr) -> [u8; IFREQ_DATA] {
    let mut data = [0; IFREQ_DATA];
    data[..2].copy_from_slice(&AF_INET.to_ne_bytes());
    data[4..8].copy_from_slice(&address.octets());
    data
}

pub(crate) fn request(
    socket: &OwnedFd,
    code: libc::c_ulong,
    req: &IfReq,
    step: Step,
) -> Result<(), Error> {
    // SAFETY: every interface `ioctl` used here reads or writes exactly one `struct ifreq`,
    // and `req` is a valid 40-byte local matching that layout.
    let result = unsafe { libc::ioctl(socket.as_raw_fd(), code, req) };
    if result == 0 {
        Ok(())
    } else {
        Err(Error::kernel(step))
    }
}

/// Installs one MAC on a link that is down.
pub(crate) fn set_hwaddr(socket: &OwnedFd, name: &str, mac: [u8; 6]) -> Result<(), Error> {
    let mut data = [0; IFREQ_DATA];
    data[..2].copy_from_slice(&ARPHRD_ETHER.to_ne_bytes());
    data[2..8].copy_from_slice(&mac);
    request(
        socket,
        libc::SIOCSIFHWADDR,
        &ifreq(name, data),
        Step::SetHwaddr,
    )
}

/// Sets the link MTU.
pub(crate) fn set_mtu(socket: &OwnedFd, name: &str, mtu: u32) -> Result<(), Error> {
    let mut data = [0; IFREQ_DATA];
    data[..4].copy_from_slice(&mtu.to_ne_bytes());
    request(socket, libc::SIOCSIFMTU, &ifreq(name, data), Step::SetMtu)
}

/// Installs one IPv4 address and prefix.
pub(crate) fn set_address(
    socket: &OwnedFd,
    name: &str,
    address: Ipv4Addr,
    prefix: u8,
) -> Result<(), Error> {
    request(
        socket,
        libc::SIOCSIFADDR,
        &ifreq(name, inet_data(address)),
        Step::SetAddress,
    )?;
    let mask = Ipv4Addr::from_bits(if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    });
    request(
        socket,
        libc::SIOCSIFNETMASK,
        &ifreq(name, inet_data(mask)),
        Step::SetNetmask,
    )
}

/// Reads whether the link is administratively up.
pub(crate) fn is_up(socket: &OwnedFd, name: &str) -> Result<bool, Error> {
    let mut req = ifreq(name, [0; IFREQ_DATA]);
    // SAFETY: `SIOCGIFFLAGS` writes the flags into the supplied valid `struct ifreq`.
    let result = unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCGIFFLAGS, &raw mut req) };
    if result != 0 {
        return Err(Error::kernel(Step::GetFlags));
    }
    Ok(i16::from_ne_bytes([req.data[0], req.data[1]]) & IFF_UP != 0)
}

/// Raises or lowers the link.
pub(crate) fn set_up(socket: &OwnedFd, name: &str, up: bool) -> Result<(), Error> {
    let mut req = ifreq(name, [0; IFREQ_DATA]);
    // SAFETY: `SIOCGIFFLAGS` writes the flags into the supplied valid `struct ifreq`.
    let result = unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCGIFFLAGS, &raw mut req) };
    if result != 0 {
        return Err(Error::kernel(Step::GetFlags));
    }
    let flags = i16::from_ne_bytes([req.data[0], req.data[1]]);
    let flags = if up { flags | IFF_UP } else { flags & !IFF_UP };
    let mut data = [0; IFREQ_DATA];
    data[..2].copy_from_slice(&flags.to_ne_bytes());
    request(
        socket,
        libc::SIOCSIFFLAGS,
        &ifreq(name, data),
        Step::SetFlags,
    )
}

/// Adds one IPv4 route to `destination` through `gateway`; a `/0` destination is the default.
pub(crate) fn add_route(
    socket: &OwnedFd,
    destination: Cidr,
    gateway: Ipv4Addr,
) -> Result<(), Error> {
    let Cidr::V4(network, prefix) = destination else {
        return Err(Error::Unimplemented("ipv6 route"));
    };
    let mask = Ipv4Addr::from_bits(if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    });
    let mut route = RouteEntry {
        pad1: 0,
        dst: [0; 16],
        gateway: [0; 16],
        genmask: [0; 16],
        flags: libc::RTF_UP | libc::RTF_GATEWAY,
        pad2: 0,
        pad3: 0,
        tos: 0,
        class: 0,
        pad4: [0; 3],
        metric: 0,
        dev: std::ptr::null_mut(),
        mtu: 0,
        window: 0,
        irtt: 0,
    };
    route.dst.copy_from_slice(&inet_data(network)[..16]);
    route.genmask.copy_from_slice(&inet_data(mask)[..16]);
    route.gateway.copy_from_slice(&inet_data(gateway)[..16]);
    // SAFETY: `SIOCADDRT` reads exactly one `struct rtentry`; `route` is a valid local with
    // the 120-byte x86_64 layout asserted by the tests and a null device pointer.
    let result = unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCADDRT, &raw const route) };
    if result == 0 {
        Ok(())
    } else {
        Err(Error::kernel(Step::AddRoute))
    }
}

/// Lists the interface names of the calling thread's namespace from `/proc`.
pub(crate) fn list_links() -> Result<Vec<String>, Error> {
    let text = fs::read_to_string("/proc/thread-self/net/dev")
        .map_err(|error| Error::io(Step::ListLinks, &error))?;
    let mut names: Vec<String> = text
        .lines()
        .skip(2)
        .filter_map(|line| line.split(':').next())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_unstable();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_structure_layouts_are_exact() {
        assert_eq!(std::mem::size_of::<IfReq>(), 40);
        assert_eq!(std::mem::size_of::<RouteEntry>(), 120);
        assert_eq!(i32::from(AF_INET), libc::AF_INET);
        assert_eq!(i32::from(IFF_UP), libc::IFF_UP);
        let req = ifreq(
            "a-very-long-interface-name",
            inet_data(Ipv4Addr::new(10, 0, 0, 2)),
        );
        assert_eq!(&req.name[..15], b"a-very-long-int");
        assert_eq!(req.name[15], 0);
        assert_eq!(&req.data[4..8], &[10, 0, 0, 2]);
    }

    #[test]
    fn missing_interfaces_fail_with_enodev_and_loopback_is_listed() {
        let socket = control_socket().expect("socket");
        let error = is_up(&socket, "soma-none0").expect_err("missing");
        assert_eq!(
            error,
            Error::Kernel {
                step: Step::GetFlags,
                errno: libc::ENODEV
            }
        );
        assert!(list_links().expect("links").iter().any(|name| name == "lo"));
    }
}
