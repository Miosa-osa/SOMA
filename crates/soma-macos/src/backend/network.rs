use std::ffi::OsString;

use crate::{DnsConfiguration, NetworkConfiguration, NetworkPolicy, TransportProtocol};

pub(super) fn append_network(arguments: &mut Vec<OsString>, network: &NetworkConfiguration) {
    match network.attachment() {
        NetworkPolicy::Unspecified => {}
        NetworkPolicy::Denied => arguments.extend(["--network".into(), "none".into()]),
        NetworkPolicy::Allowed => arguments.extend(["--network".into(), "default".into()]),
    }
    if let DnsConfiguration::Custom { servers } = network.dns() {
        for server in servers {
            arguments.extend(["--dns".into(), server.to_string().into()]);
        }
    }
    for publication in network.published_ports() {
        let protocol = match publication.protocol() {
            TransportProtocol::Tcp => "tcp",
            TransportProtocol::Udp => "udp",
        };
        arguments.extend([
            "--publish".into(),
            format!(
                "{}:{}:{}/{}",
                publication.host_address(),
                publication.host_port(),
                publication.guest_port(),
                protocol
            )
            .into(),
        ]);
    }
}
