//! A connection from inside a bundle's namespace to its own guest.
//!
//! The guest's network is a TAP in a namespace of its own, and nothing outside that namespace
//! has a route to it. The broker already enters the namespace to build it, so the broker is
//! also the one party that can open a connection to a service the guest runs, such as the live
//! browser, without publishing a host port for it.
//!
//! The connection is made from inside the namespace, so the socket belongs to it, and the
//! caller receives the connected socket and nothing else: it never holds a namespace
//! descriptor, a route, or an address. What the broker is willing to dial is narrow on purpose:
//!
//! - the destination is the guest address of the bundle named, taken from the bundle itself
//!   and never from the request, so there is no way to name another bundle's guest, an address
//!   outside the lease, or the host;
//! - the port must be one the broker was configured to allow;
//! - the bundle must be activated, so a guest whose session was never repaired is not dialed.
//!
//! The sandbox ruleset admits exactly this: a new connection out of the namespace to the guest
//! address on the allowed ports, and the guest's replies to it.

use std::{
    net::{SocketAddr, SocketAddrV4, TcpStream},
    ops::RangeInclusive,
    os::fd::OwnedFd,
    time::Duration,
};

use crate::{Assigned, Broker, Error, Step};

/// The guest ports a broker may dial: the live browser's control and view ports.
const BROWSER_PORTS: RangeInclusive<u16> = 9223..=9224;

/// The longest one connection attempt waits before it gives up.
///
/// The broker serves one request at a time, so this bounds how long a guest that drops the
/// connection attempt can hold every other caller.
pub const CONNECT_DEADLINE: Duration = Duration::from_secs(2);

/// The guest ports [`Broker::connect_guest`] will dial.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectPorts {
    ranges: Vec<RangeInclusive<u16>>,
}

impl Default for ConnectPorts {
    fn default() -> Self {
        Self {
            ranges: vec![BROWSER_PORTS],
        }
    }
}

impl ConnectPorts {
    /// Builds an allowlist from inclusive ranges.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidProfile`] for an empty list, an empty range, or port zero.
    pub fn new(ranges: &[RangeInclusive<u16>]) -> Result<Self, Error> {
        if ranges.is_empty()
            || ranges
                .iter()
                .any(|range| range.is_empty() || *range.start() == 0)
        {
            return Err(Error::InvalidProfile("connect ports"));
        }
        Ok(Self {
            ranges: ranges.to_vec(),
        })
    }

    /// The allowed ranges, in the order they were given.
    #[must_use]
    pub fn ranges(&self) -> &[RangeInclusive<u16>] {
        &self.ranges
    }

    /// Whether `port` may be dialed.
    #[must_use]
    pub fn permits(&self, port: u16) -> bool {
        port != 0 && self.ranges.iter().any(|range| range.contains(&port))
    }
}

impl Broker {
    /// Connects to `port` on the guest of `assigned` from inside its namespace.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidState`] for a bundle that is not activated,
    /// [`Error::Unauthorized`] for a port outside the allowlist, or the kernel failure at
    /// [`Step::Connect`] when the guest does not answer within [`CONNECT_DEADLINE`].
    pub fn connect_guest(&self, assigned: &Assigned, port: u16) -> Result<OwnedFd, Error> {
        self.connect_plan(assigned, port)?.dial()
    }

    /// Checks a dial and captures everything it needs, so the connection itself can be made
    /// without holding the lock that guards the assignment.
    ///
    /// # Errors
    ///
    /// Returns an error when the bundle is not activated or the port is not allowed.
    pub fn connect_plan(&self, assigned: &Assigned, port: u16) -> Result<ConnectPlan, Error> {
        if !assigned.is_active() {
            return Err(Error::InvalidState("bundle is not activated"));
        }
        if !self.connect_ports().permits(port) {
            return Err(Error::Unauthorized("connect port"));
        }
        let guest = assigned.bundle().leases().guest.guest();
        Ok(ConnectPlan {
            pin: assigned.bundle().namespace().path().to_path_buf(),
            address: SocketAddr::V4(SocketAddrV4::new(guest, port)),
        })
    }
}

/// One checked dial into one bundle's guest.
#[derive(Debug)]
pub struct ConnectPlan {
    pin: std::path::PathBuf,
    address: SocketAddr,
}

impl ConnectPlan {
    /// Dials from inside the bundle's namespace.
    ///
    /// # Errors
    ///
    /// Returns the namespace-entry or connection failure.
    pub fn dial(&self) -> Result<OwnedFd, Error> {
        let address = self.address;
        crate::namespace::NetNamespace::open(&self.pin)?.within(move || {
            let stream = TcpStream::connect_timeout(&address, CONNECT_DEADLINE)
                .map_err(|error| Error::io(Step::Connect, &error))?;
            // Interactive traffic: a frame or a keystroke must not wait for a full segment.
            stream
                .set_nodelay(true)
                .map_err(|error| Error::io(Step::Connect, &error))?;
            Ok(OwnedFd::from(stream))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_allowlist_is_the_browser_and_nothing_else() {
        let ports = ConnectPorts::default();
        assert!(ports.permits(9223) && ports.permits(9224));
        for refused in [0, 22, 53, 80, 443, 3000, 9222, 9225, 65535] {
            assert!(!ports.permits(refused), "{refused}");
        }
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init, clippy::reversed_empty_ranges)]
    fn an_allowlist_is_never_empty_and_never_allows_port_zero() {
        assert!(ConnectPorts::new(&[]).is_err());
        assert!(ConnectPorts::new(&[0..=10]).is_err());
        let inverted = [10..=5];
        assert!(ConnectPorts::new(&inverted).is_err());
        let ports = ConnectPorts::new(&[3000..=3010, 5173..=5173]).expect("ports");
        assert!(ports.permits(3005) && ports.permits(5173));
        assert!(!ports.permits(2999) && !ports.permits(3011) && !ports.permits(5174));
    }
}
