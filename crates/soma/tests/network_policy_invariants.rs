use std::net::{IpAddr, Ipv4Addr};

use soma::{
    DnsPolicy, EgressPolicy, GuestAddressIntent, Ipv4AddressIntent, Ipv6AddressIntent,
    NetworkPolicy, NetworkProfileId, NetworkProfileSelector, PortPublication, ProfileRevision,
    ProxyPolicy, ProxyProfileSelector,
};

#[test]
fn safe_and_runtime_defaults_are_exact_and_distinct() {
    let isolated = NetworkPolicy::isolated();
    assert!(isolated.profile().is_disabled());
    assert!(isolated.guest_addresses().ipv4().is_disabled());
    assert!(isolated.guest_addresses().ipv6().is_disabled());
    assert!(isolated.proxy().is_disabled());
    assert_eq!(isolated.egress(), EgressPolicy::Denied);
    assert_eq!(isolated.dns(), &DnsPolicy::Denied);
    assert!(isolated.published_ports().is_empty());

    let runtime = NetworkPolicy::runtime_default();
    assert!(runtime.profile().is_operator_default());
    assert!(runtime.guest_addresses().ipv4().is_unspecified());
    assert!(runtime.guest_addresses().ipv6().is_unspecified());
    assert!(runtime.proxy().is_disabled());
    assert_eq!(runtime.egress(), EgressPolicy::Unspecified);
    assert_eq!(runtime.dns(), &DnsPolicy::Unspecified);
    assert!(runtime.published_ports().is_empty());
}

#[test]
fn runtime_default_is_all_or_nothing() {
    assert!(
        policy(
            named_network(),
            GuestAddressIntent::runtime_default(),
            ProxyPolicy::disabled(),
            EgressPolicy::Unspecified,
            DnsPolicy::Unspecified,
            Vec::new(),
        )
        .is_err()
    );
    assert!(
        policy(
            NetworkProfileSelector::operator_default(),
            allocated_v4(),
            ProxyPolicy::disabled(),
            EgressPolicy::Unspecified,
            DnsPolicy::Unspecified,
            Vec::new(),
        )
        .is_err()
    );
    assert!(
        policy(
            NetworkProfileSelector::operator_default(),
            allocated_v4(),
            ProxyPolicy::disabled(),
            EgressPolicy::PublicInternet,
            DnsPolicy::Unspecified,
            Vec::new(),
        )
        .is_err()
    );
}

#[test]
fn denied_egress_requires_denied_dns_and_disabled_proxy() {
    let addresses = GuestAddressIntent::disabled();
    assert!(
        explicit(
            addresses,
            ProxyPolicy::disabled(),
            EgressPolicy::Denied,
            DnsPolicy::System
        )
        .is_err()
    );
    assert!(
        explicit(
            allocated_v4(),
            ProxyPolicy::required(ProxyProfileSelector::operator_default()),
            EgressPolicy::Denied,
            DnsPolicy::Denied,
        )
        .is_err()
    );
}

#[test]
fn disabled_network_profile_rejects_nonisolated_intent() {
    assert!(
        policy(
            NetworkProfileSelector::disabled(),
            allocated_v4(),
            ProxyPolicy::disabled(),
            EgressPolicy::Denied,
            DnsPolicy::Denied,
            Vec::new(),
        )
        .is_err()
    );
}

#[test]
fn connected_egress_and_dns_require_an_enabled_address_family() {
    for egress in [EgressPolicy::PublicInternet, EgressPolicy::Unrestricted] {
        assert!(
            explicit(
                GuestAddressIntent::disabled(),
                ProxyPolicy::disabled(),
                egress,
                DnsPolicy::System,
            )
            .is_err()
        );
    }
}

#[test]
fn custom_dns_requires_the_matching_guest_address_families() {
    let ipv4_dns =
        DnsPolicy::custom(vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))]).expect("IPv4 DNS");
    let ipv6_dns = DnsPolicy::custom(vec![IpAddr::V6(
        "2606:4700:4700::1111".parse().expect("IPv6"),
    )])
    .expect("IPv6 DNS");

    assert!(
        explicit(
            allocated_v6(),
            ProxyPolicy::disabled(),
            EgressPolicy::PublicInternet,
            ipv4_dns,
        )
        .is_err()
    );
    assert!(
        explicit(
            allocated_v4(),
            ProxyPolicy::disabled(),
            EgressPolicy::PublicInternet,
            ipv6_dns,
        )
        .is_err()
    );
}

