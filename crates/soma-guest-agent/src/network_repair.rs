//! Network identity repair over the classic `ioctl` interface.
//!
//! The link is forced down, the fresh MAC and IPv4 identity are installed, the link is raised,
//! the default route is added, and the resolver file is written from launch material.
//! No netlink library is used; every request is one fixed-layout `ifreq` or `rtentry`.
//!
//! Those layouts are hand-encoded for exactly one target ABI, so [`repair`] first requires the
//! compiled ABI to be the verified one and refuses every other target with
//! [`NetworkStep::UnsupportedTarget`] before it opens a socket or reaches an unsafe `ioctl`.
//! See [`target`] for the verified ABI and for how the binary gate and this check relate.

#![allow(unsafe_code)]

use std::fs;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};

use soma_guest::LaunchNetwork;

use crate::ioctl;

use self::encoding::{
    IFF_RUNNING, IFF_UP, IFREQ_DATA, IfReq, RouteEntry, flags_data, hwaddr_data, ifreq, inet_data,
};

mod encoding;
mod target;

/// Interface name of the single virtio network device.
pub const INTERFACE: &str = "eth0";
const LOOPBACK: &str = "lo";
const RESOLV_CONF: &str = "/etc/resolv.conf";
const HOSTS_FILE: &str = "/etc/hosts";

/// Redacted network-repair failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkError {
    /// The step that failed.
    pub step: NetworkStep,
    /// The kernel errno.
    pub errno: i32,
}

/// One typed network-repair step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkStep {
    /// Refuse a target ABI whose interface-request layouts are not verified.
    UnsupportedTarget,
    /// Open the control socket.
    Socket,
    /// Force the link down before changing identity.
    LinkDown,
    /// Install the fresh MAC address.
    Mac,
    /// Install the IPv4 address.
    Address,
    /// Install the IPv4 netmask.
    Netmask,
    /// Raise the loopback and primary links.
    LinkUp,
    /// Install the default route.
    Route,
    /// Write the resolver and hosts files.
    Resolver,
}

/// Installs the fresh network identity from the launch page.
///
/// # Errors
///
/// Returns [`NetworkStep::UnsupportedTarget`] when the compiled ABI is not the one whose
/// request layouts are verified, otherwise the first failed step with its errno.
pub fn repair(network: &LaunchNetwork, hostname: &str) -> Result<(), NetworkError> {
    target::require(target::COMPILED)?;
    let socket = control_socket()?;
    let flags = get_flags(&socket, INTERFACE, NetworkStep::LinkDown)?;
    set_flags(&socket, INTERFACE, flags & !IFF_UP, NetworkStep::LinkDown)?;
    request(
        &socket,
        libc::SIOCSIFHWADDR,
        &ifreq(INTERFACE, hwaddr_data(network.mac())),
        NetworkStep::Mac,
    )?;
    request(
        &socket,
        libc::SIOCSIFADDR,
        &ifreq(INTERFACE, inet_data(network.address())),
        NetworkStep::Address,
    )?;
    request(
        &socket,
        libc::SIOCSIFNETMASK,
        &ifreq(INTERFACE, inet_data(network.netmask())),
        NetworkStep::Netmask,
    )?;
    let up = IFF_UP | IFF_RUNNING;
    let loopback = get_flags(&socket, LOOPBACK, NetworkStep::LinkUp)?;
    set_flags(&socket, LOOPBACK, loopback | up, NetworkStep::LinkUp)?;
    set_flags(&socket, INTERFACE, flags | up, NetworkStep::LinkUp)?;
    add_default_route(&socket, network.gateway())?;
    fs::write(RESOLV_CONF, resolver_file(network.resolver()))
        .and_then(|()| fs::write(HOSTS_FILE, hosts_file(hostname, network.address())))
        .map_err(|error| NetworkError {
            step: NetworkStep::Resolver,
            errno: error.raw_os_error().unwrap_or(0),
        })
}

/// Raises loopback and installs nothing else, for a machine with no network device.
///
/// A sandbox that may not reach the network still has to reach itself: a workload that binds a
/// port on `127.0.0.1` and then connects to it is doing something entirely local, and leaving
/// `lo` down would break it for a reason that has nothing to do with the policy that denied it
/// egress. There is no interface, address, route, resolver, or hosts file to write, and the
/// last two would fail anyway on a read-only root.
///
/// # Errors
///
/// Returns [`NetworkStep::UnsupportedTarget`] for an unverified ABI, otherwise the failed step.
pub fn repair_loopback_only() -> Result<(), NetworkError> {
    target::require(target::COMPILED)?;
    let socket = control_socket()?;
    let loopback = get_flags(&socket, LOOPBACK, NetworkStep::LinkUp)?;
    set_flags(
        &socket,
        LOOPBACK,
        loopback | IFF_UP | IFF_RUNNING,
        NetworkStep::LinkUp,
    )
}

