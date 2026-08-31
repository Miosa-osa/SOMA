//! Admission is the point where an unsupported dimension is refused, before any kernel
//! object exists.

use soma::{
    GuestAddressIntent, HostBind, HostPort, Ipv4AddressIntent, Ipv6AddressIntent,
    NetworkProfileSelector, ProxyPolicy, TransportProtocol,
};

use super::*;
use crate::profile::tests::test_profile;

fn policy(egress: EgressPolicy, dns: DnsPolicy) -> NetworkPolicy {
    NetworkPolicy::new(egress, dns, Vec::new()).expect("portable policy")
}

#[test]
fn admission_maps_each_supported_class() {
    let profile = test_profile();
    let denied = NetworkIntent::admit(&NetworkPolicy::isolated(), &profile).expect("denied");
    assert_eq!(denied.egress(), EgressClass::Denied);
    assert!(!denied.dns_allowed());
    assert_eq!(denied, NetworkIntent::denied(&profile));
    let system = NetworkIntent::admit(
        &policy(EgressPolicy::PublicInternet, DnsPolicy::System),
        &profile,
    )
    .expect("system dns");
    assert_eq!(system.resolvers(), profile.resolvers());
    let custom = DnsPolicy::custom(vec![IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))]).expect("dns");
    let unrestricted = NetworkIntent::admit(&policy(EgressPolicy::Unrestricted, custom), &profile)
        .expect("custom dns");
    assert_eq!(unrestricted.resolvers(), &[Ipv4Addr::new(9, 9, 9, 9)]);
    assert_ne!(unrestricted.digest(), system.digest());
    assert_eq!(unrestricted.digest(), unrestricted.clone().digest());
}

#[test]
fn admission_fails_closed_on_every_unsupported_dimension() {
    let profile = test_profile();
    let cases = [
        (
            NetworkPolicy::runtime_default(),
            IntentRejection::EgressUnspecified,
        ),
        (
            policy(
                EgressPolicy::PublicInternet,
                DnsPolicy::custom(vec![IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))])
                    .expect("dns"),
            ),
            IntentRejection::ResolverProtected,
        ),
        (
            NetworkPolicy::from_intent(
                NetworkProfileSelector::operator_default(),
                GuestAddressIntent::new(
                    Ipv4AddressIntent::allocated(),
                    Ipv6AddressIntent::allocated(),
                )
                .expect("addresses"),
                ProxyPolicy::disabled(),
                EgressPolicy::PublicInternet,
                DnsPolicy::custom(vec!["2606:4700::1111".parse().expect("literal")]).expect("dns"),
                Vec::new(),
            )
            .expect("portable policy"),
            IntentRejection::Ipv6Unimplemented,
        ),
    ];
    for (policy, expected) in cases {
        assert_eq!(
            NetworkIntent::admit(&policy, &profile).expect_err("rejected"),
            Error::InvalidIntent(expected)
        );
    }
}

#[test]
fn publications_participate_in_the_digest() {
    let profile = test_profile();
    let publication = PortPublication::new(
        HostBind::loopback_v4(),
        HostPort::from_u16(8080),
        80,
        TransportProtocol::Tcp,
    )
    .expect("publication");
    let with = NetworkPolicy::new(
        EgressPolicy::PublicInternet,
        DnsPolicy::Denied,
        vec![publication],
    )
    .expect("policy");
    let without = policy(EgressPolicy::PublicInternet, DnsPolicy::Denied);
    let with = NetworkIntent::admit(&with, &profile).expect("admitted");
    let without = NetworkIntent::admit(&without, &profile).expect("admitted");
    assert_eq!(with.publications().len(), 1);
    assert_ne!(with.digest(), without.digest());
}

#[test]
fn a_publication_on_an_ipv6_host_bind_is_refused_at_admission() {
    let profile = test_profile();
    let publication = PortPublication::new(
        HostBind::ipv6(std::net::Ipv6Addr::LOCALHOST, true).expect("bind"),
        HostPort::from_u16(8080),
        80,
        TransportProtocol::Tcp,
    )
    .expect("publication");
    let policy = NetworkPolicy::new(
        EgressPolicy::PublicInternet,
        DnsPolicy::Denied,
        vec![publication],
    )
    .expect("policy");
    assert_eq!(
        NetworkIntent::admit(&policy, &profile).expect_err("rejected"),
        Error::InvalidIntent(IntentRejection::PublicationFamily),
        "an IPv6 bind has no destination while the guest lease is IPv4 only"
    );
}