#[test]
fn required_proxy_is_limited_to_public_internet_egress() {
    let required = || ProxyPolicy::required(ProxyProfileSelector::operator_default());
    assert!(
        explicit(
            allocated_v4(),
            required(),
            EgressPolicy::PublicInternet,
            DnsPolicy::System,
        )
        .is_ok()
    );
    assert!(
        explicit(
            allocated_v4(),
            required(),
            EgressPolicy::Unrestricted,
            DnsPolicy::System,
        )
        .is_err()
    );
}

#[test]
fn denied_egress_allows_explicit_ingress_only_addressing() {
    let publication = PortPublication::loopback_tcp(8_080).expect("publication");
    let ingress_only = policy(
        named_network(),
        allocated_v4(),
        ProxyPolicy::disabled(),
        EgressPolicy::Denied,
        DnsPolicy::Denied,
        vec![publication],
    )
    .expect("ingress-only policy");

    assert_eq!(ingress_only.egress(), EgressPolicy::Denied);
    assert!(ingress_only.guest_addresses().ipv4().is_enabled());
    assert_eq!(ingress_only.published_ports().len(), 1);
    assert!(
        policy(
            NetworkProfileSelector::operator_default(),
            GuestAddressIntent::disabled(),
            ProxyPolicy::disabled(),
            EgressPolicy::Denied,
            DnsPolicy::Denied,
            vec![PortPublication::loopback_tcp(8_080).expect("publication")],
        )
        .is_err()
    );
}

#[test]
fn policy_serde_round_trip_revalidates_the_full_invariant_gate() {
    let valid = explicit(
        allocated_v4(),
        ProxyPolicy::disabled(),
        EgressPolicy::PublicInternet,
        DnsPolicy::System,
    )
    .expect("valid policy");
    let encoded = serde_json::to_string(&valid).expect("policy JSON");
    assert_eq!(
        serde_json::from_str::<NetworkPolicy>(&encoded).expect("policy round trip"),
        valid
    );

    let invalid = encoded.replace(r#""mode":"system""#, r#""mode":"unspecified""#);
    assert!(serde_json::from_str::<NetworkPolicy>(&invalid).is_err());
}

#[test]
fn legacy_constructor_uses_the_canonical_gate_and_explicit_ipv4_migration() {
    let connected = NetworkPolicy::new(EgressPolicy::PublicInternet, DnsPolicy::System, Vec::new())
        .expect("legacy connected policy");
    assert!(connected.guest_addresses().ipv4().is_enabled());
    assert!(connected.guest_addresses().ipv6().is_disabled());
    assert!(
        NetworkPolicy::new(
            EgressPolicy::Unrestricted,
            DnsPolicy::Unspecified,
            Vec::new(),
        )
        .is_err()
    );
    assert_eq!(
        NetworkPolicy::new(
            EgressPolicy::Unspecified,
            DnsPolicy::Unspecified,
            Vec::new(),
        )
        .expect("runtime default"),
        NetworkPolicy::runtime_default()
    );
}

fn explicit(
    addresses: GuestAddressIntent,
    proxy: ProxyPolicy,
    egress: EgressPolicy,
    dns: DnsPolicy,
) -> Result<NetworkPolicy, soma::ValidationError> {
    policy(
        NetworkProfileSelector::operator_default(),
        addresses,
        proxy,
        egress,
        dns,
        Vec::new(),
    )
}

fn policy(
    profile: NetworkProfileSelector,
    addresses: GuestAddressIntent,
    proxy: ProxyPolicy,
    egress: EgressPolicy,
    dns: DnsPolicy,
    publications: Vec<PortPublication>,
) -> Result<NetworkPolicy, soma::ValidationError> {
    NetworkPolicy::from_intent(profile, addresses, proxy, egress, dns, publications)
}

fn allocated_v4() -> GuestAddressIntent {
    addresses(
        Ipv4AddressIntent::allocated(),
        Ipv6AddressIntent::disabled(),
    )
}

fn allocated_v6() -> GuestAddressIntent {
    addresses(
        Ipv4AddressIntent::disabled(),
        Ipv6AddressIntent::allocated(),
    )
}

fn addresses(ipv4: Ipv4AddressIntent, ipv6: Ipv6AddressIntent) -> GuestAddressIntent {
    GuestAddressIntent::new(ipv4, ipv6).expect("address intent")
}

fn named_network() -> NetworkProfileSelector {
    NetworkProfileSelector::named(
        NetworkProfileId::parse("tenant-edge").expect("profile"),
        ProfileRevision::parse(format!("sha256:{}", "a".repeat(64))).expect("revision"),
    )
}
