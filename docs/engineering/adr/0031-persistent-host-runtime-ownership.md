# ADR 0031: A persistent Host Runtime owns every managed Instance

- Status: Accepted
- Date: 2026-08-30
- Extends: ADR 0001, ADR 0006, ADR 0010, ADR 0011, and ADR 0021

## Context

The development KVM Backend stores one live Machine and authenticated guest session inside the command-line process that launched it.
This is sufficient for `soma run`, where one process performs Launch, Execute, and Cleanup.
It cannot implement the managed lifecycle, because a later `soma machine exec` process opens a new empty Backend after the Launch process and its Machine have exited.

The existing `soma-hostd` daemon owns prepared-worker allocation, durable claim records, capacity admission, and reconciliation behind a bounded Unix `SOCK_SEQPACKET` protocol.
It does not yet own the complete Instance lifecycle or launch one jailed native `soma-vmm` process per Machine.

A shallow process created only to keep an `Option<Live>` alive would make the benchmark pass while duplicating admission, idempotency, recovery, cleanup, and prepared-worker ownership.
The process-lifetime seam must instead become the production Host ownership seam.

## Decision

One long-running `soma-hostd` process is the persistent Host Runtime for one admitted Host.
It owns every managed Instance from accepted Launch intent through proven terminal cleanup.
CLI, MCP, and provider adapters are clients and never own live KVM objects, guest sessions, writable heads, TAP leases, or VMM processes.

The Host Runtime presents one versioned lifecycle interface:

```text
Resolve exact Generation
Launch request -> Launch receipt or typed rejection
Execute request -> bounded command receipt
Inspect request -> current durable lifecycle evidence
Stop request -> graceful terminal receipt
Destroy request -> forced terminal receipt
```

The existing provider-neutral request and receipt types remain the semantic interface.
The first local adapter carries bounded frames over the owned Unix `SOCK_SEQPACKET` endpoint.
A future remote control-plane adapter preserves the same semantics without exposing the local socket, Host paths, descriptors, or registry credentials.

## Ownership topology

```text
CLI / MCP / provider adapter
              |
              | versioned lifecycle request
              v
+------------------------------------------------------+
| soma-hostd persistent Host Runtime                   |
|                                                      |
| durable operations -> admission -> prepared claim    |
|                         |                            |
| InstanceId -> private Instance owner                 |
|                         |                            |
| reconciliation <- terminal cleanup evidence          |
+-------------------------+----------------------------+
                          |
                          | exact owned descriptors
                          v
                 soma-jail launcher
                          |
                          v
               one soma-vmm process per Machine
                          |
                          v
             one authenticated guest control session
```

Each live Instance has exactly one private owner inside the Host Runtime.
That owner holds the VMM process identity, authenticated session, prepared-worker lease, memory mapping identity, writable-head lease, network bundle, capacity reservation, operation history, and cleanup state.
No caller receives those resources directly.

## Deep module seam

The Host Runtime is a deep module rather than a collection of caller-visible managers.
Its external interface remains the six lifecycle operations above.
Its implementation composes these private modules:

- `operations` owns canonical request fingerprints, idempotent replay, changed-intent conflict, and durable intent-before-effect ordering.
- `admission` owns atomic Host capacity decisions across every resource dimension.
- `pool` owns sterile prepared workers and single-winner claims.
- `instance` owns one Instance state machine and all resource transfers.
- `machine` owns the jailed VMM process and authenticated guest session.
- `cleanup` owns reverse-order revocation and the terminal evidence ledger.
- `reconcile` reconstructs uncertain ownership after daemon or Host restart.
- `transport` validates bounded local protocol frames and peer authority without owning lifecycle policy.

These names describe cohesive policy or mechanism.
They are private implementation seams and do not become independent public crates unless a second real adapter or release contract requires it.

## Launch transaction

The production Launch order is fixed:

1. Decode and bound the request.
2. Authenticate the caller and canonicalize its complete request fingerprint.
3. Replay an identical completed operation or reject changed intent using the same Operation identity.
4. Durably record Launch intent before external effects.
5. Resolve one certified immutable Generation and verify Host compatibility.
6. Atomically admit capacity for the complete declared shape and policy.
7. Claim one sterile prepared worker and its prepared private resources.
8. Create the private Instance owner and durably bind every acquired resource before transfer.
9. Launch one `soma-vmm` through `soma-jail` and transfer only the exact admitted descriptors.
10. Bind fresh Instance identity, entropy, time, network identity, and one-use launch authority.
11. Restore and resume the guest.
12. Complete authenticated Repair and the fixed readiness command.
13. Activate admitted networking only after Ready.
14. Durably record the Launch result and return its receipt.

