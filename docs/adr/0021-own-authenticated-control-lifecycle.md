# ADR 0021: Own the authenticated control lifecycle

- Status: Accepted
- Date: 2026-08-29
- Extends: ADR 0017 and ADR 0020

## Context

ADR 0017 defines the authenticated Noise transport.
ADR 0020 defines fresh launch material and canonical application messages.
Both decisions intentionally leave the semantic lifecycle outside the codec.
That split permits a caller to decrypt an authenticated record and reject its application meaning without poisoning the underlying transport.
It also permits callers to enter responder transport mode before proving that handshake message two reached the peer.

SOMA needs one deep module that owns the Noise state, byte transport, repair gate, operation phase, and output accounting together.
The interface must prevent a caller from recovering a poisoned transport or starting a second operation through a stale owner.

## Decision

### Owned byte transport

`ControlIo` is the only public transport seam.
It provides deadline-aware exact byte reads, complete byte writes, and irreversible poisoning.
`HostControlIo` adds one deadline-aware `commit_repair` operation for the VMM-enforced repair gate.
Neither interface exposes descriptors, record framing, decrypted application bytes, or the adapter error.

Every byte operation receives an absolute `std::time::Instant`.
An adapter MUST return success or failure no later than that deadline, including when a deadline has already elapsed.
The adapter owns cancellation of any operating-system operation required to satisfy that contract.
Poisoning MUST initiate locally bounded cancellation and teardown without waiting for peer input or acknowledgement.
The VMM remains responsible for interrupting blocked device work, joining owned workers, closing transport resources, and cleaning up the sandbox after an ambiguous timeout.

The host uses one shared absolute deadline for all reads and writes in each stage or exchange.
The fixed liveness ceilings are ten seconds for Handshake, five seconds for Repair, the fixed one-second probe timeout plus one second of delivery grace for Probe, five seconds for Shutdown, and the validated command timeout plus one second of delivery grace for Execute.
These values are failure-containment ceilings, not launch or command latency targets.
All deadline arithmetic is checked and fails closed to an immediate deadline if an `Instant` cannot represent the requested ceiling.
Both reads of one length-prefixed frame receive the same deadline, so partial framing cannot renew the budget.
Every Execute output frame shares the deadline established before the request write, so a stream of small chunks cannot renew the command budget.

Guest connect, idle receive, and report methods require caller-supplied absolute deadlines.
This keeps sandbox TTL, agent cancellation, and control-plane policy outside the codec while still making every guest-side byte operation bounded.

The owner reads each two-byte peer length before allocating its body.
Handshake messages are limited to 256 bytes before allocation.
Encrypted records are limited to the unsigned 16-bit Noise framing maximum before allocation.
An invalid length poisons the owner without attempting stream resynchronization.

`HostControl::connect` owns the initiator handshake, writes message one, reads bounded message two, authenticates it, and only then owns transport mode.
`GuestControl::connect` reads bounded message one, authenticates it, writes the complete message two, and only then owns transport mode.
A failed responder write cannot yield a guest owner.

Raw PSKs, handshake states, handshake completion methods, `AuthenticatedSession`, and its seal and open methods are crate-private.
Delivered host launch material and guest session material can start raw handshakes only inside the owner implementation.

### Failure and poisoning

Every handshake, input/output, decrypt, application decode, operation, phase, repair, output-accounting, terminal, or acknowledgement failure consumes the owner.
The failure path invokes `ControlIo::poison` exactly once.
It returns only `ControlError`, which contains a redacted stage and class.
It never returns the transport, Noise state, peer bytes, adapter error, or a reusable poisoned owner.

The public stages are Handshake, Repair, Probe, Execute, and Shutdown.
The public classes are Io, Authentication, Protocol, Lifecycle, and Accounting.

### Host lifecycle

`HostControl` represents one authenticated session before repair.
`prepare_and_probe` consumes it and returns `RepairedHostControl` only after all readiness requirements succeed.
It sends exactly one fixed PrepareAndProbe request for the Launch operation bound into the Noise transcript.
It requires one authenticated RepairComplete with that exact operation.
It calls `commit_repair` exactly after that report authenticates and before it can accept probe completion.
It then requires one zero-output terminal report with status Exited(0) and exact zero counts.
RepairComplete alone is never Ready.

