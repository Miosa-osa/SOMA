use std::{net::IpAddr, str::FromStr};

use clap::{Args, ValueEnum};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum EgressInput {
    Unspecified,
    #[default]
    Denied,
    Internet,
    Unrestricted,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum DnsInput {
    Unspecified,
    #[default]
    Denied,
    System,
    Custom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolInput {
    Tcp,
    Udp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationInput {
    pub bind_address: IpAddr,
    pub v6_only: Option<bool>,
    pub host_port: u16,
    pub guest_port: u16,
    pub protocol: ProtocolInput,
}

impl FromStr for PublicationInput {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (mapping, v6_only) = parse_v6_mode(value)?;
        let (mapping, protocol) = parse_protocol(mapping)?;
        let (bind_address, host_port, guest_port) = parse_mapping(mapping)?;
        if bind_address.is_ipv4() && v6_only.is_some() {
            return Err("v6_only is valid only for an IPv6 bind".to_owned());
        }
        Ok(Self {
            bind_address,
            v6_only: bind_address.is_ipv6().then_some(v6_only.unwrap_or(true)),
            host_port,
            guest_port,
            protocol,
        })
    }
}

#[derive(Clone, Debug, Args)]
pub struct NetworkArgs {
    /// Guest-initiated connectivity. Internet excludes protected infrastructure destinations.
    #[arg(long, visible_alias = "network", value_enum, default_value = "denied")]
    pub egress: EgressInput,

    /// Guest resolver policy. Custom requires at least one --dns-server.
    #[arg(long, value_enum, default_value = "denied")]
    pub dns: DnsInput,

    /// Exact resolver address for --dns custom. Repeat for additional resolvers.
    #[arg(long = "dns-server", value_name = "IP")]
    pub dns_servers: Vec<IpAddr>,

    /// Publish [BIND:]HOST_PORT:GUEST_PORT[/tcp|udp][?v6_only=true|false]. Repeat as needed.
    #[arg(long = "publish", value_name = "SPEC")]
    pub publications: Vec<PublicationInput>,
}

fn parse_v6_mode(value: &str) -> Result<(&str, Option<bool>), String> {
    let Some((mapping, query)) = value.split_once('?') else {
        return Ok((value, None));
    };
    if query.contains('?') {
        return Err("publication has more than one query separator".to_owned());
    }
    let Some(raw) = query.strip_prefix("v6_only=") else {
        return Err("the only publication query is v6_only=true|false".to_owned());
    };
    let mode = raw
        .parse::<bool>()
        .map_err(|_| "v6_only must be true or false".to_owned())?;
    Ok((mapping, Some(mode)))
}

fn parse_protocol(value: &str) -> Result<(&str, ProtocolInput), String> {
    let Some((mapping, protocol)) = value.rsplit_once('/') else {
        return Ok((value, ProtocolInput::Tcp));
    };
    let protocol = match protocol {
        "tcp" => ProtocolInput::Tcp,
        "udp" => ProtocolInput::Udp,
        _ => return Err("publication protocol must be tcp or udp".to_owned()),
    };
    Ok((mapping, protocol))
}

fn parse_mapping(value: &str) -> Result<(IpAddr, u16, u16), String> {
    if let Some(without_open) = value.strip_prefix('[') {
        let close = without_open
            .find(']')
            .ok_or_else(|| "IPv6 publication bind is missing ]".to_owned())?;
        let address = without_open[..close]
            .parse::<IpAddr>()
            .map_err(|_| "publication bind address is invalid".to_owned())?;
        if !address.is_ipv6() {
            return Err("bracketed publication binds must be IPv6".to_owned());
        }
        let ports = without_open[close + 1..].strip_prefix(':').ok_or_else(|| {
            "publication bind must be followed by host and guest ports".to_owned()
        })?;
        let (host, guest) = two_ports(ports)?;
        return Ok((address, host, guest));
    }
    let components = value.split(':').collect::<Vec<_>>();
    match components.as_slice() {
        [host, guest] => Ok((IpAddr::from([127, 0, 0, 1]), port(host)?, port(guest)?)),
        [address, host, guest] => Ok((
            address
                .parse()
                .map_err(|_| "publication bind address is invalid".to_owned())?,
            port(host)?,
            port(guest)?,
        )),
        _ => Err("publication must contain host and guest ports".to_owned()),
    }
}

fn two_ports(value: &str) -> Result<(u16, u16), String> {
    let mut values = value.split(':');
    let host = values
        .next()
        .ok_or_else(|| "publication host port is missing".to_owned())?;
    let guest = values
        .next()
        .ok_or_else(|| "publication guest port is missing".to_owned())?;
    if values.next().is_some() {
        return Err("publication contains too many ports".to_owned());
    }
    Ok((port(host)?, port(guest)?))
}

fn port(value: &str) -> Result<u16, String> {
    value
        .parse()
        .map_err(|_| "publication port must be between 0 and 65535".to_owned())
}

#[cfg(test)]
mod tests {
    use std::{net::IpAddr, str::FromStr as _};

    use super::{ProtocolInput, PublicationInput};

    #[test]
    fn parses_safe_defaults_and_explicit_dual_stack_udp() {
        let safe = PublicationInput::from_str("0:8080").expect("safe publication");
        let dual = PublicationInput::from_str("[::]:9000:53/udp?v6_only=false")
            .expect("dual-stack publication");

        assert_eq!(safe.bind_address, IpAddr::from([127, 0, 0, 1]));
        assert_eq!(safe.host_port, 0);
        assert_eq!(safe.protocol, ProtocolInput::Tcp);
        assert_eq!(dual.v6_only, Some(false));
        assert_eq!(dual.protocol, ProtocolInput::Udp);
    }
}
