# ADR 0020: Define launch-page and authenticated application wire contracts

- Status: Accepted
- Date: 2026-08-29
- Extends: ADR 0003 and ADR 0017
- Extended by: ADR 0021, ADR 0023, and ADR 0024

## Context

ADR 0017 authenticates a Noise session when both peers already possess one Instance PSK and one transcript binding.
It deliberately leaves confidential PSK delivery, restored-guest entropy repair, semantic command framing, and Ready integration undefined.
ADR 0003 requires an authenticated guest to complete its certified Repair contract and a real no-op command before Ready.

The next vertical slice needs exact portable bytes for two boundaries.
The first boundary carries fresh per-Launch secrets into restored guest memory without placing them in immutable snapshot state or on the control transport.
The second boundary carries one bounded typed application message inside one authenticated Noise record.

These byte codecs cannot prove that a KVM memory slot is confidential, that guest Repair actually occurred, or that an exchange is in the correct lifecycle state.
ADR 0021 owns the semantic lifecycle state, while physical repair and execution remain obligations of the Linux KVM adapter and static guest agent.

## Decision

### Fresh launch material

`HostLaunchMaterial::generate` accepts one nonzero Generation, Instance, and operation identity.
It rejects a zero caller identity before requesting randomness.
It requests 128 bytes from Snow's operating-system random resolver and partitions them into a 32-byte launch nonce, 32-byte Instance PSK, and independent 64-byte guest entropy seed.
An all-zero nonce, PSK, or entropy seed rejects that sample and retries the entire sample up to four times.
Four reserved-zero samples fail as `RandomnessUnavailable`.
An operating-system random failure fails immediately and never substitutes deterministic bytes.

Host launch material, delivered host launch material, guest launch material, guest session material, the private Instance PSK, and the entropy seed do not implement `Clone` or `Copy`.
Their debug representations do not include secret or identity bytes.

The host API uses typestate to enforce the local delivery order.
`HostLaunchMaterial::deliver_with(self, callback)` consumes undelivered material and encodes one exact 4096-byte page in an internal `Zeroizing` buffer.
It invokes the callback exactly once with an immutable scoped reference, wipes the internal page after callback success or failure, and returns `DeliveredHostLaunchMaterial` only when the callback reports success.
ADR 0021 makes `DeliveredHostLaunchMaterial::start_initiator` crate-private and lets `HostControl::connect` consume delivered material for exactly one Noise initiator handshake.
The callback boundary makes a partial source page unrepresentable, but callback success is only a local report and is not evidence of a correct KVM mapping.
The typestate prevents accidental reuse of the owned Rust states through the safe API.
It does not prevent a callback from deliberately copying the bearer-secret page bytes.

`InstancePsk`, `InitiatorHandshake`, and `ResponderHandshake` are crate-private.
Both raw factories consume their Instance PSK by value.
ADR 0021 makes the initiator-awaiting-response and responder-pending-response states crate-private behind the semantic owner.

### Launch-page version 1

The launch page is exactly 4096 bytes with this canonical layout.

| Offset | Size | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 16 | Domain | Exact ASCII bytes `SOMA-LAUNCH-PAGE` |
| 16 | 2 | Page schema | Unsigned 16-bit big-endian integer `1` |
| 18 | 2 | Authentication profile | Unsigned 16-bit big-endian integer `1` |
| 20 | 32 | Generation | Raw content-digest bytes |
| 52 | 16 | Instance | Raw canonical identity bytes |
| 68 | 16 | Operation | Raw canonical identity bytes |
| 84 | 32 | Launch nonce | Fresh operating-system random bytes |
| 116 | 32 | Instance PSK | Fresh operating-system random bytes |
| 148 | 64 | Entropy seed | Fresh operating-system random bytes |
| 212 | 3884 | Reserved | All zero |

The page schema versions the delivery encoding and is independent from the session-prologue schema.
The authentication profile identifies the same fixed cryptographic profile used by `SessionBinding`, so both encodings use one private `AUTH_PROFILE` constant.
Changing either schema or profile requires a new frozen vector and an explicit compatibility decision.

The page is a bearer secret because possession exposes the Instance PSK and entropy seed for this Launch.
The guest parser accepts only `&mut [u8; 4096]` with the exact domain, schema, profile, nonzero identities and secrets, and zero reserved tail.
Every malformed page maps to the single redacted `LaunchPageRejected` error.
`GuestLaunchMaterial::take_from_page` wipes the entire supplied slice after both successful and failed parsing.
Secret fields are copied directly into `Zeroizing` storage and partially constructed secret buffers are dropped on every error path.

`GuestLaunchMaterial::reseed_with` consumes the entropy seed through one callback.
Only a successful callback returns `GuestSessionMaterial`, which ADR 0021's `GuestControl::connect` consumes to start exactly one Noise responder handshake.
This typestate makes responder ephemeral-key generation unavailable before the caller reports successful guest entropy repair.
The callback must actually mix the seed into the guest kernel random subsystem and must not retain its own copy.

