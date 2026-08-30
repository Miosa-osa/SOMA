//! Field-class rejection tests for the declared IPv4 profile.

use super::*;

type Fields = (u32, u32, [u8; 6], [u8; 4], u8, [u8; 4], [u8; 4], u64);

const MAC: [u8; 6] = [0x02, 0, 0, 0, 0, 1];

fn valid() -> LaunchNetwork {
    LaunchNetwork::new(
        3,
        1,
        MAC,
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        1,
    )
    .expect("valid network")
}

/// Builds one candidate that differs from the valid fixture only in the named IPv4 fields.
fn ipv4(address: [u8; 4], prefix: u8, gateway: [u8; 4], resolver: [u8; 4]) -> Fields {
    (3, 1, MAC, address, prefix, gateway, resolver, 1)
}

#[track_caller]
fn rejects(case: Fields) {
    let (cid, generation, mac, address, prefix, gateway, resolver, time) = case;
    assert_eq!(
        LaunchNetwork::new(
            cid, generation, mac, address, prefix, gateway, resolver, time
        )
        .expect_err("invalid network"),
        Error::InvalidLaunchNetwork
    );
}

#[track_caller]
fn accepts(case: Fields) {
    let (cid, generation, mac, address, prefix, gateway, resolver, time) = case;
    LaunchNetwork::new(
        cid, generation, mac, address, prefix, gateway, resolver, time,
    )
    .expect("valid network");
}

#[test]
fn netmask_follows_prefix_length() {
    assert_eq!(valid().netmask(), [255, 255, 255, 0]);
    assert_eq!(prefix_mask(30), 0xFFFF_FFFC);
    assert_eq!(prefix_mask(1), 0x8000_0000);
}

#[test]
fn round_trips_through_the_fixed_encoding() {
    let mut encoded = [0; ENCODED_SIZE];
    valid().encode(&mut encoded);

    assert_eq!(LaunchNetwork::decode(&encoded).expect("decodes"), valid());
    assert_eq!(
        LaunchNetwork::decode(&encoded[..ENCODED_SIZE - 1]).expect_err("short input"),
        Error::LaunchPageRejected
    );
}

#[test]
fn rejects_every_invalid_field_class() {
    let base = valid();
    let mac = base.mac;
    let address = base.address;
    let gateway = base.gateway;
    let resolver = base.resolver;
    let cases: [Fields; 12] = [
        (2, 1, mac, address, 24, gateway, resolver, 1),
        (u32::MAX, 1, mac, address, 24, gateway, resolver, 1),
        (3, 0, mac, address, 24, gateway, resolver, 1),
        (3, 1, [1, 0, 0, 0, 0, 1], address, 24, gateway, resolver, 1),
        (3, 1, [0; 6], address, 24, gateway, resolver, 1),
        (3, 1, mac, [127, 0, 0, 1], 24, gateway, resolver, 1),
        (3, 1, mac, address, 0, gateway, resolver, 1),
        (3, 1, mac, address, 31, gateway, resolver, 1),
        (3, 1, mac, address, 24, [10, 0, 1, 1], resolver, 1),
        (3, 1, mac, address, 24, address, resolver, 1),
        (3, 1, mac, address, 24, gateway, [0; 4], 1),
        (3, 1, mac, address, 24, gateway, resolver, 0),
    ];
    for case in cases {
        rejects(case);
    }
}

#[test]
fn a_host_route_and_a_point_to_point_link_are_outside_the_declared_profile() {
    assert_eq!(MIN_PREFIX_LENGTH, 1);
    assert_eq!(MAX_PREFIX_LENGTH, 30);

    // /32 leaves no gateway inside the prefix, /31 leaves no broadcast address, and the
    // profile requires both, so neither is accepted with any gateway choice.
    rejects(ipv4([10, 0, 0, 2], 32, [10, 0, 0, 1], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 32, [10, 0, 0, 2], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 31, [10, 0, 0, 3], [10, 0, 0, 3]));
    rejects(ipv4([10, 0, 0, 3], 31, [10, 0, 0, 2], [10, 0, 0, 2]));
    // The narrowest accepted subnet still has exactly two usable hosts.
    accepts(ipv4([10, 0, 0, 2], 30, [10, 0, 0, 1], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 33, [10, 0, 0, 1], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 255, [10, 0, 0, 1], [10, 0, 0, 1]));
}

