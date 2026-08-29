use std::net::{SocketAddrV4, TcpListener, UdpSocket};

use soma::{BackendFailureKind, EffectivePortPublication, HostBind, TransportProtocol};
use soma_macos::{
    InspectedMachine, PublishedPort as MacPublishedPort, TransportProtocol as MacProtocol,
};

use super::ActivationExpectation;

pub(in crate::backend::macos) fn verify_active(
    expected: &ActivationExpectation,
    inspection: &InspectedMachine,
) -> Result<(), BackendFailureKind> {
    let configured = configured_publications(inspection)?;
    if configured != expected.publications() {
        return Err(BackendFailureKind::IsolationFailure);
    }
    for publication in &configured {
        if endpoint_is_exclusively_available(publication)? {
            return Err(BackendFailureKind::IsolationFailure);
        }
    }
    Ok(())
}

pub(in crate::backend::macos) fn verify_released(
    publications: &[EffectivePortPublication],
) -> Result<(), BackendFailureKind> {
    for publication in publications {
        if !endpoint_is_exclusively_available(publication)? {
            return Err(BackendFailureKind::CleanupFailure);
        }
    }
    Ok(())
}

pub(in crate::backend::macos) fn configured_publications(
    inspection: &InspectedMachine,
) -> Result<Vec<EffectivePortPublication>, BackendFailureKind> {
    let configured = inspection
        .network()
        .published_ports()
        .ok_or(BackendFailureKind::IsolationFailure)?;
    effective_publications(configured)
}

pub(in crate::backend::macos) fn effective_publications(
    publications: &[MacPublishedPort],
) -> Result<Vec<EffectivePortPublication>, BackendFailureKind> {
    publications
        .iter()
        .map(mac_publication)
        .collect::<Result<Vec<_>, _>>()
}

fn mac_publication(
    publication: &MacPublishedPort,
) -> Result<EffectivePortPublication, BackendFailureKind> {
    let bind = HostBind::ipv4(publication.host_address())
        .map_err(|_| BackendFailureKind::IsolationFailure)?;
    EffectivePortPublication::new(
        bind,
        publication.host_port().get(),
        publication.guest_port().get(),
        match publication.protocol() {
            MacProtocol::Tcp => TransportProtocol::Tcp,
            MacProtocol::Udp => TransportProtocol::Udp,
        },
    )
    .map_err(|_| BackendFailureKind::IsolationFailure)
}

fn endpoint_is_exclusively_available(
    publication: &EffectivePortPublication,
) -> Result<bool, BackendFailureKind> {
    let HostBind::Ipv4 { address } = publication.bind() else {
        return Err(BackendFailureKind::Unsupported);
    };
    let endpoint = SocketAddrV4::new(address, publication.host_port().get());
    let result = match publication.protocol() {
        TransportProtocol::Tcp => TcpListener::bind(endpoint).map(|_socket| ()),
        TransportProtocol::Udp => UdpSocket::bind(endpoint).map(|_socket| ()),
    };
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => Ok(false),
        Err(_) => Err(BackendFailureKind::IsolationFailure),
    }
}
