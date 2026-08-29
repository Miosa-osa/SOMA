use std::{
    collections::BTreeSet,
    ffi::OsString,
    net::{IpAddr, Ipv4Addr},
};

use serde_json::Value;

use crate::{
    CommandFailure, CommandFailureReason, ExecutionStatus, InspectedNetwork, InstanceId,
    MachineResources, NetworkAddress, NetworkAttachment, Operation, OwnershipFailure,
    PublishedPort, TransportProtocol,
};

use super::MacOsBackend;

const OWNERSHIP_TIMEOUT_MILLIS: u64 = 30_000;
const OWNERSHIP_OUTPUT_BYTES: u64 = 65_536;
const OWNERSHIP_LABEL: &str = "io.miosa.soma.instance";

pub(super) struct OwnedInspection {
    pub(super) document: Value,
    pub(super) resources: Option<MachineResources>,
    pub(super) network: InspectedNetwork,
}

pub(super) struct OwnedCleanup {
    pub(super) resources: Option<MachineResources>,
    pub(super) published_ports: Option<Vec<PublishedPort>>,
}

impl MacOsBackend {
    pub(super) fn inspect_owned(
        &self,
        instance_id: &InstanceId,
    ) -> Result<OwnedInspection, CommandFailure> {
        let output = self.commands.execute(
            Operation::Inspect,
            vec![
                OsString::from("inspect"),
                OsString::from(instance_id.container_name()),
            ],
            OWNERSHIP_TIMEOUT_MILLIS,
            OWNERSHIP_OUTPUT_BYTES,
        )?;
        require_success(Operation::Inspect, output.status())?;
        verify_document(output.stdout(), instance_id)
    }

    pub(super) fn force_delete_owned(
        &self,
        instance_id: &InstanceId,
    ) -> Result<OwnedCleanup, CommandFailure> {
        let inspection = self.inspect_owned(instance_id)?;
        let published_ports = inspection.network.published_ports().map(<[_]>::to_vec);
        let output = self.commands.execute(
            Operation::Delete,
            vec![
                OsString::from("delete"),
                OsString::from("--force"),
                OsString::from(instance_id.container_name()),
            ],
            OWNERSHIP_TIMEOUT_MILLIS,
            OWNERSHIP_OUTPUT_BYTES,
        )?;
        require_success(Operation::Delete, output.status())?;
        Ok(OwnedCleanup {
            resources: inspection.resources,
            published_ports,
        })
    }
}

fn verify_document(
    document: &[u8],
    instance_id: &InstanceId,
) -> Result<OwnedInspection, CommandFailure> {
    let document = serde_json::from_slice::<Value>(document)
        .map_err(|_| ownership_failure(OwnershipFailure::InvalidJson))?;
    let records = document
        .as_array()
        .ok_or_else(|| ownership_failure(OwnershipFailure::MalformedRecord))?;
    let record = match records.as_slice() {
        [] => return Err(ownership_failure(OwnershipFailure::MissingRecord)),
        [record] => record,
        _ => return Err(ownership_failure(OwnershipFailure::MultipleRecords)),
    };
    let record = record
        .as_object()
        .ok_or_else(|| ownership_failure(OwnershipFailure::MalformedRecord))?;
    let configuration = record
        .get("configuration")
        .and_then(Value::as_object)
        .ok_or_else(|| ownership_failure(OwnershipFailure::MalformedRecord))?;
    let expected_name = instance_id.container_name();
    if record.get("id").and_then(Value::as_str) != Some(expected_name.as_str())
        || configuration.get("id").and_then(Value::as_str) != Some(expected_name.as_str())
    {
        return Err(ownership_failure(OwnershipFailure::NameMismatch));
    }
    let labels = configuration
        .get("labels")
        .and_then(Value::as_object)
        .ok_or_else(|| ownership_failure(OwnershipFailure::MalformedRecord))?;
    let label = labels
        .get(OWNERSHIP_LABEL)
        .ok_or_else(|| ownership_failure(OwnershipFailure::MissingLabel))?
        .as_str()
        .ok_or_else(|| ownership_failure(OwnershipFailure::MalformedRecord))?;
    if label != instance_id.as_str() {
        return Err(ownership_failure(OwnershipFailure::LabelMismatch));
    }
    let resources = parse_resources(configuration);
    let network = parse_network(configuration, record);
    Ok(OwnedInspection {
        document,
        resources,
        network,
    })
}

fn parse_network(
    configuration: &serde_json::Map<String, Value>,
    record: &serde_json::Map<String, Value>,
) -> InspectedNetwork {
    let attachment = parse_network_attachment(configuration, record);
    let dns_servers = parse_dns_servers(configuration);
    let published_ports = parse_published_ports(configuration);
    let addresses = attachment.and_then(|value| parse_addresses(record, value));
    InspectedNetwork::new(attachment, dns_servers, published_ports, addresses)
}

