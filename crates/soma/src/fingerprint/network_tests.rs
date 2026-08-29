use std::net::{Ipv4Addr, Ipv6Addr};

use crate::{
    Capabilities, DirectCommand, DnsPolicy, EgressPolicy, ExecutionLimits, GenerationId,
    GuestAddressIntent, InstanceId, Ipv4AddressIntent, Ipv6AddressIntent, MachineShape,
    NetworkPolicy, NetworkProfileId, NetworkProfileSelector, OciDigest, OciPlatform,
    ProfileRevision, ProxyPolicy, ProxyProfileId, ProxyProfileSelector, RequestFingerprint,
    WorkloadIdentity,
};

use super::{launch, run};

#[test]
fn run_and_launch_hash_every_profile_address_and_proxy_dimension() {
    let baseline = explicit(
        NetworkProfileSelector::operator_default(),
        allocated_v4(),
        ProxyPolicy::disabled(),
    );
    let named_a = explicit(
        named_network("edge-a", 'a'),
        allocated_v4(),
        ProxyPolicy::disabled(),
    );
    let named_b = explicit(
        named_network("edge-b", 'a'),
        allocated_v4(),
        ProxyPolicy::disabled(),
    );
    let named_revision = explicit(
        named_network("edge-a", 'b'),
        allocated_v4(),
        ProxyPolicy::disabled(),
    );
    let explicit_isolated = NetworkPolicy::from_intent(
        NetworkProfileSelector::operator_default(),
        GuestAddressIntent::disabled(),
        ProxyPolicy::disabled(),
        EgressPolicy::Denied,
        DnsPolicy::Denied,
        Vec::new(),
    )
    .expect("explicit isolated policy");
    assert_changed(&NetworkPolicy::isolated(), &explicit_isolated);
    assert_changed(&baseline, &named_a);
    assert_changed(&named_a, &named_b);
    assert_changed(&named_a, &named_revision);

    let requested_v4_a = explicit(
        NetworkProfileSelector::operator_default(),
        addresses(
            Ipv4AddressIntent::requested(Ipv4Addr::new(10, 0, 0, 7)).expect("IPv4"),
            Ipv6AddressIntent::disabled(),
        ),
        ProxyPolicy::disabled(),
    );
    let requested_v4_b = explicit(
        NetworkProfileSelector::operator_default(),
        addresses(
            Ipv4AddressIntent::requested(Ipv4Addr::new(10, 0, 0, 8)).expect("IPv4"),
            Ipv6AddressIntent::disabled(),
        ),
        ProxyPolicy::disabled(),
    );
    assert_changed(&baseline, &requested_v4_a);
    assert_changed(&requested_v4_a, &requested_v4_b);

    let allocated_v6 = explicit(
        NetworkProfileSelector::operator_default(),
        addresses(
            Ipv4AddressIntent::allocated(),
            Ipv6AddressIntent::allocated(),
        ),
        ProxyPolicy::disabled(),
    );
    let requested_v6 = explicit(
        NetworkProfileSelector::operator_default(),
        addresses(
            Ipv4AddressIntent::allocated(),
            Ipv6AddressIntent::requested(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 7))
                .expect("IPv6"),
        ),
        ProxyPolicy::disabled(),
    );
    assert_changed(&baseline, &allocated_v6);
    assert_changed(&allocated_v6, &requested_v6);

    let proxy_default = explicit(
        NetworkProfileSelector::operator_default(),
        allocated_v4(),
        ProxyPolicy::required(ProxyProfileSelector::operator_default()),
    );
    let proxy_a = explicit(
        NetworkProfileSelector::operator_default(),
        allocated_v4(),
        ProxyPolicy::required(named_proxy("proxy-a", 'a')),
    );
    let proxy_b = explicit(
        NetworkProfileSelector::operator_default(),
        allocated_v4(),
        ProxyPolicy::required(named_proxy("proxy-b", 'a')),
    );
    let proxy_revision = explicit(
        NetworkProfileSelector::operator_default(),
        allocated_v4(),
        ProxyPolicy::required(named_proxy("proxy-a", 'b')),
    );
    assert_changed(&baseline, &proxy_default);
    assert_changed(&proxy_default, &proxy_a);
    assert_changed(&proxy_a, &proxy_b);
    assert_changed(&proxy_a, &proxy_revision);
}

fn assert_changed(left: &NetworkPolicy, right: &NetworkPolicy) {
    let (left_run, left_launch) = fingerprints(left);
    let (right_run, right_launch) = fingerprints(right);
    assert_ne!(left_run, right_run, "run fingerprint omitted a dimension");
    assert_ne!(
        left_launch, right_launch,
        "launch fingerprint omitted a dimension"
    );
}

fn fingerprints(policy: &NetworkPolicy) -> (RequestFingerprint, RequestFingerprint) {
    let workload = WorkloadIdentity::new(
        OciDigest::parse(format!("sha256:{}", "a".repeat(64))).expect("digest"),
        OciPlatform::linux_amd64(),
        Some(GenerationId::new(format!("sha256:{}", "b".repeat(64))).expect("generation")),
    );
    let instance = InstanceId::new("22222222222222222222222222222222").expect("instance");
    let shape = MachineShape::new(1, 1_024, 10_240)
        .expect("shape")
        .with_capabilities(Capabilities::isolated().with_network_policy(policy.clone()));
    let command = DirectCommand::new("/bin/true", std::iter::empty::<String>()).expect("command");
    let limits = ExecutionLimits::new(30_000, 1_024).expect("limits");
    (
        run(&workload, &instance, None, &shape, &command, &limits),
        launch(&workload, &instance, None, &shape),
    )
}

fn explicit(
    profile: NetworkProfileSelector,
    addresses: GuestAddressIntent,
    proxy: ProxyPolicy,
) -> NetworkPolicy {
    NetworkPolicy::from_intent(
        profile,
        addresses,
        proxy,
        EgressPolicy::PublicInternet,
        DnsPolicy::System,
        Vec::new(),
    )
    .expect("valid explicit policy")
}

fn addresses(ipv4: Ipv4AddressIntent, ipv6: Ipv6AddressIntent) -> GuestAddressIntent {
    GuestAddressIntent::new(ipv4, ipv6).expect("valid address intent")
}

fn allocated_v4() -> GuestAddressIntent {
    addresses(
        Ipv4AddressIntent::allocated(),
        Ipv6AddressIntent::disabled(),
    )
}

fn named_network(id: &str, revision_digit: char) -> NetworkProfileSelector {
    NetworkProfileSelector::named(
        NetworkProfileId::parse(id).expect("network profile"),
        revision(revision_digit),
    )
}

fn named_proxy(id: &str, revision_digit: char) -> ProxyProfileSelector {
    ProxyProfileSelector::named(
        ProxyProfileId::parse(id).expect("proxy profile"),
        revision(revision_digit),
    )
}

fn revision(digit: char) -> ProfileRevision {
    ProfileRevision::parse(format!("sha256:{}", digit.to_string().repeat(64))).expect("revision")
}
