# ADR 0039: The authenticated repair report alone proves readiness

- Status: Accepted
- Date: 2026-08-31
- Amends: [ADR 0003, authenticated command readiness](0003-authenticated-command-readiness.md), [ADR 0020, launch page and application wire contracts](0020-launch-page-and-application-wire-contracts.md), [ADR 0021, own the authenticated control lifecycle](0021-own-authenticated-control-lifecycle.md)

## Context

ADR 0003 defined Ready as authenticated repair followed by one successful no-op command, and ADR 0021 fixed that command as the guest agent executing itself with `--soma-ready-probe-v1`.
The reasoning was that a connected agent and an authenticated ping leave the execution path untested, so readiness should exercise the same path a caller's first command will take.

The measured cost of that reasoning was not known when it was written.
A per-stage timeline of a restored Instance on the measurement host, recorded through `SOMA_KVM_TIMELINE` and retained in [the eval-1 readiness split](../evidence/2026-08-31-eval1-ready-segment-split.md), puts the whole `machine_launched` to `ready` segment at 27.9 ms and the probe at 3.9 ms of it, in every configuration and at every concurrency: it is a fixed cost paid by every Instance ever launched.

The probe also proves less than it appears to.
The readiness receipt of ADR 0033 binds the snapshot, the published launch authority, and the live Noise transcript, and the transcript is fixed when the handshake completes.
Running a command afterwards adds nothing the receipt can attest, because the receipt is identical whether the command ran or not.
What the probe genuinely proves is that the guest agent can fork, exec, stream, and reap in the restored process; what it cannot prove is anything about identity, which is the property ADR 0030 and ADR 0033 exist to protect.

## Decision

Ready is authenticated repair, committed and reported over this Instance's own session, and nothing further.

`PrepareAndProbe` becomes `Prepare`, wire kind 1, with an empty body.
It carries no command, so no command can be substituted into it and the decoder rejects any body at all.
The guest answers it with exactly one authenticated `RepairComplete` for the bound Launch operation, the host commits the repair gate, and the host owner becomes `RepairedHostControl`.
`ControlStage::Probe`, `GuestState::ProbeAwaitRepair`, `GuestState::ProbeStreaming`, the reserved `--soma-ready-probe-v1` agent mode, and the guest lifecycle phase `Probed` are removed rather than left as names for work nobody does; the guest phase between `Authenticated` and `Ready` is `Prepared`.

Nothing about the identity boundary moves.
The snapshot is still captured at the pre-launch repair point, the readiness receipt still binds Instance, Launch operation, and live transcript, and every repair step ADR 0003 requires still runs and is still reported before Ready.
What is removed is a process, not a proof.

## Alternatives considered

### Keep the probe and make it cheaper

A lighter self-check inside the agent, reported over the existing session, would cost less than a fork and exec.
It is rejected because it would still be a second message about a guest that has already authenticated one, and the fork is most of the cost it would save.

### Retire the launch page earlier so the probe overlaps it

Removing the launch page's KVM memory slot costs a read-side grace period, and it was measured at 1.3 ms sitting between the repair report and Ready.
Moving it to just after the guest's control connection opens, so that it overlaps the guest's identity and network repair, was implemented and measured: the ready segment got 1.2 ms *slower*, because removing a memory slot disturbs a running guest more than it disturbs an idle one.
It is rejected on that measurement, and the removal stays at the repair commit.

### Generate the host's ephemeral handshake key before the request

The host's Noise initiator setup costs 0.42 ms on the measurement host.
It is not on the critical path: the host completes it and writes handshake message one about 7 ms before the guest, still repairing its identity and network, reads it.
Moving it earlier is rejected as a change that would buy nothing measurable until a prepared-worker pool makes "before the request" mean something.

## Consequences

An Instance whose agent can authenticate but cannot execute now reaches Ready and fails on the caller's first command instead of failing at Launch.
That is a worse failure position for a broken Generation and a better one for every working Generation, and a Generation that cannot execute fails its own compilation long before an Instance restores from it.

The wire contract changes, so an existing prepared Generation cannot be launched by a newer host.
Generations are rebuilt from their candidate, so this is a rebuild rather than a migration.

## Verification

Codec tests prove that `Prepare` round trips with an empty body and that a `Prepare` frame carrying any body is rejected.
Hostile-peer tests prove that a body smuggled into `Prepare` poisons the guest owner once, that every other guest message before repair still poisons the host owner once, and that a second repair report after the commit poisons the next exchange.
Owner tests prove that repair commits exactly once, that the repaired owner is returned only after an authenticated `RepairComplete` for the bound operation, and that the deadline of every frame is still shared within the frame.
Scripted transports still fail every individual read and write byte across handshake, repair, Execute, and Shutdown, and every injected failure returns no owner and records exactly one poison call.