Any failure runs the reverse cleanup transaction.
Capacity returns only after terminal evidence proves every owned resource complete or reconciliation retains explicit uncertainty.

## Execute and terminal operations

Execute addresses an existing Instance by exact Instance identity and Operation identity.
It never accepts a Host path, process identifier, socket name, TAP name, or descriptor from the caller.
Output, time, descendants, and protocol frames remain bounded.
A timeout or ambiguous guest response poisons the session and begins bounded destruction so a late response cannot satisfy another operation.

Stop and Destroy are idempotent terminal operations.
Stop requests authenticated graceful shutdown before the deadline.
Destroy revokes authority and forces reclamation.
Both return the same complete cleanup evidence shape and preserve unresolved ownership for reconciliation rather than reporting success from process disappearance alone.

## Concurrency and recovery

The Instance ownership table is bounded by admission rather than by map capacity.
Entries are keyed by Instance identity and contain no globally shared mutable guest state.
Per-Instance lifecycle serialization prevents two operations from mutating one Machine concurrently, while unrelated Instances proceed independently.

On daemon or Host restart, `soma-hostd` replays durable operations and resource ledgers before admitting new work.
Every nonterminal entry is suspect until process, cgroup, namespace, storage, network, capacity, and authority disposition is reconstructed.
An uncertain Instance is destroyed or retained for reconciliation and is never returned to a sterile pool.

## Configuration ownership

Host-internal configuration belongs to `soma-hostd` startup and its admitted HostProfile.
Prepared-store roots, writable-head roots, filesystem tools, jail paths, cgroup roots, network brokers, and descriptor sources never travel through portable client requests.
Development environment variables may configure the current in-process Backend, but they are not the production interface.

The benchmark records safe configuration identities and exact Generation and receipt identities.
It never treats mutable Host paths as sufficient artifact provenance.

## Performance and benchmark consequence

Persistent ownership makes the local managed lifecycle measurable, but it does not by itself admit the production fast path.
The measured KVM campaign requires the certified Generation, prepared restore, jailed VMM, private resources, authenticated Ready, and cleanup transactions described above.

The local burst harness validates the lifecycle at concurrency 1, 10, and 100 through the same client seam.
The exact ComputeSDK campaign additionally requires a provider adapter outside the upstream benchmark repository and an unmodified upstream 100-way run.
No one-shot cold-boot result may be labeled restore or ComputeSDK Burst TTI.

## Rejected alternatives

Keeping one command-line process alive per managed sandbox was rejected because clients would own Host resources and client death would define sandbox lifetime.

Serializing `Option<Live>` to disk was rejected because KVM descriptors, memory mappings, authenticated sessions, and process ownership cannot be reconstructed from serialized Rust state.

Adding a benchmark-only keeper daemon was rejected because it would duplicate the production lifecycle and allow tests to pass through a seam operators do not use.

Putting every Instance into one VMM process was rejected because one compromised guest-facing VMM could affect unrelated tenants and one failure could destroy multiple Machines.

Exposing Host paths or descriptors in the portable interface was rejected because it leaks implementation details, weakens authority, and prevents remote adapters from preserving the same semantics.

## Verification gates

- Separate client processes can Launch, Execute, Inspect, and Destroy the same Instance through the Host Runtime.
- An identical Operation request replays exactly and changed intent conflicts without side effects.
- Client disconnect after every transaction step leaves either a completed result or durable reconcilable ownership.
- Daemon restart reconstructs or destroys every nonterminal Instance before admitting capacity.
- One Instance timeout, crash, or hostile workload does not block unrelated Instances.
- Every Machine has one separately jailed `soma-vmm` process and one authenticated guest session.
- Capacity, prepared-worker, storage, network, process, descriptor, and authority ownership return to baseline after the 1, 10, and 100 concurrency ladder.
- The local burst harness retains raw failures and cleanup results through the same lifecycle interface used by clients.
- The unmodified upstream ComputeSDK campaign runs through the provider adapter only after the local Host gates pass.

## Consequences

Managed sandboxes outlive individual CLI and MCP processes without weakening the one-process-per-Machine isolation rule.
Lifecycle complexity concentrates inside one Host Runtime interface instead of spreading across every client.
The existing `soma-hostd` pool and daemon become the foundation rather than being replaced by a benchmark workaround.
Production work must complete certification, jail, prepared restore, networking, ownership, reconciliation, and evidence gates before performance admission.
