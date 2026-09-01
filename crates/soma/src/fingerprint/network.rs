//! The network half of a run request fingerprint: the policy, the profile it selects, and the
//! proxy it routes through.
//!
//! Every field a request can vary has to reach the hash, or two requests that differ would
//! fingerprint alike and one would be served the other's result. Network policy is where that
//! risk lives, because it is the part that keeps growing, so its encoding is kept in one file
//! with its proofs in `network_tests.rs` beside it.

use super::CanonicalHash;

pub(super) fn network_fields(encoder: &mut CanonicalHash, policy: &crate::NetworkPolicy) {
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
