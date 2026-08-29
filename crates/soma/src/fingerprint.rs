use sha2::{Digest, Sha256};

use crate::{
    DirectCommand, ExecutionLimits, InstanceId, MachineName, MachineShape, OciImage,
    RequestFingerprint, WorkloadIdentity,
};

pub(crate) fn source(image: &OciImage) -> RequestFingerprint {
    let mut encoder = CanonicalHash::new(b"soma.oci-source.v1");
    encoder.field(b"image", image.as_str().as_bytes());
    encoder.finish()
}

pub(crate) fn run(
    workload: &WorkloadIdentity,
    instance_id: &InstanceId,
    machine_name: Option<&MachineName>,
    shape: &MachineShape,
    command: &DirectCommand,
    limits: &ExecutionLimits,
) -> RequestFingerprint {
    let mut encoder = CanonicalHash::new(b"soma.run-request.v2");
    workload_fields(&mut encoder, workload);
    encoder.field(b"instance_id", instance_id.as_str().as_bytes());
    machine_name_field(&mut encoder, machine_name);
    shape_fields(&mut encoder, shape);
    command_fields(&mut encoder, command);
    encoder.u64(b"timeout_ms", limits.timeout_ms());
    encoder.u64(b"max_output_bytes", limits.max_output_bytes());
    encoder.finish()
}

pub(crate) fn launch(
    workload: &WorkloadIdentity,
    instance_id: &InstanceId,
    machine_name: Option<&MachineName>,
    shape: &MachineShape,
) -> RequestFingerprint {
    let mut encoder = CanonicalHash::new(b"soma.managed-launch.v2");
    workload_fields(&mut encoder, workload);
    encoder.field(b"instance_id", instance_id.as_str().as_bytes());
    machine_name_field(&mut encoder, machine_name);
    shape_fields(&mut encoder, shape);
    encoder.finish()
}

pub(crate) fn execute(
    workload: &WorkloadIdentity,
    instance_id: &InstanceId,
    command: &DirectCommand,
    limits: &ExecutionLimits,
) -> RequestFingerprint {
    let mut encoder = CanonicalHash::new(b"soma.managed-execute.v1");
    workload_fields(&mut encoder, workload);
    encoder.field(b"instance_id", instance_id.as_str().as_bytes());
    command_fields(&mut encoder, command);
    encoder.u64(b"timeout_ms", limits.timeout_ms());
    encoder.u64(b"max_output_bytes", limits.max_output_bytes());
    encoder.finish()
}

pub(crate) fn stop(workload: &WorkloadIdentity, instance_id: &InstanceId) -> RequestFingerprint {
    let mut encoder = CanonicalHash::new(b"soma.managed-stop.v1");
    workload_fields(&mut encoder, workload);
    encoder.field(b"instance_id", instance_id.as_str().as_bytes());
    encoder.finish()
}

pub(crate) fn inspect(workload: &WorkloadIdentity, instance_id: &InstanceId) -> RequestFingerprint {
    let mut encoder = CanonicalHash::new(b"soma.managed-inspect.v1");
    workload_fields(&mut encoder, workload);
    encoder.field(b"instance_id", instance_id.as_str().as_bytes());
    encoder.finish()
}

pub(crate) fn destroy(workload: &WorkloadIdentity, instance_id: &InstanceId) -> RequestFingerprint {
    let mut encoder = CanonicalHash::new(b"soma.managed-destroy.v1");
    workload_fields(&mut encoder, workload);
    encoder.field(b"instance_id", instance_id.as_str().as_bytes());
    encoder.finish()
}

pub(crate) fn bytes(value: &[u8]) -> String {
    digest_bytes(value).as_str().to_owned()
}

pub(crate) fn digest_bytes(value: &[u8]) -> RequestFingerprint {
    RequestFingerprint::from_digest(Sha256::digest(value).into())
}

fn workload_fields(encoder: &mut CanonicalHash, workload: &WorkloadIdentity) {
    encoder.optional_field(
        b"index_digest",
        workload
            .index_digest()
            .map(|digest| digest.as_str().as_bytes()),
    );
    encoder.field(
        b"manifest_digest",
        workload.manifest_digest().as_str().as_bytes(),
    );
    encoder.field(
        b"operating_system",
        workload.platform().operating_system().as_bytes(),
    );
    encoder.field(
        b"architecture",
        workload.platform().architecture().as_bytes(),
    );
    encoder.optional_field(b"variant", workload.platform().variant().map(str::as_bytes));
    encoder.optional_field(
        b"generation_id",
        workload
            .generation_id()
            .map(|value| value.as_str().as_bytes()),
    );
}

fn shape_fields(encoder: &mut CanonicalHash, shape: &MachineShape) {
    encoder.u64(b"vcpu_count", u64::from(shape.vcpu_count()));
    encoder.u64(b"memory_mib", shape.memory_mib());
    encoder.u64(b"storage_mib", shape.storage_mib());
    network_fields(encoder, shape.capabilities().network_policy());
}

