# ADR 0003: Define Ready as authenticated command completion

- Status: Accepted
- Date: 2026-08-28
- Decision owners: SOMA maintainers
- Amended by: [ADR 0039, the authenticated repair report alone proves readiness](0039-repair-report-alone-proves-readiness.md)

## Context

Restoring vCPU and device state does not prove that a cloned guest is safe to expose.
A restored guest may still carry a duplicate machine identity, stale entropy state, inherited network configuration, invalid clocks, old transport sessions, cached credentials, or a guest agent from the wrong Generation.
Console text, a connected socket, and an unauthenticated ready message can all occur before those hazards are repaired.

The public benchmark also measures from create request through a successful command.
SOMA therefore needs a readiness definition that is both a security property and an honest end-to-end performance boundary.

## Decision

An Instance is Ready only after the expected guest agent authenticates to the Launch and completes all required Repair work, then successfully executes an authenticated no-op command over the repaired control channel.
ADR 0039 removed the no-op command from that definition on measured cost: Ready is the authenticated repair report, and every other requirement below still stands.
The Phase 0 Ready value contains the Instance identity, Generation identity, operation identity, the Generation's exact `MachineSpec`, and ordered milestones.
It does not yet contain a normalized request fingerprint, command outcome, evidence digest, or monotonic host timestamps.
Those fields remain production receipt requirements after their wire representations and authenticated evidence formats are specified.

The following is the required production sequence.
Phase 0 models the post-restore authentication, acknowledgement, Repair, and first-command ordering without claiming a vCPU or guest exists.

```text
VCPU_RESUMED
AGENT_AUTHENTICATED
GENERATION_ACKNOWLEDGED
IDENTITY_REPAIRED
NETWORK_REPAIRED
REPAIR_REPORTED
READY
```

The guest agent must answer a fresh per-Launch challenge and prove that it belongs to the expected Generation rather than merely presenting a static image credential.
Challenge material and authenticated channel state from an earlier Instance are invalid and must not become reusable snapshot authority.
The control protocol must resist replay of a success from an earlier Instance or connection generation.

Repair replaces or invalidates cloned machine identity, hostname, entropy-dependent user-space state, time assumptions, network identity, vsock generation, stale connections, and captured one-time credentials before user work may execute.
If a Generation cannot satisfy the declared Repair contract, Launch fails and cleanup begins.

`PROCESS_STARTED`, `MEMORY_MAPPED`, `KVM_STATE_RESTORED`, `VCPU_RESUMED`, console output, socket acceptance, agent connection, and an agent ping are milestones only.
None authorizes the caller to publish the Instance.
There is no timeout path that converts an intermediate milestone into Ready.

## Alternatives considered

### Process-start readiness

Reporting Ready when `soma-vmm` starts measures process creation rather than an executable guest.
It is rejected because KVM creation, restore, Repair, and guest execution may all fail afterward.

### vCPU-resume readiness

Reporting Ready after the first vCPU enters `KVM_RUN` provides a useful internal latency milestone.
It is rejected because the restored guest has not proved liveness, Generation identity, or Repair.

### Guest-agent connection readiness

A connected agent proves that some guest process reached the control transport.
It is rejected because cloned sessions, the wrong agent, incomplete Repair, and immediate command failure remain possible.

### Authenticated ping readiness

An authenticated ping proves transport and agent liveness.
It is rejected because the command execution path, process setup, process creation, and result transport remain untested.

### Workload-specific command readiness

Executing the caller's first workload command provides the closest product signal.
It makes Launch semantics depend on arbitrary command behavior and can conflate a workload error with a Machine failure.
Production SOMA transitions to Ready on the authenticated repair report, while an external benchmark continues through its required command such as `node -v`.

## Consequences

Launch latency includes guest control and Repair.
This definition is intentionally stricter than VMM resume microbenchmarks.
The guest agent and its authentication protocol become security-critical parts of every Generation.
The operator may expose intermediate milestones for diagnosis but may not label them Ready.

The exact ComputeSDK Burst TTI result remains longer than SOMA's internal Ready latency because its timer begins outside SOMA and ends after `node -v` succeeds through the provider path.
Both measurements must be retained without substituting one for the other.

## Verification

End-to-end tests must prove that Ready is impossible when authentication, Generation acknowledgement, any Repair step, or result authentication fails.
Replay tests must reject receipts and agent messages from another Instance or Launch.
Concurrent restore tests must prove unique machine, network, transport, and entropy-derived identities before Ready.
Milestone-order tests must reject missing, repeated, reordered, or regressing lifecycle evidence.
