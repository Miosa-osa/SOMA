# ADR 0024: Carry fresh per-Instance responder authority in launch-page schema 3

- Status: Accepted
- Date: 2026-08-30
- Extends: ADR 0017, ADR 0020, ADR 0021, and ADR 0023
- Supersedes in part: [ADR 0030, pre-launch snapshot capture point](0030-pre-launch-snapshot-capture-point.md), whose Generation-scoped responder-key consequence this decision replaces

## Context

ADR 0017 decided that Generation construction creates the responder keypair, provisions the private half into the trusted guest-agent artifact, and records the public half as trusted Generation metadata.
The same decision already states that a private key embedded in a publicly retrievable artifact cannot prove exclusive Generation possession.
The implementation took the weaker branch: `crates/soma-generation` accepted a responder private key as a fifth machine input, initramfs layout v2 stored it at `etc/soma/responder.key`, and the compiler published that initramfs as a content-addressed artifact.

Any party able to retrieve the initramfs therefore held the guest side of the static key.
Two Instances compiled from one Generation also shared exactly one responder identity, so the static key could not distinguish them.
The implementation audit of 2026-08-29 records this as Priority 0 finding P0.1.

ADR 0020 already requires a dedicated non-snapshot memory slot that carries fresh per-Launch secrets after Host ownership is established, and ADR 0023 raised that page to schema 2 with a digest and a reserved-zero tail.
That mechanism is the correct carrier for the responder secret, and it is the only carrier that satisfies the fresh-per-Instance requirement.

## Decision

The responder static secret becomes fresh per Instance and travels only in the launch page.

The launch page schema version becomes `3`.
Bytes 0 through 246 keep the exact schema 2 layout, so the domain, page schema, authentication profile, identities, launch nonce, Instance PSK, entropy seed, and network identity keep their offsets.

| Offset | Size | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 247 | Schema 2 prefix | Exactly the ADR 0023 layout with page schema `3` |
| 247 | 32 | Responder static secret | Fresh X25519 private scalar, rejected when all zero |
| 279 | 32 | Digest | BLAKE2s-256 over bytes 0 through 278 |
| 311 | 3785 | Reserved | All zero |

`HostLaunchMaterial::generate` now samples 160 operating-system random bytes instead of 128 and partitions them into the launch nonce, Instance PSK, entropy seed, and responder static secret.
An all-zero value in any of the four fields rejects the sample and retries the entire sample up to four times, exactly as ADR 0020 already required for the first three.
The host derives the matching X25519 public half through the same fixed suite and contributory resolver that ADR 0017 pins, and a sample whose derived public half is not contributory is retried.

`HostLaunchMaterial::responder_public_key` and `DeliveredHostLaunchMaterial::responder_public_key` return that public half.
It is the only half of the per-Instance guest authority that may enter a receipt, an evidence record, a log, or any other publicly retrievable object.

`DeliveredHostLaunchMaterial` retains the public half and `GuestSessionMaterial` retains the private half, so neither peer accepts a caller-supplied responder key.
`HostControl::connect` takes the delivered material and the transport, and `GuestControl::connect` takes the guest session material, the transport, and a deadline.
Substituting one Instance's responder identity for another's is therefore unrepresentable through the safe API rather than merely discouraged.

Initramfs layout version becomes `3`.
The `etc`, `etc/soma`, and `etc/soma/responder.key` entries are removed, the archive holds exactly two byte bodies and both are executables, and `verify_initramfs` rejects a layout v2 archive because its key entry is not in the v3 allowlist.
`MachineInputs` names four files rather than five, so the Generation compiler has no secret input at all.
`InitramfsBinding::layout_version` carries `3` in the `SOMAGEN` manifest and `require_profile` accepts only that value.

## Consequences for the earlier decisions

ADR 0017's statement that Generation construction creates the responder keypair and provisions the private key into the guest artifact is superseded.
The responder keypair is created by the Host once per Instance, after the winning worker receives the concrete Instance identity, from the same operating-system CSPRNG sample that produces the Instance PSK.
The host no longer obtains a pinned responder public key from the Generation manifest, because the Generation no longer has one; it holds the public half it just generated.
Everything else in ADR 0017 is unchanged: the fixed `Noise_NKpsk0_25519_ChaChaPoly_BLAKE2s` profile, the transcript binding, the contributory resolver, the record grammar, and the poisoning rules.

ADR 0020's sentence that the responder private identity remains immutable Generation material and is not carried in the launch page is superseded by this decision.
Every other launch-page rule stands and now covers 32 more secret bytes: single physical guest copy, snapshot exclusion, erasure after consumption, host observation of zeroes, memory-slot retirement, and Instance destruction on any ambiguous step.

ADR 0021 is unchanged in substance.
Its `HostControl::connect` and `GuestControl::connect` seams lose their explicit responder-key parameters because the launch material already carries the correct half.

ADR 0023 is extended rather than replaced.
Its schema 2 prefix, its `LaunchNetwork` validation rules, its digest discipline, and its reserved-zero rule are the schema 3 prefix, and its frozen vector is reissued at 311 bytes.

## Verification

The frozen launch-page vector pins the complete 311-byte schema 3 prefix including the responder secret and the recomputed digest.
Hostile page tests zero the responder field, flip its first and last bytes, flip the first and last digest bytes, flip the first reserved byte, and flip the final page byte; every case is rejected and the whole page is wiped.
The zero-sample retry matrix covers the responder field alongside the nonce, PSK, and entropy seed.

Two Instances compiled from one Generation are proven to hold different responder public keys, different launch pages, and different authenticated transcripts.
A party holding only public artifact bytes and the published responder public identity is proven unable to complete the handshake, and the host poisons its transport exactly once.
A compiled Generation is scanned object by object, including the kernel, initramfs, guest agent, EROFS root, overlay templates, and encoded manifest, and none of them contains any Instance's launch nonce, Instance PSK, entropy seed, or responder secret, nor the retired `etc/soma` path.
An archive carrying a responder key entry is rejected by the layout v3 verifier.

## Consequences

A reusable Generation now contains public identity only, so retrieving every artifact of a Generation grants no guest authentication authority.
Restoring a snapshot cannot reuse a previous Instance's responder identity, because the responder secret arrives with the launch page after restore rather than from immutable state.
A Generation compiled before this decision is not launchable by this code, which is the intended fail-closed behavior for a VMM and Generation pair that must change together.

This decision does not implement snapshot capture, certification, remote attestation, or production key erasure.
It does not change the fact that the launch page remains a bearer secret whose confidentiality depends on the Linux KVM adapter honoring ADR 0020's injection contract.
