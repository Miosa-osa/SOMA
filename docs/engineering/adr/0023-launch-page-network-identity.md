# ADR 0023: Carry fresh network and transport identity in launch-page schema 2

- Status: Accepted
- Date: 2026-08-29
- Extends: ADR 0020 and ADR 0021
- Extended by: [ADR 0024, per-Instance guest responder authority](0024-per-instance-guest-responder-authority.md) and ADR 0028

## Context

The Linux guest integration research requires the launch page to deliver the assigned vsock CID, the network generation, and a wall-clock sample in addition to the schema 1 secrets.
The static guest agent must install a fresh MAC, IPv4 address, prefix, gateway, and resolver before it raises the link, and it must verify that the vsock device reports the CID the VMM assigned.
Schema 1 of ADR 0020 reserves the page tail as zero and rejects any nonzero reserved byte, so these fields cannot be added without a new page schema.

## Decision

The launch page schema version becomes `2`.
The first 212 bytes keep the exact schema 1 layout.
Bytes 212 through 246 carry the non-secret network identity, bytes 247 through 278 carry a BLAKE2s digest over bytes 0 through 246, and the remaining bytes stay reserved zero.

| Offset | Size | Field | Encoding |
| ---: | ---: | --- | --- |
| 212 | 4 | Vsock CID | Unsigned 32-bit big-endian, above 2 and below the wildcard value |
| 216 | 4 | Network generation | Nonzero unsigned 32-bit big-endian |
| 220 | 6 | MAC | Nonzero unicast address |
| 226 | 4 | IPv4 address | Usable unicast address |
| 230 | 1 | Prefix length | 1 through 30 |
| 231 | 4 | Gateway | Usable unicast address inside the prefix and different from the address |
| 235 | 4 | Resolver | Usable unicast address |
| 239 | 8 | Time sample | Nonzero Unix nanoseconds, unsigned 64-bit big-endian |
| 247 | 32 | Digest | BLAKE2s-256 over bytes 0 through 246 |
| 279 | 3817 | Reserved | All zero |

`LaunchNetwork` is a public non-secret value validated by the same rules on both peers.
`HostLaunchMaterial::generate` requires it, `GuestLaunchMaterial::network` returns it, and the digest is compared in constant time before any identity is accepted.
The digest uses the BLAKE2s implementation already resolved by the fixed Noise suite, so no dependency is added.

The crate also fixes two machine-contract constants: `LAUNCH_PAGE_GUEST_ADDRESS` is `0xd0100000`, one page above the five virtio-mmio pages and outside the 3 GiB RAM ceiling, and `CONTROL_VSOCK_PORT` is `0x534f4d41` on host CID 2.
`SessionBinding` now exposes its non-secret Generation, Instance, and operation identities so the guest agent can derive hostname and machine identity.

## Verification

The frozen vector test pins the complete 279-byte prefix including the digest.
Hostile page tests additionally zero the network block and flip the digest, the last digest byte, the first reserved byte, and the final page byte.
`LaunchNetwork` tests reject every invalid field class and round-trip the encoding.
Every existing control-owner and launch-page test passes with the new schema.

## Consequences

[ADR 0024, per-Instance guest responder authority](0024-per-instance-guest-responder-authority.md) raises the page to schema 3 by appending a fresh per-Instance responder static secret at byte 247 and moving the digest to byte 279.
The first 247 bytes of a schema 3 page are exactly the layout decided here, with `3` in the page-schema field, so every rule and every `LaunchNetwork` validation above still applies.

A schema 1 page is rejected by a schema 2 guest and the reverse, which is the intended fail-closed behavior for a Generation and VMM pair that must change together.
The page remains a bearer secret; the added fields are non-secret but their delivery still depends on the confidential non-snapshot slot required by ADR 0020.
This decision does not prove that the VMM installs the slot, that the guest kernel maps it, or that the network identity is safe to activate; those remain obligations of the Linux KVM adapter and the guest-agent evidence gates.