`RepairedHostControl::execute` consumes the idle owner while one operation is in flight.
It returns the only next repaired owner together with one typed `ExecuteOutcome` on success.
Valid nonzero exit, signal, timeout, exact output-limit, exec failure, and agent failure statuses remain ordinary typed outcomes.
`shutdown` consumes the repaired owner and requires one exact authenticated acknowledgement for the Stop operation.

Every operation identity is single-use within its authenticated session, including the Launch identity.
The host and guest each retain a private ledger capped at 65,536 identities per session.
Reuse or exhaustion is a lifecycle failure that consumes and poisons the owner before another request can be accepted.
This prevents a late terminal record from being interpreted as the result of a later operation that reused the same identity.

### Fixed readiness self-probe

Version 1 uses the trusted guest agent executable at `/proc/self/exe` with the single argument `--soma-ready-probe-v1`.
The timeout is 1,000 milliseconds and the output allowance is one byte because the version 1 command contract forbids a zero allowance.
Successful readiness still requires exactly zero output.

Public callers supply only the Launch operation when constructing PrepareAndProbe.
The codec constructs the fixed command internally.
The decoder rejects any substituted program, argument, timeout, or allowance.
`GuestRequest::PrepareAndProbe` exposes no command bytes to the trusted guest-agent caller.
The static guest agent must reserve this exact mode and make it a deterministic no-output self-check through the same executor and terminal path as Execute.

### Guest lifecycle

`GuestControl` owns the internal AwaitPrepare, ProbeAwaitRepair, ProbeStreaming, RepairedIdle, ExecuteStreaming, and ShutdownPending states.
`next_request` can receive only while no operation is in flight.
The initial request must be the fixed PrepareAndProbe for the Launch operation bound into the session.
RepairComplete is legal once and before any probe output or terminal result.
The version 1 probe rejects all stdout and stderr.

Execute output is admitted chunk by chunk against the command allowance before it is written.
The guest owner constructs terminal counts from the bytes it successfully sent.
OutputLimit is legal only when the exact combined count equals the allowance.
An illegal local reporting call consumes and poisons the guest owner under the same rule as hostile peer input.

### Output accounting

The host checks each authenticated output chunk before appending it.
Checked arithmetic enforces the exact combined allowance while the stream arrives.
The terminal stdout and stderr counts must equal the exact authenticated chunk totals.
OutputLimit additionally requires the combined total to equal the requested allowance.

One terminal report returns the host to idle.
A duplicate terminal or output record remains buffered on a stream transport until the next owner read.
It is rejected and poisons the session before any later operation can succeed.
The single-use operation ledger closes the otherwise ambiguous case where a caller attempts to reuse the completed operation identity.
The interface does not require a nonblocking transport probe or introduce a second end-of-exchange acknowledgement.
No protocol acknowledgement can prove that an authenticated peer will not send another late record after acknowledging.
The trusted guest agent and its authenticated control channel therefore form a trust boundary, while operation identities and the next owner read contain detectable violations.

## Verification

Public integration tests cross fresh launch material through the byte-only seam, complete both handshake messages, commit repair, pass the fixed probe, execute counted binary output, retain typed process outcomes, and require exact shutdown acknowledgement.
Compile-fail tests prove that raw handshake and transport states cannot be imported and that consumed host states cannot be reused.
Hostile authenticated-peer tests cover every guest message kind before repair, wrong operations, duplicate and late repair, output before repair, substituted probes, allowance overflow, count mismatch, incorrect OutputLimit, duplicate terminal, output after terminal, and wrong shutdown acknowledgement.
Host and guest tests prove that a completed operation identity cannot be reused.
Local guest tests cover output and terminal before repair, duplicate repair, allowance overflow, incorrect OutputLimit, and a second receive while an operation is active.
Scripted transports fail every individual read and write byte across handshake, repair, probe, Execute, and Shutdown.
Every injected failure returns no owner and records exactly one poison call.
Deadline-aware end-to-end tests prove that an already expired guest receive fails closed without peer input.
They record propagated `Instant` values and prove that both frame reads, host repair commit, and each host exchange retain their original absolute deadline.

## Consequences

SOMA gains one semantic owner whose small interface hides cryptography, framing, lifecycle, repair gating, and accounting.
The future UART or virtio adapter implements `ControlIo` and must not add a second lifecycle state machine.
The future Linux VMM implements `commit_repair` only after it has observed launch-page erasure and retired the injection memory slot.

This decision does not implement the static guest agent, KVM launch-page mapping, memory-slot retirement, entropy-system call, command executor, UART or virtio adapter, snapshot restoration, Generation certification, or a sandbox performance claim.
