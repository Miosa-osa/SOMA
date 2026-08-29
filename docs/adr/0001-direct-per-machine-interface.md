# ADR 0001: Use a direct per-machine interface

- Status: Accepted
- Date: 2026-08-28
- Decision owners: SOMA maintainers
- Host allocation amended by: ADR 0006

## Context

SOMA needs one provider-neutral seam between an operator and one hardware-isolated Machine.
The interface must hide KVM setup, snapshot compatibility, artifact verification, private copy-on-write restore, clone Repair, authenticated guest control, milestone ordering, rollback, and cleanup.
The topology requires one `soma-vmm` process per Machine.
ADR 0006 later adds a node-local allocator for unassigned single-use workers and sterile resources without changing the per-Machine interface.

The design was explored three ways before choosing the initial interface.
Each alternative was evaluated for depth, leverage, locality, seam placement, failure behavior, and fit with the one-process-per-Machine topology.

## Decision drivers

- A caller must not be able to publish a restored Machine before authenticated Repair and command readiness succeed.
- Provider concepts such as tenants, billing, plans, placement, pools, and public sandbox identifiers must remain outside SOMA.
- Dangerous host details such as filesystem paths, TAP names, device names, and arbitrary file descriptors must not become ordinary caller-supplied strings.
- Ambiguous retries must not create a second Instance or execute a different request under the same operation identity.
- The interface must remain usable from languages other than Rust.
- The per-Machine command path must not require a shared daemon merely to coordinate one Machine after ownership transfer.
- Tests must exercise the same public seam as production callers.

## Alternatives considered

### Alternative A: Direct per-machine commands

This alternative exposes three commands to one `soma-vmm` process:

```text
Launch(LaunchRequest)   -> LaunchReceipt
Execute(ExecuteRequest) -> ExecutionResult
Stop(StopRequest)       -> StopReceipt
```

`Launch` accepts an immutable Generation with exact Machine sizing, an Instance identity, and an operation identity in Phase 0.
Constrained resource capabilities prepared by the operator remain a production-interface requirement.
It owns verification, restore, authenticated Repair, a no-op execution probe, rollback, and the transition to Ready.
Phase 0 returns monotonic milestones in terminal outcomes and does not stream progress.
Any future progress transport must keep milestones inside the command response rather than creating a fourth lifecycle command.

Phase 0 `Execute` runs a command only on the Ready Instance and returns status plus bounded retained stdout and stderr.
Authenticated terminal evidence remains a production requirement.
`Stop` performs idempotent cleanup of resources owned by the matching Instance.
Phase 0 stores the terminal Stop receipt after successful cleanup but has no process shutdown handshake.
A production process must retain no KVM or tenant resources after cleanup and may exit after the terminal response is acknowledged or a bounded shutdown deadline expires.

Phase 0 binds every mutating request to an `OperationId` and compares the complete Rust request structurally.
Repeating the same identity and structurally equal request returns the recorded terminal outcome, while reusing an identity with a different request fails with an operation-conflict error.
An admitted Stop with incomplete cleanup remains in Reaping, and replaying that exact Stop is the only Phase 0 same-operation continuation.
The future encoded protocol must define canonical request normalization and fingerprinting before it can make equivalent guarantees across language adapters.

This alternative has the smallest external interface that still supports launch, useful work, and cleanup.
Its implementation is deep because all unsafe ordering and rollback knowledge remains behind the seam.
Its locality is strong because changes to restore or Repair affect the Machine module rather than every caller.

### Alternative B: Declarative host reconciler

This alternative exposes `Negotiate`, `Apply`, `Inspect`, and `Watch` against a host-level reconciler.
The caller submits a declarative Machine goal, required capabilities, certified artifact references, and resource grants.
The reconciler selects a restore strategy, adopts processes after crashes, converges durable desired state, and publishes sequenced events.

This shape is flexible for multiple backends, rolling policy changes, crash recovery, and fleet operations.
It also places the seam above a mandatory shared daemon, capability negotiation, a durable journal, and host-wide reconciliation policy.
Those responsibilities belong to an operator control plane in the first SOMA topology.

