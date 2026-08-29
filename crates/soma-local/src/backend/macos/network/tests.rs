use std::net::{IpAddr, Ipv4Addr};

use soma::{
    BackendFailureKind, DnsPolicy, EgressPolicy, GuestAddressIntent, Ipv4AddressIntent,
    Ipv6AddressIntent, NetworkPolicy, NetworkProfileId, NetworkProfileSelector, ProfileRevision,
    ProxyPolicy, ProxyProfileSelector,
};

use super::prepare;

#[test]
fn secure_and_runtime_defaults_are_supported() {
    assert!(prepare(&NetworkPolicy::isolated()).is_ok());
    assert!(prepare(&NetworkPolicy::runtime_default()).is_ok());
}

#[test]
fn named_network_profile_fails_closed() {
    let profile = NetworkProfileSelector::named(
        NetworkProfileId::parse("restricted-egress").expect("profile ID"),
        ProfileRevision::parse(format!("sha256:{}", "a".repeat(64))).expect("revision"),
    );
    let policy = explicit_policy(
        profile,
        ProxyPolicy::disabled(),
        Ipv4Addr::new(192, 0, 2, 10),
    );

    assert_eq!(
        prepare(&policy).err(),
        Some(BackendFailureKind::Unsupported)
    );
}

#[test]
fn exact_guest_address_fails_closed() {
    let policy = explicit_policy(
        NetworkProfileSelector::operator_default(),
        ProxyPolicy::disabled(),
        Ipv4Addr::new(192, 0, 2, 10),
    );

    assert_eq!(
        prepare(&policy).err(),
        Some(BackendFailureKind::Unsupported)
    );
}

#[test]
fn required_proxy_fails_closed() {
    let addresses = GuestAddressIntent::new(
        Ipv4AddressIntent::allocated(),
        Ipv6AddressIntent::disabled(),
    )
    .expect("address intent");
    let policy = NetworkPolicy::from_intent(
        NetworkProfileSelector::operator_default(),
        addresses,
        ProxyPolicy::required(ProxyProfileSelector::operator_default()),
        EgressPolicy::PublicInternet,
        DnsPolicy::System,
        Vec::new(),
    )
    .expect("proxy policy");

    assert_eq!(
        prepare(&policy).err(),
        Some(BackendFailureKind::Unsupported)
    );
}

fn explicit_policy(
    profile: NetworkProfileSelector,
    proxy: ProxyPolicy,
    address: Ipv4Addr,
) -> NetworkPolicy {
    let addresses = GuestAddressIntent::new(
        Ipv4AddressIntent::requested(address).expect("requested address"),
        Ipv6AddressIntent::disabled(),
    )
    .expect("address intent");
    NetworkPolicy::from_intent(
        profile,
        addresses,
        proxy,
        EgressPolicy::Unrestricted,
        DnsPolicy::custom(vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))]).expect("DNS"),
        Vec::new(),
    )
    .expect("network policy")
}