#[test]
fn the_subnet_network_address_is_never_a_guest_gateway_or_in_prefix_resolver() {
    rejects(ipv4([10, 0, 0, 0], 24, [10, 0, 0, 1], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 0], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 1], [10, 0, 0, 0]));
    // The same address on a /30 whose network address is 10.0.0.0.
    rejects(ipv4([10, 0, 0, 0], 30, [10, 0, 0, 1], [10, 0, 0, 1]));
    // A /16 moves the network address, so 10.0.1.0 is a network address on a /24 and an
    // ordinary host on a /16.
    rejects(ipv4([10, 0, 1, 0], 24, [10, 0, 1, 1], [10, 0, 1, 1]));
    accepts(ipv4([10, 0, 1, 0], 16, [10, 0, 0, 1], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 0], 16, [10, 0, 0, 1], [10, 0, 0, 1]));
}

#[test]
fn the_directed_broadcast_address_is_never_a_guest_gateway_or_in_prefix_resolver() {
    rejects(ipv4([10, 0, 0, 255], 24, [10, 0, 0, 1], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 255], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 1], [10, 0, 0, 255]));
    rejects(ipv4([10, 0, 0, 3], 30, [10, 0, 0, 1], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 255, 255], 16, [10, 0, 0, 1], [10, 0, 0, 1]));
    // 10.0.0.255 is an ordinary host inside a /16 rather than its broadcast address.
    accepts(ipv4([10, 0, 0, 255], 16, [10, 0, 0, 1], [10, 0, 0, 1]));
}

#[test]
fn a_resolver_outside_the_prefix_is_allowed_but_still_must_be_usable_unicast() {
    accepts(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 1], [9, 9, 9, 9]));
    accepts(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 1], [1, 1, 1, 1]));
    rejects(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 1], [127, 0, 0, 53]));
    rejects(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 1], [224, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 1], [255, 255, 255, 255]));
    // Outside the guest prefix, an address ending in .0 or .255 is an ordinary resolver.
    accepts(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 1], [10, 0, 1, 0]));
}

#[test]
fn a_gateway_equal_to_the_guest_address_is_rejected_on_every_prefix() {
    for prefix in [8, 16, 24, 29, 30] {
        rejects(ipv4([10, 0, 0, 2], prefix, [10, 0, 0, 2], [10, 0, 0, 1]));
    }
    rejects(ipv4([10, 0, 0, 1], 24, [10, 0, 0, 1], [10, 0, 0, 1]));
}

#[test]
fn a_gateway_outside_the_prefix_is_rejected() {
    rejects(ipv4([10, 0, 0, 2], 24, [10, 0, 1, 1], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 30, [10, 0, 0, 5], [10, 0, 0, 1]));
    rejects(ipv4([10, 0, 0, 2], 24, [192, 168, 0, 1], [10, 0, 0, 1]));
}

#[test]
fn the_unusable_address_classes_are_rejected_in_every_position() {
    let unusable = [
        [0, 0, 0, 0],
        [0, 1, 2, 3],
        [127, 0, 0, 1],
        [127, 255, 255, 254],
        [169, 254, 0, 1],
        [169, 254, 255, 254],
        [224, 0, 0, 1],
        [239, 255, 255, 250],
        [240, 0, 0, 1],
        [255, 255, 255, 255],
    ];
    for address in unusable {
        assert!(!usable_unicast(address), "{address:?} was called usable");
        rejects(ipv4(address, 24, [10, 0, 0, 1], [10, 0, 0, 1]));
        rejects(ipv4([10, 0, 0, 2], 24, address, [10, 0, 0, 1]));
        rejects(ipv4([10, 0, 0, 2], 24, [10, 0, 0, 1], address));
    }
    // The addresses that border the rejected ranges stay usable.
    for address in [
        [1, 0, 0, 1],
        [126, 0, 0, 1],
        [128, 0, 0, 1],
        [223, 255, 255, 254],
    ] {
        assert!(usable_unicast(address), "{address:?} was called unusable");
    }
    assert!(usable_unicast([169, 253, 0, 1]));
    assert!(usable_unicast([169, 255, 0, 1]));
    assert!(usable_unicast([170, 254, 0, 1]));
}

#[test]
fn the_subnet_reserves_exactly_its_network_and_broadcast_addresses() {
    let subnet = Subnet {
        network: u32::from_be_bytes([10, 0, 0, 0]),
        broadcast: u32::from_be_bytes([10, 0, 0, 255]),
    };

    assert!(!subnet.usable_host(subnet.network));
    assert!(!subnet.usable_host(subnet.broadcast));
    assert!(subnet.usable_host(u32::from_be_bytes([10, 0, 0, 1])));
    assert!(subnet.usable_host(u32::from_be_bytes([10, 0, 0, 254])));
}
