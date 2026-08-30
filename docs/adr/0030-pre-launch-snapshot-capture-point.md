# ADR 0030: Capture a Generation snapshot before any launch material exists

- Status: Accepted, with the responder-key consequence superseded
- Date: 2026-08-29
- Number: allocated as 0024 when this decision was written and corrected to 0030 on 2026-08-30, because [ADR 0024](0024-per-instance-guest-responder-authority.md) already held that number
- Extends: ADR 0002, ADR 0020, and ADR 0021
- Superseded in part by: [ADR 0024, per-Instance guest responder authority](0024-per-instance-guest-responder-authority.md)

## Context

Snapshot format v1 orders the quiesce preconditions a Generation builder must prove before it reads any machine state, and the first of them was `GuestAuthenticated`.
That ordering assumed a builder that authenticates an Instance, does work, and later returns the machine to a disconnected repair point, so that a captured session would have to be scrubbed out of guest memory before the image could be published.

The implemented guest agent reaches a strictly earlier point.
It performs early init, composes the root, flushes the private overlay, and then blocks in the launch-page wait with no Instance identity, no session key, no assigned context identifier, and no network identity anywhere in guest memory, because none of those values has been created yet.
[The fast path](../architecture/fast-path.md) already describes the Generation pipeline as booting the managed guest to its repair point and capturing there.

Proving `GuestAuthenticated` at that point is impossible, and arranging for it would mean creating exactly the material the snapshot must not contain and then trusting a scrub.

## Decision

Version 1 certifies one capture point: the disconnected repair wait the pinned guest agent enters before any launch page is written.

The first quiesce precondition becomes `GenerationAgentBooted` rather than `GuestAuthenticated`.
The builder proves it by observing the agent's own console marker, whose prefix identifies the pinned agent and whose text identifies the repair wait; the second precondition, `RepairPointReached`, is proven by the same observation.
The rest of the order is unchanged: ingress disabled, device work drained, overlay flushed, vCPU paused, queues proven quiescent.

Every snapshot carries a repair-point marker section naming the capture point and the exact console line, and restore refuses a snapshot that does not describe the pre-launch repair wait.

The guest agent flushes filesystems and prints the marker immediately before it blocks, so the console line is also the overlay durability boundary the device surface requires.

## Consequences

Capture has nothing to scrub.
No Instance identity, operation identity, nonce, pre-shared key, or context identifier can be in the memory object, because the machine never held one; the published objects are checked for the launch-page domain and for per-Instance authority rather than trusted to have been cleaned.

This consequence is superseded by [ADR 0024, per-Instance guest responder authority](0024-per-instance-guest-responder-authority.md), and is retained here only to explain what the 2026-08-29 capture run recorded.

> Superseded text, historical only: the Generation-scoped responder private key remains in the memory object by construction, since the compiler binds it into the initramfs and the agent holds it for the life of the machine.
> It is identical for every Instance of the Generation and is not Instance authority, and the retained evidence records its presence rather than implying its absence.

Under ADR 0024 no reusable responder secret exists.
Initramfs layout v3 removes `etc/soma/responder.key`, the responder static secret is sampled fresh per Instance, and it reaches the guest only through the non-snapshot launch page, so the memory object of a capture taken with current code holds no responder authority either.
The retained [x86_64 snapshot restore evidence](../evidence/2026-08-29-x86_64-snapshot-restore.md) predates that change and is historical: it records and scans a Generation-scoped responder private key in `memory.raw`, which current code no longer produces.

A builder that wants to warm caches with authenticated work before capturing cannot do so under version 1.
Adding that would require a second certified capture point, an agent transition that retires a session and returns to a disconnected wait, and evidence that the retirement leaves nothing behind; it is not part of this version.

## Alternatives considered

Keeping `GuestAuthenticated` and authenticating before capture was rejected: it puts launch material into the image that must then be removed, and a scrub is a weaker statement than never having created the material.

Treating the precondition as vacuously satisfied was rejected because a proof that cannot fail is not a proof.

## Verification

The snapshot codec's ordering test drives the renamed precondition in order and rejects every other order.
The live `x86_64` capture on the obsolete responder-key revision proves the marker, the quiesce preconditions, and the fixed read and publish order on a real machine, and the retained scan shows that the published objects carry no decodable launch page; the result is in [the x86_64 snapshot restore evidence](../evidence/2026-08-29-x86_64-snapshot-restore.md).
