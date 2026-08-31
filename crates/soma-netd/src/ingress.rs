//! Host-side port reservation bookkeeping.
//!
//! Each requested publication is reserved by binding one real exclusive socket with an explicit
//! `IPV6_V6ONLY` value, so a conflicting listener is detected before assignment.
//! Reservation is transactional: any failure releases every socket taken so far.
//!
//! A reservation is not reachability. It proves only that nothing else owns the host endpoint;
//! the socket is deliberately never put into the listening state, so a client that reaches a
//! reserved but unpublished port is refused rather than accepted into a backlog the broker has
//! no way to serve. [`publish`] turns one reservation into the [`PublishedPort`] mapping that
//! [`crate::activate`] installs, and until that admitted activation step runs, nothing routes
//! to the guest.

#[cfg(target_os = "linux")]
use std::net::SocketAddr;
use std::net::{IpAddr, Ipv4Addr};

use soma::{HostBind, HostPort, PortPublication};

use crate::Error;

mod published;
#[cfg(target_os = "linux")]
mod socket;

pub use published::PublishedPort;

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
/// Always returns [`Error::Unimplemented`] in this slice; proxy profiles are already refused
/// at admission, so no admitted intent can reach this call.
pub fn attach_proxy(_reservation: &PortReservation) -> Result<(), Error> {
    Err(Error::Unimplemented("ingress proxy attachment"))
}

/// Turns one reservation into the destination mapping activation installs.
///
/// # Errors
///
/// Returns [`Error::Unimplemented`] for an IPv6 host bind, which cannot be translated onto the
/// IPv4-only guest lease of this profile slice. Admission already refuses such a publication,
/// so this is the second, local refusal rather than the first.
pub fn publish(reservation: &PortReservation, guest: Ipv4Addr) -> Result<PublishedPort, Error> {
    let publication = reservation.publication();
    let IpAddr::V4(address) = publication.bind().address() else {
        return Err(Error::Unimplemented("ingress forwarding for an IPv6 bind"));
    };
    Ok(PublishedPort::new(
        (!address.is_unspecified()).then_some(address),
        reservation.host_port(),
        guest,
        publication.guest_port().get(),
        publication.protocol(),
    ))
}

/// Turns every reservation of one assignment into its mapping, or none of them.
///
/// # Errors
///
/// Returns the first refusal from [`publish`].
pub fn publish_all(
    reservations: &[PortReservation],
    guest: Ipv4Addr,
) -> Result<Vec<PublishedPort>, Error> {
    reservations
        .iter()
        .map(|reservation| publish(reservation, guest))
        .collect()
}

#[cfg(target_os = "linux")]
fn reserve_one(publication: &PortPublication) -> Result<PortReservation, Error> {
    let requested = publication.host_port().requested().map_or(0, u16::from);
    let address = SocketAddr::new(publication.bind().address(), requested);
    let v6_only = publication.bind().v6_only();
    let (socket, host_port) = socket::bind_exclusive(
        address,
        v6_only,
        publication.protocol() == soma::TransportProtocol::Tcp,
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
mod tests;