Rust and `zeroize` cannot promise that the optimizer, Snow internals, kernel copies, or a delivery or reseed callback never create an additional secret copy.
This slice promises erasure only for the internal delivery page, the exact guest page passed to the parser, and explicit owned `Zeroizing` buffers while this crate owns them.
ADR 0017's process-lifetime containment requirement for residual Snow state remains in force.

### Required Linux injection integration

The production Linux KVM adapter must place this page in a dedicated fresh anonymous 4096-byte guest-memory slot that is not part of the immutable Generation snapshot.
It must create the slot only after the single winning worker receives the concrete Instance identity.
The delivery callback must copy directly into that dedicated mapping and must report success only after the complete page is present.
Outside the scoped internal delivery buffer, the adapter must maintain exactly one physical guest copy and must erase that mapping after any failed delivery.
It must exclude the mapping from snapshot capture and complete the page write before any restored vCPU can resume.
The page must never travel over UART, virtio-console, vsock, network, logs, receipts, snapshot artifacts, or caller-visible configuration.

The trusted guest agent must consume the page before accepting the first Noise handshake message.
It must prevent workload children from inheriting or mapping the page and must wipe and unmap it before executing user work.
The host must observe the page as zero before it can authorize Ready.
Callback success alone does not establish single-copy delivery, snapshot exclusion, guest unmapping, or host zero observation.
Failure at any injection, consumption, entropy-repair, authentication, or zero-observation step destroys the one-use VMM process.

ADR 0024 supersedes the original rule that the responder private identity is immutable Generation material outside the launch page.
The responder static secret is now sampled per Instance with the launch nonce, Instance PSK, and entropy seed, and it occupies bytes 247 through 278 of the schema 3 page.
Every injection, single-copy, snapshot-exclusion, erasure, host-observation, and retirement obligation above therefore also protects the responder secret.
The public half is retained by the Host that generated it and is the only half that may reach a receipt, log, or other publicly retrievable object.

### Application frame version 1

One application message occupies exactly one `AuthenticatedSession` payload.
There is no application fragmentation, concatenation, compression, algorithm negotiation, extension block, or plaintext fallback.

Every application message begins with this 28-byte header.

| Offset | Size | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 4 | Magic | Exact ASCII bytes `SOMA` |
| 4 | 2 | Version | Unsigned 16-bit big-endian integer `1` |
| 6 | 1 | Kind | Fixed value from the kind table |
| 7 | 1 | Flags | Zero |
| 8 | 2 | Reserved | Zero |
| 10 | 16 | Operation | Nonzero raw canonical identity bytes |
| 26 | 2 | Body length | Unsigned 16-bit big-endian exact remaining length |

The complete header and body must not exceed the existing 65,509-byte Noise record payload maximum.
Truncated bytes, trailing bytes, unknown kinds, nonzero flags, nonzero reserved bytes, zero operations, wrong-direction kinds, and any invalid body map to `ApplicationMessageRejected`.

The fixed kind table is:

| Value | Direction | Meaning | Body |
| ---: | --- | --- | --- |
| 1 | Host to guest | `PrepareAndProbe` | Command |
| 2 | Host to guest | `Execute` | Command |
| 3 | Host to guest | `Shutdown` | Empty |
| 129 | Guest to host | `RepairComplete` | Empty |
| 130 | Guest to host | `Stdout` | Output chunk |
| 131 | Guest to host | `Stderr` | Output chunk |
| 132 | Guest to host | `Terminal` | Terminal report |
| 133 | Guest to host | `ShutdownAck` | Empty |

`PrepareAndProbe` instructs the trusted agent to complete the certified Generation Repair contract and then run ADR 0021's fixed self-probe through the same executor and result path as `Execute`.
`RepairComplete` is an authenticated agent report bound to the session and Launch operation.
It is not independently sufficient evidence for Ready.

### Command body

A command body contains, in order, a nonzero unsigned 32-bit big-endian timeout in milliseconds, a nonzero unsigned 64-bit big-endian combined-output allowance, an unsigned 16-bit big-endian program length plus program bytes, an unsigned 16-bit big-endian argument count, and that many unsigned 16-bit length-prefixed argument byte strings.

The program is nonempty, absolute, NUL-free, and at most 4096 bytes.
Arguments are shell-free byte strings, may be empty, must be NUL-free, and are individually at most 4096 bytes.
A command contains at most 64 arguments.
The timeout is at most 3,600,000 milliseconds.
The combined-output allowance is at most 16 MiB.
The complete encoded command must fit one application body and therefore one Noise record.

The current `soma-vmm` request surface admits up to 4096 arguments and 1 MiB of aggregate argument bytes.
That surface must be reconciled with this wire contract before KVM integration.
The adapter must not truncate arguments, fragment a version 1 command, or silently accept a request it cannot represent.

