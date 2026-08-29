use soma::{
    Capabilities, DnsPolicy, EgressPolicy, HostBind, HostPort, NetworkPolicy, PortPublication,
    TransportProtocol,
};

use crate::cli::{DnsInput, EgressInput, NetworkArgs, ProtocolInput};

pub(super) fn capabilities(arguments: NetworkArgs) -> Result<Capabilities, ()> {
    let egress = match arguments.egress {
        EgressInput::Unspecified => EgressPolicy::Unspecified,
        EgressInput::Denied => EgressPolicy::Denied,
        EgressInput::Internet => EgressPolicy::PublicInternet,
        EgressInput::Unrestricted => EgressPolicy::Unrestricted,
    };
    let dns = match (arguments.dns, arguments.dns_servers.is_empty()) {
        (DnsInput::Unspecified, true) => DnsPolicy::Unspecified,
        (DnsInput::Denied, true) => DnsPolicy::Denied,
        (DnsInput::System, true) => DnsPolicy::System,
        (DnsInput::Custom, false) => DnsPolicy::custom(arguments.dns_servers).map_err(|_| ())?,
        (DnsInput::Unspecified | DnsInput::Denied | DnsInput::System, false)
        | (DnsInput::Custom, true) => return Err(()),
    };
    let publications = arguments
        .publications
        .into_iter()
        .map(|publication| {
            let bind = match publication.bind_address {
                std::net::IpAddr::V4(address) => HostBind::ipv4(address),
                std::net::IpAddr::V6(address) => {
                    HostBind::ipv6(address, publication.v6_only.unwrap_or(true))
                }
            }
            .map_err(|_| ())?;
            PortPublication::new(
                bind,
                HostPort::from_u16(publication.host_port),
                publication.guest_port,
                match publication.protocol {
                    ProtocolInput::Tcp => TransportProtocol::Tcp,
                    ProtocolInput::Udp => TransportProtocol::Udp,
                },
            )
            .map_err(|_| ())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let policy = NetworkPolicy::new(egress, dns, publications).map_err(|_| ())?;
    Ok(Capabilities::isolated().with_network_policy(policy))
}
