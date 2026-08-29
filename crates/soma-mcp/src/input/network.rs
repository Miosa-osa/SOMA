use std::net::IpAddr;

use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use soma::{
    Capabilities, DnsPolicy, EgressPolicy, HostBind, HostPort, MAX_DNS_SERVERS,
    MAX_PORT_PUBLICATIONS, NetworkPolicy, PortPublication, TransportProtocol,
};

use super::InputError;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum EgressInput {
    Unspecified,
    #[default]
    Denied,
    Internet,
    Unrestricted,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum DnsInput {
    Unspecified,
    #[default]
    Denied,
    System,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProtocolInput {
    #[default]
    Tcp,
    Udp,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PublicationInput {
    #[serde(default = "default_bind_address")]
    #[schemars(default = "default_bind_address")]
    pub bind_address: String,
    #[serde(default)]
    pub v6_only: Option<bool>,
    #[serde(default)]
    #[schemars(default)]
    pub host_port: u16,
    #[schemars(range(min = 1))]
    pub guest_port: u16,
    #[serde(default)]
    #[schemars(default)]
    pub protocol: ProtocolInput,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NetworkInput {
    #[serde(default)]
    #[schemars(default)]
    egress: EgressInput,
    #[serde(default)]
    #[schemars(default)]
    dns: DnsInput,
    #[serde(default)]
    #[schemars(length(max = MAX_DNS_SERVERS), inner(length(min = 2, max = 45)))]
    dns_servers: Vec<String>,
    #[serde(default)]
    #[schemars(length(max = MAX_PORT_PUBLICATIONS))]
    published_ports: Vec<PublicationInput>,
}

impl Default for NetworkInput {
    fn default() -> Self {
        Self {
            egress: EgressInput::Denied,
            dns: DnsInput::Denied,
            dns_servers: Vec::new(),
            published_ports: Vec::new(),
        }
    }
}

impl NetworkInput {
    pub(super) fn capabilities(self) -> Result<Capabilities, InputError> {
        let egress = match self.egress {
            EgressInput::Unspecified => EgressPolicy::Unspecified,
            EgressInput::Denied => EgressPolicy::Denied,
            EgressInput::Internet => EgressPolicy::PublicInternet,
            EgressInput::Unrestricted => EgressPolicy::Unrestricted,
        };
        let servers = self
            .dns_servers
            .into_iter()
            .map(|value| value.parse::<IpAddr>().map_err(|_| InputError::Network))
            .collect::<Result<Vec<_>, _>>()?;
        let dns = match (self.dns, servers.is_empty()) {
            (DnsInput::Unspecified, true) => DnsPolicy::Unspecified,
            (DnsInput::Denied, true) => DnsPolicy::Denied,
            (DnsInput::System, true) => DnsPolicy::System,
            (DnsInput::Custom, false) => {
                DnsPolicy::custom(servers).map_err(|_| InputError::Network)?
            }
            (DnsInput::Unspecified | DnsInput::Denied | DnsInput::System, false)
            | (DnsInput::Custom, true) => return Err(InputError::Network),
        };
        let publications = self
            .published_ports
            .iter()
            .map(publication)
            .collect::<Result<Vec<_>, _>>()?;
        let policy =
            NetworkPolicy::new(egress, dns, publications).map_err(|_| InputError::Network)?;
        Ok(Capabilities::isolated().with_network_policy(policy))
    }
}

fn publication(input: &PublicationInput) -> Result<PortPublication, InputError> {
    let address = input
        .bind_address
        .parse::<IpAddr>()
        .map_err(|_| InputError::Network)?;
    let bind = match address {
        IpAddr::V4(address) if input.v6_only.is_none() => HostBind::ipv4(address),
        IpAddr::V4(_) => return Err(InputError::Network),
        IpAddr::V6(address) => HostBind::ipv6(address, input.v6_only.unwrap_or(true)),
    }
    .map_err(|_| InputError::Network)?;
    PortPublication::new(
        bind,
        HostPort::from_u16(input.host_port),
        input.guest_port,
        match input.protocol {
            ProtocolInput::Tcp => TransportProtocol::Tcp,
            ProtocolInput::Udp => TransportProtocol::Udp,
        },
    )
    .map_err(|_| InputError::Network)
}

fn default_bind_address() -> String {
    "127.0.0.1".to_owned()
}

#[cfg(test)]
mod tests {
    use soma::{DnsPolicy, EgressPolicy, HostPort};

    use super::NetworkInput;

    #[test]
    fn default_is_fully_isolated() {
        let capabilities = NetworkInput::default().capabilities().expect("network");
        let policy = capabilities.network_policy();

        assert_eq!(policy.egress(), EgressPolicy::Denied);
        assert_eq!(policy.dns(), &DnsPolicy::Denied);
        assert!(policy.published_ports().is_empty());
    }

    #[test]
    fn automatic_publication_is_preserved() {
        let input: NetworkInput = serde_json::from_value(serde_json::json!({
            "egress": "unrestricted",
            "dns": "system",
            "published_ports": [{"guest_port": 8080}]
        }))
        .expect("input");
        let capabilities = input.capabilities().expect("network");

        assert_eq!(
            capabilities.network_policy().published_ports()[0].host_port(),
            HostPort::Automatic
        );
    }
}