fn network_fields(encoder: &mut CanonicalHash, policy: &crate::NetworkPolicy) {
    network_profile_fields(encoder, policy.profile());
    let addresses = policy.guest_addresses();
    encoder.field(b"network_ipv4_mode", &[addresses.ipv4().fingerprint_code()]);
    if let Some(address) = addresses.ipv4().requested_address() {
        encoder.field(b"network_ipv4_address", address.to_string().as_bytes());
    }
    encoder.field(b"network_ipv6_mode", &[addresses.ipv6().fingerprint_code()]);
    if let Some(address) = addresses.ipv6().requested_address() {
        encoder.field(b"network_ipv6_address", address.to_string().as_bytes());
    }
    proxy_fields(encoder, policy.proxy());
    encoder.field(
        b"network_egress",
        &[match policy.egress() {
            crate::EgressPolicy::Unspecified => 0,
            crate::EgressPolicy::Denied => 1,
            crate::EgressPolicy::PublicInternet => 2,
            crate::EgressPolicy::Unrestricted => 3,
        }],
    );
    encoder.field(
        b"network_dns_mode",
        &[match policy.dns() {
            crate::DnsPolicy::Unspecified => 0,
            crate::DnsPolicy::Denied => 1,
            crate::DnsPolicy::System => 2,
            crate::DnsPolicy::Custom { .. } => 3,
        }],
    );
    encoder.u64(
        b"network_dns_server_count",
        u64::try_from(policy.dns().servers().len()).expect("bounded DNS count fits u64"),
    );
    for server in policy.dns().servers() {
        encoder.field(b"network_dns_server", server.to_string().as_bytes());
    }
    encoder.u64(
        b"network_publication_count",
        u64::try_from(policy.published_ports().len()).expect("bounded publication count fits u64"),
    );
    for publication in policy.published_ports() {
        encoder.field(
            b"network_bind_address",
            publication.bind().address().to_string().as_bytes(),
        );
        encoder.field(
            b"network_bind_v6_only",
            &[publication
                .bind()
                .v6_only()
                .map_or(0, |value| if value { 2 } else { 1 })],
        );
        encoder.u64(
            b"network_host_port",
            u64::from(
                publication
                    .host_port()
                    .requested()
                    .map_or(0, std::num::NonZeroU16::get),
            ),
        );
        encoder.u64(
            b"network_guest_port",
            u64::from(publication.guest_port().get()),
        );
        encoder.field(
            b"network_protocol",
            &[match publication.protocol() {
                crate::TransportProtocol::Tcp => 0,
                crate::TransportProtocol::Udp => 1,
            }],
        );
    }
}

fn network_profile_fields(encoder: &mut CanonicalHash, profile: &crate::NetworkProfileSelector) {
    match profile {
        crate::NetworkProfileSelector::Disabled => encoder.field(b"network_profile_mode", &[0]),
        crate::NetworkProfileSelector::OperatorDefault => {
            encoder.field(b"network_profile_mode", &[1]);
        }
        crate::NetworkProfileSelector::Named {
            profile_id,
            revision,
        } => {
            encoder.field(b"network_profile_mode", &[2]);
            encoder.field(b"network_profile_id", profile_id.as_str().as_bytes());
            encoder.field(b"network_profile_revision", revision.as_str().as_bytes());
        }
    }
}

fn proxy_fields(encoder: &mut CanonicalHash, proxy: &crate::ProxyPolicy) {
    match proxy {
        crate::ProxyPolicy::Disabled => encoder.field(b"network_proxy_mode", &[0]),
        crate::ProxyPolicy::Required { profile } => {
            encoder.field(b"network_proxy_mode", &[1]);
            match profile {
                crate::ProxyProfileSelector::OperatorDefault => {
                    encoder.field(b"network_proxy_profile_mode", &[0]);
                }
                crate::ProxyProfileSelector::Named {
                    profile_id,
                    revision,
                } => {
                    encoder.field(b"network_proxy_profile_mode", &[1]);
                    encoder.field(b"network_proxy_profile_id", profile_id.as_str().as_bytes());
                    encoder.field(
                        b"network_proxy_profile_revision",
                        revision.as_str().as_bytes(),
                    );
                }
            }
        }
    }
}

fn machine_name_field(encoder: &mut CanonicalHash, machine_name: Option<&MachineName>) {
    encoder.optional_field(
        b"machine_name",
        machine_name.map(|name| name.as_str().as_bytes()),
    );
}

fn command_fields(encoder: &mut CanonicalHash, command: &DirectCommand) {
    encoder.field(b"executable", command.executable().as_bytes());
    encoder.u64(
        b"argument_count",
        u64::try_from(command.arguments().len()).expect("validated argument count fits u64"),
    );
    for argument in command.arguments() {
        encoder.field(b"argument", argument.as_bytes());
    }
}

struct CanonicalHash(Sha256);

impl CanonicalHash {
    fn new(domain: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update(domain.len().to_be_bytes());
        hash.update(domain);
        Self(hash)
    }

    fn field(&mut self, name: &[u8], value: &[u8]) {
        self.0.update(name.len().to_be_bytes());
        self.0.update(name);
        self.0.update(value.len().to_be_bytes());
        self.0.update(value);
    }

    fn optional_field(&mut self, name: &[u8], value: Option<&[u8]>) {
        match value {
            Some(value) => {
                self.field(name, &[1]);
                self.field(name, value);
            }
            None => self.field(name, &[0]),
        }
    }

    fn u64(&mut self, name: &[u8], value: u64) {
        self.field(name, &value.to_be_bytes());
    }

    fn finish(self) -> RequestFingerprint {
        RequestFingerprint::from_digest(self.0.finalize().into())
    }
}

#[cfg(test)]
mod network_tests;
