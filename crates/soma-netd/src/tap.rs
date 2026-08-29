//! TAP creation inside the sandbox namespace.
//!
//! The device is opened through `/dev/net/tun` with `IFF_TAP | IFF_NO_PI`, is never made
//! persistent, receives the broker-side MAC, MTU, and gateway address, and stays down.
//! The returned descriptor is the only handle the VMM will ever receive.

#![allow(unsafe_code)]

use std::{
    fs::OpenOptions,
    net::Ipv4Addr,
    os::fd::{AsRawFd, OwnedFd},
};

use crate::{
    Error, Step,
    link::{self, ifreq},
};

const TUN_DEVICE: &str = "/dev/net/tun";
const MTU: u32 = 1500;

/// Everything the TAP needs at creation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TapSpec<'a> {
    pub(crate) name: &'a str,
    pub(crate) mac: [u8; 6],
    pub(crate) gateway: Ipv4Addr,
    pub(crate) prefix: u8,
}

/// Creates the TAP in the calling thread's namespace and returns its descriptor.
pub(crate) fn create(spec: TapSpec<'_>) -> Result<OwnedFd, Error> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(TUN_DEVICE)
        .map_err(|error| {
            if error.raw_os_error() == Some(libc::ENOENT) {
                Error::MissingPrivilege("/dev/net/tun")
            } else {
                Error::io(Step::OpenTun, &error)
            }
        })?;
    let fd: OwnedFd = file.into();
    let flags = i16::try_from(libc::IFF_TAP | libc::IFF_NO_PI)
        .map_err(|_| Error::InvalidState("tap flags"))?;
    let mut data = [0; 24];
    data[..2].copy_from_slice(&flags.to_ne_bytes());
    let req = ifreq(spec.name, data);
    // SAFETY: `TUNSETIFF` reads one `struct ifreq`; `req` is a valid 40-byte local of that
    // layout and the descriptor is an open `/dev/net/tun` handle.
    let result = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TUNSETIFF, &raw const req) };
    if result != 0 {
        return Err(Error::kernel(Step::TunSetIff));
    }
    let socket = link::control_socket()?;
    link::set_hwaddr(&socket, spec.name, spec.mac)?;
    link::set_mtu(&socket, spec.name, MTU)?;
    link::set_address(&socket, spec.name, spec.gateway, spec.prefix)?;
    if link::is_up(&socket, spec.name)? {
        return Err(Error::InvalidState("tap came up during creation"));
    }
    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_creation_without_privilege_fails_typed() {
        if crate::namespace::NetNamespace::probe_privilege().is_ok() {
            return;
        }
        let error = create(TapSpec {
            name: "tap0",
            mac: [2, 0, 0, 0, 0, 1],
            gateway: Ipv4Addr::new(10, 200, 0, 1),
            prefix: 30,
        })
        .expect_err("unprivileged");
        assert!(matches!(
            error,
            Error::MissingPrivilege(_)
                | Error::Kernel {
                    step: Step::OpenTun | Step::TunSetIff,
                    ..
                }
        ));
    }
}