/// Renders the resolver configuration for the single launch resolver.
#[must_use]
pub fn resolver_file(resolver: [u8; 4]) -> String {
    format!(
        "nameserver {}\noptions timeout:2 attempts:2\n",
        dotted(resolver)
    )
}

/// Renders the hosts file binding the fresh hostname to the fresh address.
#[must_use]
pub fn hosts_file(hostname: &str, address: [u8; 4]) -> String {
    format!("127.0.0.1 localhost\n{} {hostname}\n", dotted(address))
}

fn dotted(address: [u8; 4]) -> String {
    format!(
        "{}.{}.{}.{}",
        address[0], address[1], address[2], address[3]
    )
}

fn control_socket() -> Result<OwnedFd, NetworkError> {
    // SAFETY: `socket` has no memory preconditions; the descriptor is checked before ownership
    // is taken.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(failure(NetworkStep::Socket));
    }
    // SAFETY: `fd` is a freshly created descriptor owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn request(
    socket: &OwnedFd,
    code: libc::c_ulong,
    req: &IfReq,
    step: NetworkStep,
) -> Result<(), NetworkError> {
    // SAFETY: every interface `ioctl` used here reads or writes exactly one `struct ifreq`,
    // and `req` is a valid 40-byte local matching that layout.
    let result = unsafe { libc::ioctl(socket.as_raw_fd(), ioctl::request(code), req) };
    if result == 0 {
        Ok(())
    } else {
        Err(failure(step))
    }
}

fn get_flags(socket: &OwnedFd, name: &str, step: NetworkStep) -> Result<i16, NetworkError> {
    let mut req = ifreq(name, [0; IFREQ_DATA]);
    // SAFETY: `SIOCGIFFLAGS` writes the flags into the supplied valid `struct ifreq`.
    let result = unsafe {
        libc::ioctl(
            socket.as_raw_fd(),
            ioctl::request(libc::SIOCGIFFLAGS),
            &raw mut req,
        )
    };
    if result != 0 {
        return Err(failure(step));
    }
    Ok(i16::from_ne_bytes([req.data[0], req.data[1]]))
}

fn set_flags(
    socket: &OwnedFd,
    name: &str,
    flags: i16,
    step: NetworkStep,
) -> Result<(), NetworkError> {
    request(
        socket,
        libc::SIOCSIFFLAGS,
        &ifreq(name, flags_data(flags)),
        step,
    )
}

fn add_default_route(socket: &OwnedFd, gateway: [u8; 4]) -> Result<(), NetworkError> {
    let mut route = RouteEntry {
        pad1: 0,
        dst: [0; 16],
        gateway: [0; 16],
        genmask: [0; 16],
        flags: (libc::RTF_UP | libc::RTF_GATEWAY),
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
    route.dst[..16].copy_from_slice(&inet_data([0; 4])[..16]);
    route.genmask[..16].copy_from_slice(&inet_data([0; 4])[..16]);
    route.gateway[..16].copy_from_slice(&inet_data(gateway)[..16]);
    // SAFETY: `SIOCADDRT` reads exactly one `struct rtentry`; `route` is a valid local with the
    // 120-byte x86_64 layout asserted by the tests and a null device pointer.
    let result = unsafe {
        libc::ioctl(
            socket.as_raw_fd(),
            ioctl::request(libc::SIOCADDRT),
            &raw const route,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(failure(NetworkStep::Route))
    }
}

fn failure(step: NetworkStep) -> NetworkError {
    NetworkError {
        step,
        errno: io::Error::last_os_error().raw_os_error().unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_and_hosts_files_are_rendered_from_launch_material() {
        assert_eq!(
            resolver_file([10, 0, 0, 1]),
            "nameserver 10.0.0.1\noptions timeout:2 attempts:2\n"
        );
        assert_eq!(
            hosts_file("soma-abc", [10, 0, 0, 2]),
            "127.0.0.1 localhost\n10.0.0.2 soma-abc\n"
        );
    }

    #[test]
    fn repair_refuses_an_unverified_target_before_it_opens_a_control_socket() {
        assert_eq!(target::require(target::COMPILED), Ok(()));
        assert_eq!(
            target::require(target::TargetAbi {
                architecture: "aarch64",
                ..target::VERIFIED
            }),
            Err(NetworkError {
                step: NetworkStep::UnsupportedTarget,
                errno: libc::ENOTSUP,
            })
        );
    }

    #[test]
    fn reading_flags_of_a_missing_interface_fails_with_a_kernel_errno() {
        let socket = control_socket().expect("datagram socket");
        let error = get_flags(&socket, "soma-none0", NetworkStep::LinkDown).expect_err("missing");
        assert_eq!(error.step, NetworkStep::LinkDown);
        assert_eq!(error.errno, libc::ENODEV);
    }
}
