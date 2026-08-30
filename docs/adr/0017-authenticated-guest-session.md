# ADR 0017: Establish an authenticated guest-session foundation

- Status: Accepted
- Date: 2026-08-28
- Extends: ADR 0003 and ADR 0016
- Superseded in part by: [ADR 0024, per-Instance guest responder authority](0024-per-instance-guest-responder-authority.md)

## Context

ADR 0016 proves one bounded command over a dedicated control UART, but its echoed launch challenge is observable by the guest and does not authenticate either peer.
ADR 0003 requires a fresh Instance, expected Generation, and exact operation to participate in authenticated first-command readiness.
The next safe increment needs a small cryptographic seam without claiming that key injection, snapshot repair, attestation, production isolation, or Ready already exists.

## Decision

`soma-guest` implements one fixed Noise profile: `Noise_NKpsk0_25519_ChaChaPoly_BLAKE2s`.
The host is the initiator and pins the guest responder's X25519 public key.
The guest holds the corresponding private key.
The original source of that keypair was trusted Generation metadata; under [ADR 0024, per-Instance guest responder authority](0024-per-instance-guest-responder-authority.md) the Host now samples it fresh for every Instance and delivers the private half in the launch page, as the amendment below records.
Both peers also hold one fresh 256-bit PSK provisioned with the exact 16-byte identity of one concrete Instance.

NK authenticates possession of the pinned responder private key to the host.
The `psk0` modifier authenticates possession of the Instance PSK before the first handshake message is accepted and gives the guest an authentication basis for the host.
This trust statement is valid only if the PSK is generated from an operating-system CSPRNG after Instance allocation, is injected through a confidential non-snapshot seam before vCPU resume, is never sent over the control UART, and is never reused by another Instance.
The current crate accepts explicitly Instance-bound caller provisioning and rejects a credential whose Instance does not equal the transcript binding before it constructs Snow state.
It does not implement the secret-injection seam.
Until that seam and its lifecycle evidence exist, this is an authenticated-protocol foundation rather than a production authenticated guest channel.

Generation construction originally created the responder keypair, provisioned the private key into the trusted guest-agent artifact, and recorded the public key in the trusted content-addressed Generation manifest.
[ADR 0024, per-Instance guest responder authority](0024-per-instance-guest-responder-authority.md) supersedes that provisioning rule because a private key embedded in a publicly retrievable artifact cannot prove exclusive Generation possession and must not support a production authentication claim.
The Host now generates one fresh responder keypair per Instance, delivers the private half in the non-snapshot launch page, and retains the public half itself.
The host must never obtain the responder public key from an unauthenticated guest message.
Responder public-key admission performs an X25519 exchange with a fixed validation scalar and rejects a non-contributory all-zero shared output.
Every handshake uses a focused wrapper over Snow's `DefaultResolver` that applies the same all-zero rejection after every static or ephemeral DH operation.
The wrapper delegates the primitive implementation to Snow and does not maintain a hand-written low-order-key list.

## Transcript binding

Both peers encode one exact prologue before the Noise handshake.
The canonical byte layout is:

| Field | Encoding |
| --- | --- |
| Domain | Exact ASCII bytes `SOMA-GUEST-CONTROL\0` |
| Schema version | Unsigned 16-bit big-endian integer, currently `1` |
| Authentication profile | Unsigned 16-bit big-endian integer, currently `1` |
| Generation | Raw 32-byte content digest |
| Instance | Raw 16-byte canonical identifier |
| Operation | Raw 16-byte canonical identifier |
| Launch nonce | Fresh 32-byte value |

No string, JSON, optional field, host-endian integer, or algorithm negotiation participates in this encoding.
The Noise protocol name is also bound by the Noise handshake transcript.
Changing any binding field causes authentication to fail.
The launch nonce and Noise ephemeral key prevent a captured response from completing a fresh launch handshake, provided the operating-system RNG is sound.

## Handshake and record grammar

Handshake messages carry no application payload.
Each of the two Noise messages is prefixed by one unsigned 16-bit big-endian length and is capped at 256 bytes.
The responder API retains transport state behind a typestate boundary while the caller borrows message two.
The caller must transmit that message before consuming the explicit transition, although this byte-oriented module cannot observe or prove the external I/O operation.
Neither side exposes an authenticated transport session before its own two-message state transition is complete.
An integration must not send a command or record an authenticated milestone before the complete handshake.

