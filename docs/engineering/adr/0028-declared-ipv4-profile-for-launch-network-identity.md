# ADR 0028: Reject every unusable address in the declared launch IPv4 profile

- Status: Accepted
- Date: 2026-08-30
- Extends: ADR 0023

## Context

`LaunchNetwork` validates the non-secret network identity that ADR 0023 added to the launch page.
It checked common unicast properties, the prefix range, membership of the gateway in the prefix, and inequality of the guest address and the gateway.

It did not reject two addresses that exist in every subnet and can never name a host: the subnet network address and the directed broadcast address.
A launch page carrying `10.0.0.0/24` as the guest address, or `10.0.0.255/24` as the gateway, was accepted by both peers, and the guest agent would have installed it and raised the link.
Link-local `169.254.0.0/16` was also accepted, which is the address block a host uses precisely when it has no configured identity.

ADR 0023 states the field rules as "usable unicast address" without saying what usable excludes, so the two implementations of the same rule had nothing to agree on.

The implementation audit of 2026-08-29 records this as Priority 1 finding P1.3.

## Decision

The IPv4 profile carried by a launch page is declared, and `LaunchNetwork::validate` enforces exactly it.

A prefix length is 1 through 30.
That range is unchanged, and it is now a decision rather than a constant: the profile requires a distinct gateway and a directed broadcast address inside the subnet, so a `/31` RFC 3021 point-to-point link and a `/32` host route are deliberately excluded.
There is no point-to-point exception.
Admitting one is a launch-page schema decision, because the network and broadcast rejections below would otherwise have nothing to reject.

A usable unicast address is not the unspecified address, not inside `0.0.0.0/8`, not loopback, not link-local `169.254.0.0/16`, not `224.0.0.0` or above, which covers multicast and the reserved space, and not the limited broadcast address.
The guest address, the gateway, and the resolver must each be usable unicast.

The guest address and the gateway must both be assignable hosts of the subnet the guest address and prefix describe: neither may be the subnet network address, neither may be its directed broadcast address, the gateway must lie inside the prefix, and the two must differ.

The resolver may lie outside the subnet, because a resolver reached through the gateway is a normal deployment.
When it lies inside the subnet it is held to the same host rules as the guest and the gateway.

The wire encoding is unchanged, so the frozen launch-page vector is unchanged.
This decision narrows the accepted set only.

## Verification

`crates/soma-guest/src/launch_page/network/tests.rs` covers `/32`, `/31`, `/33`, and `/255` against the accepted `/30`; the subnet network address and the directed broadcast address in the guest, gateway, and in-prefix resolver positions on `/16`, `/24`, and `/30`; the same octets accepted as an ordinary host under a shorter prefix; a gateway equal to the guest address on five prefixes; a gateway outside the prefix; a resolver inside and outside the prefix; and every unusable class, with the addresses bordering each rejected range proven still usable.

The `soma-netd` allocator hands out `/30` leases whose host and guest addresses are the two assignable hosts, so the profile accepts every lease it produces.
The frozen schema 2 launch-page vector and every existing launch-page, control-owner, and Generation secret test pass unchanged.

## Consequences

A Host that computes an unusable identity now fails to build launch material instead of delivering an identity the guest would install.
Both peers validate the same declared rules, so a page that one accepts the other cannot reject.

This decision does not add IPv6, multiple resolvers, static routes, or a point-to-point profile.
It does not prove that the addresses a Host allocates are reachable, only that they are not addresses that can never name a host.