fn parse_network_attachment(
    configuration: &serde_json::Map<String, Value>,
    record: &serde_json::Map<String, Value>,
) -> Option<NetworkAttachment> {
    let configured = parse_network_names(configuration.get("networks")?)?;
    let active = parse_network_names(record.get("status")?.get("networks")?)?;
    if configured != active {
        return None;
    }
    if configured.is_empty() {
        Some(NetworkAttachment::Detached)
    } else {
        Some(NetworkAttachment::Attached)
    }
}

fn parse_network_names(value: &Value) -> Option<BTreeSet<&str>> {
    let mut names = BTreeSet::new();
    for network in value.as_array()? {
        let name = network.get("network")?.as_str()?;
        if name.is_empty() || !names.insert(name) {
            return None;
        }
    }
    Some(names)
}

fn parse_dns_servers(configuration: &serde_json::Map<String, Value>) -> Option<Vec<IpAddr>> {
    let nameservers = configuration.get("dns")?.get("nameservers")?.as_array()?;
    let mut servers = nameservers
        .iter()
        .map(|value| value.as_str()?.parse::<IpAddr>().ok())
        .collect::<Option<Vec<_>>>()?;
    if servers.iter().any(|address| {
        address.is_unspecified()
            || address.is_multicast()
            || matches!(address, IpAddr::V4(value) if value.is_broadcast())
    }) {
        return None;
    }
    servers.sort_unstable();
    servers.dedup();
    (servers.len() == nameservers.len()).then_some(servers)
}

fn parse_published_ports(
    configuration: &serde_json::Map<String, Value>,
) -> Option<Vec<PublishedPort>> {
    let records = configuration.get("publishedPorts")?.as_array()?;
    let mut ports = records
        .iter()
        .map(|record| {
            let record = record.as_object()?;
            if record.get("count")?.as_u64()? != 1 {
                return None;
            }
            let host_address = record
                .get("hostAddress")?
                .as_str()?
                .parse::<Ipv4Addr>()
                .ok()?;
            let host_port = u16::try_from(record.get("hostPort")?.as_u64()?).ok()?;
            let guest_port = u16::try_from(record.get("containerPort")?.as_u64()?).ok()?;
            let protocol = match record.get("proto")?.as_str()? {
                "tcp" => TransportProtocol::Tcp,
                "udp" => TransportProtocol::Udp,
                _ => return None,
            };
            PublishedPort::new(host_address, host_port, guest_port, protocol).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    ports.sort_unstable();
    ports.dedup();
    (ports.len() == records.len()).then_some(ports)
}

fn parse_addresses(
    record: &serde_json::Map<String, Value>,
    attachment: NetworkAttachment,
) -> Option<Vec<NetworkAddress>> {
    let networks = record.get("status")?.get("networks")?.as_array()?;
    if attachment == NetworkAttachment::Detached {
        return networks.is_empty().then(Vec::new);
    }
    let mut addresses = Vec::new();
    for network in networks {
        for field in ["ipv4Address", "ipv6Address"] {
            if let Some(value) = network.get(field) {
                addresses.push(parse_address(value.as_str()?)?);
            }
        }
    }
    addresses.sort_unstable();
    addresses.dedup();
    Some(addresses)
}

fn parse_address(value: &str) -> Option<NetworkAddress> {
    let (address, prefix) = value.rsplit_once('/')?;
    NetworkAddress::new(address.parse().ok()?, prefix.parse().ok()?)
}

fn parse_resources(configuration: &serde_json::Map<String, Value>) -> Option<MachineResources> {
    let resources = configuration.get("resources")?.as_object()?;
    let vcpus = u16::try_from(resources.get("cpus")?.as_u64()?).ok()?;
    let memory_bytes = resources.get("memoryInBytes")?.as_u64()?;
    if vcpus == 0 || memory_bytes == 0 || !memory_bytes.is_multiple_of(1_048_576) {
        return None;
    }
    Some(MachineResources::new(vcpus, memory_bytes))
}

fn require_success(operation: Operation, status: ExecutionStatus) -> Result<(), CommandFailure> {
    if status.is_success() {
        Ok(())
    } else {
        Err(CommandFailure::new(
            operation,
            CommandFailureReason::Status(status),
        ))
    }
}

const fn ownership_failure(failure: OwnershipFailure) -> CommandFailure {
    CommandFailure::new(Operation::Inspect, CommandFailureReason::Ownership(failure))
}