The interface would have lower locality for a per-Machine VMM because lifecycle mechanics, host policy, and fleet recovery would change together.
It would also risk turning SOMA into a second control plane before it has a proven host-wide responsibility.

### Alternative C: Daemon-owned `StartReady` transaction

This alternative optimizes the common caller through one host-wide `somad` command named `StartReady`.
The request contains a signed launch ticket plus the first command, and the result is a non-clone live handle.
The daemon owns lease renewal, a durable journal, reconnect replay, crash adoption, ordered cleanup, and creation of one `soma-vmm` child per Machine.

This alternative offers excellent product ergonomics because a caller makes one request and receives a command-ready Machine.
It also adds a mandatory process hop, host-wide lease authority, persistent state, process adoption, and a new high-value compromise target.
Those costs are justified only when SOMA owns a real cross-Machine responsibility that the operator cannot provide cleanly.

The useful ideas from this alternative remain in the chosen design.
The Phase 0 Ready value contains the operation, Instance, Generation, the Generation's exact `MachineSpec`, and ordered milestones.
Authenticated terminal evidence remains a production receipt requirement, while host paths and device names stay outside the ordinary public contract.

### Alternative D: Staged lifecycle toolkit

A staged toolkit would expose operations such as `Verify`, `MapMemory`, `CreateVm`, `RestoreState`, `ResumeVcpus`, `RepairGuest`, `Probe`, and `Destroy`.
This shape gives expert callers precise control and makes individual phases easy to benchmark.
It is rejected because it moves ordering, rollback, compatibility, and security invariants into every caller.
The module would become a shallow pass-through whose deletion removes little complexity.

Phase 0 exposes ordered milestones without timestamps.
Future production receipts may add monotonic stage timestamps behind the chosen interface.
They are evidence, not caller-controlled lifecycle transitions.

## Decision

SOMA will begin with Alternative A, the direct per-machine `Launch`, `Execute`, and `Stop` interface.
The seam lives at the local control channel of one `soma-vmm` process.
The Phase 0 public contract is provider-neutral and describes a Generation, an Instance, commands, receipts, milestones, and typed faults.
Resource capabilities remain part of the intended production contract.

Production `Launch` is an atomic security transaction rather than a synonym for process creation.
The Phase 0 state machine cannot report Ready until its Generation verification, restore, authentication, Repair, and no-op platform stages succeed in order.
Real compatibility checks, private restore, and authenticated guest evidence require production adapters.
No implementation may add a silent cold-boot, alternate-VMM, or compatibility downgrade path.

The future protocol may carry progress events and a terminal receipt within each command response.
Phase 0 has no encoded protocol or progress stream.
It does not expose an independent `Watch`, `Inspect`, or stage-control command in the first interface.
Under the production topology, the operator supervises the local process and retains the immutable terminal receipt for recovery.
After the VMM exits, retry resolution relies on that operator-retained receipt rather than a hidden host-wide SOMA journal.

## Consequences

Callers learn three commands while gaining the full launch, work, and cleanup lifecycle.
Restore strategy and guest protocol changes remain local to the Machine implementation.
The production per-Machine process must be independently jailed, supervised, measured, and terminated.
Fleet placement, durable host journals, and crash adoption remain operator concerns.
ADR 0006 assigns node-local admission and prepared unassigned capacity to a focused allocator because they are required for the certified latency path.

The interface cannot provide cross-process durable inspection after both the caller and VMM lose state.
The operator must persist requests and receipts if that recovery property is required.
A host component requires a separate ADR and must demonstrate a responsibility that cannot remain within one Machine.
ADR 0006 satisfies that condition for prepared worker allocation and resource inventory.

## Verification

Contract tests must drive `Launch`, `Execute`, and `Stop` through the same semantic public interface used by external callers.
When a protocol codec exists, transport conformance tests must prove that it preserves those semantics.
Phase 0 tests must cover replay with a structurally equal request, conflict under a reused operation identity, failure before Ready, terminal milestone ordering, Execute rejection before Ready, and idempotent Stop.
Wire conformance tests must cover canonical fingerprints only after that encoding is specified.
Linux end-to-end tests must additionally prove that the direct process topology contains a VMM crash to one Machine.