### Output and terminal bodies

One stdout or stderr body is a nonempty binary chunk of at most 4096 bytes.
NUL and non-UTF-8 bytes are valid output.

A terminal report is exactly 16 bytes.
It contains a one-byte status kind, three zero reserved bytes, one signed 32-bit big-endian detail, one unsigned 32-bit big-endian stdout byte count, and one unsigned 32-bit big-endian stderr byte count.
The two counts must have a checked sum no greater than the protocol's 16 MiB output maximum.

The terminal status mapping is:

| Kind | Meaning | Detail |
| ---: | --- | --- |
| 1 | Exited | Exit code from 0 through 255 |
| 2 | Signaled | Linux signal from 1 through 64 |
| 3 | Timed out | Zero |
| 4 | Output limit | Zero |
| 5 | `execve` failed | Positive Linux errno from 1 through 4095 |
| 6 | Agent failed | Positive internal code from 1 through 4095 |

### Stateful acceptance remains outside the codec

The application codec is deliberately stateless.
Codec validity alone is never semantic acceptance, lifecycle evidence, or authority to publish Ready.

ADR 0021 implements the authenticated session owner that enforces exactly one in-flight command or Shutdown operation.
It must reject a kind that is illegal for the current direction or exchange phase.
It must require every guest response operation to equal the exact in-flight operation.
It must allow `RepairComplete` only once and only during `PrepareAndProbe`, before any command output or terminal report.
It must accumulate stdout and stderr lengths with checked arithmetic and enforce the requested combined-output allowance while chunks arrive.
It must require the terminal stdout and stderr counts to equal the exact accumulated authenticated chunk counts.
For `OutputLimit`, it must additionally require the terminal count sum to equal the requested allowance.
It must accept exactly one terminal report and no later output for that operation.
It must require `ShutdownAck` to match the exact in-flight Stop operation.

Any decode error, operation mismatch, illegal state transition, count mismatch, duplicate terminal, output after terminal, or output overflow must poison the authenticated channel and fail the Machine.
There is no application resynchronization or retry inside a poisoned session.
The state owner must own Snow plus transport so poisoning is atomic and cannot be bypassed through a second shallow state machine.
Direct raw transport and `AuthenticatedSession` ownership are crate-private so callers cannot bypass semantic acceptance.

## Verification

Frozen vectors pin the complete launch-page prefix, its zero reserved tail, the relation between page and Noise authentication profiles, one Execute frame, and one counted terminal frame.
Public tests cross one generated page, wipe it, pass the entropy-repair boundary, complete Noise, and exchange an authenticated record.
Compile-fail tests prove that raw PSKs, handshake states, factories, and transport are not public, the guest parser requires an exact page, undelivered launch material is non-cloneable, and consumed owner states cannot start two operations.

Deterministic fault tests prove that zero caller identities do not touch the random source, zero nonce, PSK, and entropy samples retry, and four zero samples fail.
The fixed-array parser makes wrong-size pages unrepresentable.
Hostile page tests cover domain, page schema, authentication profile, every zero identity or secret class, nonzero reserved bytes, and full-array wipe after every rejection.

Application tests cover every typed kind, exact maximum command size, one byte over the maximum, all local command and terminal bounds, direction confusion, malformed headers, malformed commands, malformed outputs, malformed terminal reports, and nonempty unit bodies.
Deterministic matrices exhaust all 65,536 values for the application body length, program length, argument count, and argument length fields.

The isolated `fuzz/` package pins `libfuzzer-sys` 0.4.13 and is not a member of the production workspace.
Its application-codec target sends raw arbitrary bytes through both directional decoders, then requires every accepted value to re-encode to the identical canonical bytes and decode to the identical typed message.
It also exercises deterministic canonical seeds for every message kind, a maximum-length command, a maximum output chunk, and header-preserving body mutations so actual runs reach deep valid parsers.
The reviewed cargo-fuzz release for this decision is 0.13.2.
A bounded cargo-fuzz smoke requires a supported nightly toolchain and remains a required CI or developer gate when that toolchain is available.

## Consequences

SOMA gains exact portable launch-delivery and authenticated application byte contracts with bounded allocation and no unsafe code.
The public codec remains small while ADR 0021's owner retains the deeper semantic state and poisoning policy.
The typestate boundaries make accidental owned-state reuse, pre-delivery host authentication, pre-reseed guest authentication, and repeated host or guest handshake starts unavailable through the safe API.
They do not prevent deliberate byte copying by delivery code or duplication of an already populated guest page.

This decision does not implement a static guest agent, KVM memory-slot injection, snapshot exclusion, guest kernel entropy repair, responder-key custody, UART or virtio transport, process execution, host zero observation, production Ready, or a production security claim.
It creates the narrow protocol foundation those integrations must satisfy.
