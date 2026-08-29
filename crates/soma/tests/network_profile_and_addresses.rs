use std::net::{Ipv4Addr, Ipv6Addr};

use soma::{
    GuestAddressIntent, Ipv4AddressIntent, Ipv6AddressIntent, NetworkProfileId,
    NetworkProfileSelector, ProfileRevision, ProxyProfileId, ProxyProfileSelector,
};

#[test]
fn profile_identifiers_accept_only_the_portable_grammar() {
    for valid in ["a", "edge-1", &"a".repeat(63)] {
        assert_eq!(
            NetworkProfileId::parse(valid)
                .expect("valid network profile")
                .as_str(),
            valid
        );
        assert_eq!(
            ProxyProfileId::parse(valid)
                .expect("valid proxy profile")
                .as_str(),
            valid
        );
    }

    for invalid in [
        "",
        "Edge",
        "edge_profile",
        "-edge",
        "edge-",
        "edge.proxy",
        "é",
        &"a".repeat(64),
    ] {
        assert!(
            NetworkProfileId::parse(invalid).is_err(),
            "accepted {invalid:?}"
        );
        assert!(
            ProxyProfileId::parse(invalid).is_err(),
            "accepted {invalid:?}"
        );
        assert!(serde_json::from_str::<NetworkProfileId>(&format!(r#""{invalid}""#)).is_err());
    }
}

#[test]
fn profile_revisions_require_canonical_lowercase_sha256() {
    let canonical = format!("sha256:{}", "a1".repeat(32));
    assert_eq!(
        ProfileRevision::parse(&canonical)
            .expect("canonical revision")
            .as_str(),
        canonical
    );

    for invalid in [
        "a1".repeat(32),
        format!("sha256:{}", "A1".repeat(32)),
        format!("sha256:{}", "a".repeat(63)),
        format!("sha512:{}", "a".repeat(64)),
    ] {
        assert!(ProfileRevision::parse(&invalid).is_err());
        assert!(serde_json::from_str::<ProfileRevision>(&format!(r#""{invalid}""#)).is_err());
    }
}

#[test]
fn named_selectors_round_trip_without_credentials_or_endpoints() {
    let revision = revision('a');
    let network = NetworkProfileSelector::named(
        NetworkProfileId::parse("public-egress").expect("profile"),
        revision.clone(),
    );
    let proxy = ProxyProfileSelector::named(
        ProxyProfileId::parse("audit-proxy").expect("profile"),
        revision,
    );

    let network_json = serde_json::to_string(&network).expect("network selector JSON");
    let proxy_json = serde_json::to_string(&proxy).expect("proxy selector JSON");
    assert_eq!(
        serde_json::from_str::<NetworkProfileSelector>(&network_json).expect("network round trip"),
        network
    );
    assert_eq!(
        serde_json::from_str::<ProxyProfileSelector>(&proxy_json).expect("proxy round trip"),
        proxy
    );
    for forbidden in ["token", "password", "endpoint", "credential"] {
        assert!(!network_json.contains(forbidden));
        assert!(!proxy_json.contains(forbidden));
    }
}

#[test]
fn requested_addresses_reject_nonportable_special_ranges() {
    for invalid in [
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::LOCALHOST,
        Ipv4Addr::new(127, 22, 1, 9),
        Ipv4Addr::new(224, 0, 0, 1),
        Ipv4Addr::BROADCAST,
    ] {
        assert!(Ipv4AddressIntent::requested(invalid).is_err());
    }
    for invalid in [
        Ipv6Addr::UNSPECIFIED,
        Ipv6Addr::LOCALHOST,
        "ff02::1".parse().expect("multicast"),
        "fe80::1".parse().expect("link local"),
    ] {
        assert!(Ipv6AddressIntent::requested(invalid).is_err());
    }

    let ipv4 = Ipv4AddressIntent::requested(Ipv4Addr::new(10, 24, 0, 7)).expect("exact IPv4");
    let ipv6 =
        Ipv6AddressIntent::requested("2001:db8::7".parse().expect("IPv6")).expect("exact IPv6");
    assert_eq!(ipv4.requested_address(), Some(Ipv4Addr::new(10, 24, 0, 7)));
    assert_eq!(
        ipv6.requested_address(),
        Some("2001:db8::7".parse().expect("IPv6"))
    );
}

#[test]
fn address_deserialization_revalidates_requested_and_family_pair_intent() {
    let loopback = r#"{"mode":"requested","address":"127.0.0.1"}"#;
    let link_local = r#"{"mode":"requested","address":"fe80::1"}"#;
    let partial = r#"{"ipv4":{"mode":"unspecified"},"ipv6":{"mode":"disabled"}}"#;

    assert!(serde_json::from_str::<Ipv4AddressIntent>(loopback).is_err());
    assert!(serde_json::from_str::<Ipv6AddressIntent>(link_local).is_err());
    assert!(serde_json::from_str::<GuestAddressIntent>(partial).is_err());
    assert!(
        GuestAddressIntent::new(
            Ipv4AddressIntent::unspecified(),
            Ipv6AddressIntent::disabled()
        )
        .is_err()
    );
}

fn revision(digit: char) -> ProfileRevision {
    ProfileRevision::parse(format!("sha256:{}", digit.to_string().repeat(64))).expect("revision")
}