After Noise split, each UART record has one unsigned 16-bit big-endian ciphertext length followed by exactly that many ciphertext bytes.
The authenticated plaintext contains an unsigned 64-bit big-endian directional sequence, an unsigned 16-bit big-endian payload length, and exactly that payload.
Each direction starts at sequence zero and accepts only the next exact sequence.
The fixed 16-byte AEAD tag and 10-byte plaintext header leave a maximum caller payload of 65,509 bytes inside Noise's 65,535-byte message bound.
The first malformed, unauthenticated, duplicate, skipped, reordered, truncated, or trailing peer record poisons both directions of that session.
There is no resynchronization, plaintext fallback, counter override, algorithm negotiation, early application data, or rekey operation in this one-command foundation.

The module interface owns handshake state, framing, counters, bounds, error redaction, and poisoning.
Callers see typestate handshake transitions plus `seal` and `open` rather than Snow, cipher, nonce, or transcript internals.
The existing version 1 semantic command frame can later become one encrypted payload without changing its parser.
That integration must use a parallel authenticated device profile or explicit protocol version and must not silently fall back to challenge echo.

## Dependency and secret handling

The crate pins `snow` 0.10.0 with default features disabled and enables only Curve25519, ChaCha20-Poly1305, BLAKE2, and operating-system randomness.
It pins `zeroize` 1.9.0 for owned private-key and PSK wrappers.
Secret types do not implement `Clone` or `Copy`, and their `Debug` output is redacted.
The private-key `Vec` returned by Snow and the crate-owned destination array are placed in `Zeroizing` wrappers while this crate owns them.
This statement does not cover optimizer-created copies, caller-created copies, or Snow's internal key storage.
The explicit provisioning callback warns that any caller-created copy leaves this crate's erasure boundary.

Snow 0.10.0 does not implement `Zeroize` or a zeroizing `Drop` for all internal DH, handshake, cipher, and transport key copies.
This crate therefore cannot claim complete key erasure even though its caller-owned wrappers zeroize their storage.
Production admission requires an independently reviewed resolver or upstream erasure guarantee, plus process-lifetime containment that destroys residual state after one Machine.

This initial crate is a portable Rust foundation and contains no C ABI.
The current static C ARM64 fixture cannot consume it directly.
A later guest adapter may link a Rust `staticlib` behind a fixed, narrow C ABI with explicit ownership, panic-abort, length, and destruction tests.
That adapter must not expose a general Noise configuration surface or add unsafe code to this protocol module.

## Verification

Public-interface tests complete a real two-message handshake and exchange authenticated records in both directions.
Hostile tests cover a PSK bound to the wrong Instance, the wrong PSK bytes, the wrong responder key, known non-contributory X25519 public values, changed Generation, changed Instance, changed operation, changed launch nonce, replayed handshake response, truncated and trailing handshake messages, corrupted records, duplicate and reordered records, cross-launch record replay, malformed outer framing, and poison persistence.
The cross-launch record test holds the Generation keypair, Instance PSK, Generation, Instance, and operation constant while changing only the launch nonce and fresh Noise session.
Test-only authenticated senders encrypt deliberately invalid inner sequences, declared lengths, and trailing bytes so those checks are reached after successful AEAD verification.
Boundary tests round-trip an exact 65,509-byte payload, reject one byte more before cipher state advances, and then prove the session remains usable.
Tests also verify all-zero key and identity rejection, explicit key-provisioning access, and redacted secret and error formatting.
A frozen byte vector pins the exact canonical prologue and Noise protocol name.
The fixed Noise profile and contributory resolver also pass the upstream Cacophony `NKpsk0_25519_ChaChaPoly_BLAKE2s` handshake vector distributed with Snow 0.10.0.

No standalone fuzz target is added for this fixed framing layer because its only pre-AEAD parser branches are exact frame size, one 16-bit declared length, and a fixed minimum, while its post-AEAD branches are one 64-bit sequence and one 16-bit declared length.
Deterministic tests exhaust all 65,536 values of the handshake, outer-record, and inner-payload declared-length fields, cover the handshake and exact outer-record boundaries, exercise sequence failures through an authenticated peer, and poison on all rejection paths.
This finite exhaustive matrix is stronger than sampled fuzzing for the current length grammar.
A semantic command codec, streaming decoder, or new variable-width field must add its own fuzz target rather than relying on this rationale.
Integration must later repeat the existing exact ARM64 command matrix over the encrypted transport and prove that no request bytes precede the authenticated handshake.

## Consequences

SOMA gains a small cryptographically authenticated session module with explicit transcript and replay boundaries.
It does not yet provide confidential snapshot-safe PSK injection, trusted manifest verification, static C guest integration, KVM transport integration, remote attestation, production key erasure, authenticated repair, or ADR 0003 Ready.
No sandbox, production-security, availability, or latency claim follows from these tests.
