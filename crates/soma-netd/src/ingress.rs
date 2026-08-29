//! Host-side port reservation bookkeeping.
//!
//! Each requested publication is reserved by binding one real exclusive socket with an explicit
//! `IPV6_V6ONLY` value, so a conflicting listener is detected before assignment.
//! Reservation is transactional: any failure releases every socket taken so far.
//! Forwarding or proxy attachment to a reserved port is a later slice and reports
//! [`Error::Unimplemented`].

use std::net::{IpAddr, SocketAddr};

use soma::{HostBind, HostPort, PortPublication, TransportProtocol};

use crate::Error;

#[cfg(target_os = "linux")]
mod socket;

/// One held reservation.
#[derive(Debug)]
pub struct PortReservation {
    publication: PortPublication,
    host_port: u16,
    #[cfg(target_os = "linux")]
    _socket: std::os::fd::OwnedFd,
}

impl PortReservation {
    /// Returns the requested publication.
    #[must_use]
    pub const fn publication(&self) -> &PortPublication {
        &self.publication
    }

    /// Returns the exact host port held, resolved for automatic requests.
    #[must_use]
    pub const fn host_port(&self) -> u16 {
        self.host_port
    }
}

/// Reserves every publication or none.
///
/// # Errors
///
/// Returns [`Error::PortUnavailable`] when any exclusive bind fails, or
/// [`Error::Unimplemented`] outside Linux.
pub fn reserve(publications: &[PortPublication]) -> Result<Vec<PortReservation>, Error> {
    let mut held = Vec::with_capacity(publications.len());
    for publication in publications {
        held.push(reserve_one(publication)?);
    }
    Ok(held)
}

/// Attaches the operator proxy route for one reservation.
///
/// # Errors
///
/// Always returns [`Error::Unimplemented`] in this slice.
pub fn attach_proxy(_reservation: &PortReservation) -> Result<(), Error> {
    Err(Error::Unimplemented("ingress proxy attachment"))
}

/// Publishes one reserved port toward the guest.
///
/// # Errors
///
/// Always returns [`Error::Unimplemented`] in this slice; ports are reserved but never
/// forwarded before the ingress activation slice exists.
pub fn publish(_reservation: &PortReservation) -> Result<(), Error> {
    Err(Error::Unimplemented("ingress forwarding"))
}

#[cfg(target_os = "linux")]
fn reserve_one(publication: &PortPublication) -> Result<PortReservation, Error> {
    let requested = publication.host_port().requested().map_or(0, u16::from);
    let address = SocketAddr::new(publication.bind().address(), requested);
    let v6_only = publication.bind().v6_only();
    let (socket, host_port) = socket::bind_exclusive(
        address,
        v6_only,
        publication.protocol() == TransportProtocol::Tcp,
    )?;
    Ok(PortReservation {
        publication: publication.clone(),
        host_port,
        _socket: socket,
    })
}

#[cfg(not(target_os = "linux"))]
fn reserve_one(_publication: &PortPublication) -> Result<PortReservation, Error> {
    Err(Error::Unimplemented("ingress reservation outside Linux"))
}

/// Describes one reservation for evidence without exposing the socket.
#[must_use]
pub fn describe(reservation: &PortReservation) -> (IpAddr, u16, Option<bool>, HostPort) {
    let bind = reservation.publication().bind();
    let v6_only = match bind {
        HostBind::Ipv4 { .. } => None,
        HostBind::Ipv6 { v6_only, .. } => Some(v6_only),
    };
    (
        bind.address(),
        reservation.host_port(),
        v6_only,
        reservation.publication().host_port(),
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn publication(host_port: u16, protocol: TransportProtocol) -> PortPublication {
        PortPublication::new(
            HostBind::loopback_v4(),
            HostPort::from_u16(host_port),
            80,
            protocol,
        )
        .expect("publication")
    }

    #[test]
    fn automatic_ports_resolve_and_fixed_conflicts_roll_back_the_transaction() {
        let automatic = reserve(&[publication(0, TransportProtocol::Tcp)]).expect("automatic");
        let port = automatic[0].host_port();
        assert_ne!(port, 0);
        assert_eq!(describe(&automatic[0]).3, HostPort::Automatic);
        let error = reserve(&[
            publication(0, TransportProtocol::Udp),
            publication(port, TransportProtocol::Tcp),
        ])
        .expect_err("conflict");
        assert_eq!(error, Error::PortUnavailable);
        drop(automatic);
        let again = reserve(&[publication(port, TransportProtocol::Tcp)]).expect("released");
        assert_eq!(again[0].host_port(), port);
        assert_eq!(
            attach_proxy(&again[0]).expect_err("proxy"),
            Error::Unimplemented("ingress proxy attachment")
        );
        assert_eq!(
            publish(&again[0]).expect_err("forwarding"),
            Error::Unimplemented("ingress forwarding")
        );
    }

    #[test]
    fn ipv6_binds_carry_an_explicit_v6only_value() {
        let publication = PortPublication::new(
            HostBind::ipv6(std::net::Ipv6Addr::LOCALHOST, true).expect("bind"),
            HostPort::Automatic,
            80,
            TransportProtocol::Tcp,
        )
        .expect("publication");
        let held = reserve(&[publication]).expect("v6");
        assert_eq!(describe(&held[0]).2, Some(true));
    }
}
