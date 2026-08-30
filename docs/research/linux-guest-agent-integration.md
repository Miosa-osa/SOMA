# SOMA Linux guest integration v1

## Decision

The statically linked `soma-guest` agent is the only process allowed to cross the restored-to-Ready boundary.
It starts from the deterministic initramfs, mounts the EROFS lower and private ext4 upper, pivots to OverlayFS, and blocks at a disconnected repair point before tenant code.

## Fresh launch material

The VMM creates one fresh 4 KiB launch page containing magic, schema, GenerationId, InstanceId, OperationId, nonce, Noise PSK material, guest entropy seed, assigned vsock CID, network generation, time sample, the fresh per-Instance responder static secret decided by ADR 0024, and a digest over the page.
The page occupies a dedicated KVM memory slot absent from the snapshot.
The guest copies it once into locked memory, validates every identity and bound, overwrites the page, and reports consumption.
The VMM removes the slot and observes host-side zeroes before committing repair.

## Repair state machine

The only states are `Captured`, `MaterialAccepted`, `EntropyRepaired`, `TransportFresh`, `IdentityRepaired`, `NetworkRepaired`, `Authenticated`, `Probed`, `Ready`, `Running`, `Stopping`, and terminal `Poisoned`.
Transitions are monotonic and owned by one typestated controller.
Failure, replay, duplicate transition, deadline, or protocol violation consumes the controller and destroys the single-use VM.

Repair reseeds the kernel CSPRNG from fresh host-provided virtio-rng output, discards user-space PRNG state, replaces machine-id, boot-id-dependent application state, hostname, vsock generation, network identity, resolver state, wall-clock assumptions, and cached authority.
Captured TCP and vsock connections are invalidated.
No tenant environment, credential, command, or network ingress exists before repair completes.

## Control and execution

The agent opens only the fixed vsock control port and completes the pinned Noise handshake already defined by ADRs 0017, 0020, and 0021.
Every request carries operation identity, sequence, absolute deadline, command and argument vector, bounded environment, working-directory policy, input allowance, output allowance, and cancellation generation.
There is no implicit shell.
The agent uses a bounded child process, process group, pipe set, output accounting, and terminal result, and reaps every descendant before acknowledging completion.
Both pipes are read by one bounded poll loop with no queue and no reader thread, and every read is bounded by the unspent output allowance plus one probe byte, so the resident cost of a command is one fixed chunk buffer whatever the child writes.
Reaching the allowance, a sink failure, or the deadline kills the complete process group at once and switches the loop to a drain bounded by a fixed grace.

Ready requires authenticated repair plus one fixed no-op command through the same production executor.
Shutdown requires an authenticated request, refusal of new work, child termination, filesystem sync, exact acknowledgement, and orderly poweroff.

## Modules and gates

`boot`, `launch_page`, `entropy`, `identity`, `network_repair`, `control`, `executor`, `output`, `shutdown`, and `pid1` are separate guest modules.
Linux tests must boot the pinned x86 Generation, restore it repeatedly, prove unique identities and entropy, reject captured sessions and malformed pages, enforce deadlines and output limits, prevent pre-Ready execution and ingress, execute Node 22, and prove complete descendant and filesystem cleanup.

## Status

The cold-boot half of that list has live x86_64 evidence: the static agent ran as PID 1 from the layout v2 initramfs, composed the root, consumed and erased the launch page, repaired entropy, identity, and network state, authenticated over vsock, answered the probe, executed one command, and shut down through the authenticated channel, as recorded in [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md).
The warm half now has evidence too: a `node:22` Generation was captured at the disconnected repair point and restored repeatedly, each Instance consumed a launch page it had never seen, repaired entropy, identity, and network state, adopted a fresh vsock context identifier, authenticated, answered the probe, and executed `node --version`, with different Instance identities, hostnames, machine identities, context identifiers, and private overlay heads between Instances.
The agent flushes filesystems and announces the repair point on the console before it blocks, so a builder can find the capture point while the machine is still running, and it waits for its assigned context identifier because the vsock driver adopts a restored assignment asynchronously.
Captured-session rejection remains unproven because version 1 never captures a session; the retained result is [the x86_64 snapshot restore evidence](../evidence/2026-08-29-x86_64-snapshot-restore.md).
The guest half of that interval is now measured from inside the agent, which attributes every step between resume and Ready to a named repair step rather than to a host-observed gap; the retained result is [the warm path guest breakdown](../evidence/2026-08-30-x86_64-warm-path-guest-breakdown.md).
Three of the intervals that breakdown named were then removed: the launch-page poll on both sides of the resume, the executor's flat wait for a reapable child, and the position at which the launch-page memory slot is added; the retained result is [the three cuts evidence](../evidence/2026-08-30-x86_64-warm-path-three-cuts.md).
The whole pass, its two baselines, its refusals, and its final ten-iteration `Ready` figures are consolidated in [the warm path optimization evidence](../evidence/2026-08-30-warm-path-optimization.md).
