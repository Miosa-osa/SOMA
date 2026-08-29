use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};

use soma::{BackendFailureKind, HostPort, TransportProtocol};

pub(super) enum PortReservation {
    Tcp(TcpListener),
    Udp(UdpSocket),
}

impl PortReservation {
    pub(super) fn bind(
        address: Ipv4Addr,
        port: HostPort,
        protocol: TransportProtocol,
    ) -> Result<(Self, u16), BackendFailureKind> {
        let endpoint = SocketAddrV4::new(address, port.requested().map_or(0, |value| value.get()));
        match protocol {
            TransportProtocol::Tcp => {
                let socket = TcpListener::bind(endpoint).map_err(bind_failure)?;
                let selected = socket
                    .local_addr()
                    .map_err(|_| BackendFailureKind::Unavailable)?;
                Ok((Self::Tcp(socket), selected.port()))
            }
            TransportProtocol::Udp => {
                let socket = UdpSocket::bind(endpoint).map_err(bind_failure)?;
                let selected = socket
                    .local_addr()
                    .map_err(|_| BackendFailureKind::Unavailable)?;
                Ok((Self::Udp(socket), selected.port()))
            }
        }
    }

    pub(super) fn release(self) {
        match self {
            Self::Tcp(socket) => drop(socket),
            Self::Udp(socket) => drop(socket),
        }
    }
}

fn bind_failure(error: std::io::Error) -> BackendFailureKind {
    if error.kind() == std::io::ErrorKind::AddrInUse {
        BackendFailureKind::ResourceConflict
    } else {
        BackendFailureKind::Unavailable
    }
}
