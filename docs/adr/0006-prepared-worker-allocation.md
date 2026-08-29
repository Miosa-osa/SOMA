# ADR 0006: Prepare single-use workers and resources on each host

- Status: Accepted
- Date: 2026-08-28
- Amends: ADR 0001

## Context

ADR 0001 selected a direct `Launch`, `Execute`, and `Stop` interface to one VMM process per Machine.
It intentionally deferred a host-wide daemon until SOMA had a concrete cross-Machine responsibility.

SOMA now has a measured architectural requirement that cannot remain inside each request.
Reliable server-side create below 5 ms p50 and 10 ms p99 cannot include executable loading, allocator growth, jail initialization, dynamic TAP creation, cgroup creation, filesystem clone contention, immutable descriptor setup, and process supervision from zero.
A 100-way burst also needs node-local admission, CPU and NUMA placement, pool depth, and resource accounting without a fleet-wide mutex or synchronous central database.

The per-Machine VMM must remain single-tenant.
A guest-facing device-model failure must not gain authority over other Machines.
Prepared state must not carry identity, writable memory, credentials, or authenticated authority from one tenant to another.

## Decision

The certified fast path includes one small node-local allocator process, implemented by a focused `soma-host` library and `soma-hostd` binary when that phase begins.
The allocator owns only unassigned workers, sterile resource bundles, immutable Generation handles, host admission, and asynchronous replenishment.
It is not a shared VMM and never executes guest device logic.

Pools are sharded by compatibility class, CPU or NUMA domain, Machine shape, and Generation when measurements justify Generation-specific preparation.
There is no fleet-wide allocation mutex.
Fleet placement must select a host without adding synchronous central storage to the node launch transaction.

A prepared worker may contain:

- Loaded executable code and initialized fixed-size allocator arenas.
- A verified immutable jail policy.
- Open invariant host interfaces such as KVM, epoll, eventfd, and control descriptors.
- Immutable Generation file handles and read-only metadata.
- A virgin VM or vCPU shell only after experiments prove that its ownership and reset invariants are safe.

A sterile resource bundle may contain:

- An empty cgroup and namespace set.
- An unattached point-to-point TAP and preinstalled eBPF policy slots.
- A pre-created private disk head.
- Fixed-size control buffers and an unassigned socket pair.

Prepared state must not contain:

- Tenant, Instance, operation, credential, or billing identity.
- Writable guest memory or disk state previously assigned to another Machine.
- A reusable guest authentication session or challenge.
- A network lease that has been published to another Instance.
- A Machine that has executed tenant code.

Launch claims one worker and one compatible resource bundle through a single-winner ownership transition.
The allocator transfers constrained descriptors over a fixed-frame local `SOCK_SEQPACKET` channel with descriptor passing when that transport is implemented.
Fresh Instance identity, entropy, authentication material, network lease identity, and writable Machine state are attached only after ownership transfer.

An assigned worker is single-use and is destroyed after its Machine stops.
It is never scrubbed and returned to another tenant.
The allocator replenishes a new sterile worker asynchronously.

The direct on-demand launcher remains a correctness path and a separately measured fallback.
A result from that path must not be mixed with prepared-worker results.

## Per-Machine control path

The `Launch`, `Execute`, and `Stop` semantic interface from ADR 0001 remains unchanged.
After ownership transfer, the assigned VMM owns exactly one Machine and receives commands directly.
The allocator does not proxy steady-state guest commands and does not own the Machine's device loop.

This division preserves the deep per-Machine module while giving host allocation its own deep responsibility.
The VMM does not learn pool policy, fleet placement, central metadata, billing, or provider identity.

## Alternatives considered

### Start every VMM and resource from zero

This path remains useful as a fallback and diagnostic baseline.
It is rejected as the certified fastest path because process, jail, network, storage, and allocator tail latency would consume the complete create budget.

### Keep already-assigned VMMs in a reusable tenant pool

This option was rejected because proving complete removal of writable state, identity, connections, entropy, and credentials is harder than destroying the process.
SOMA uses single-use workers and never reassigns a tenant-executing process.

### One multi-tenant VMM daemon

This option was rejected because a guest-facing parser or device-model flaw could cross Machine ownership boundaries.
The allocator never handles guest device queues or guest memory.

### Fork a multithreaded runtime

This option was rejected because inherited locks, allocator state, descriptors, random state, and runtime threads make post-fork correctness difficult to prove.
Workers are created through controlled process supervision before they contain multithreaded tenant state.

## Performance budget

The following additive critical-path budget is an experiment target rather than a measured claim:

| Server create stage | p50 | p99 |
|---|---:|---:|
| Decode, authentication, and admission | 0.10 ms | 0.30 ms |
| Worker acquisition and dispatch | 0.10 ms | 0.50 ms |
| Fresh resource activation | 0.30 ms | 1.00 ms |
| Private mapping and KVM memory-slot binding | 0.10 ms | 0.30 ms |
| KVM, vCPU, interrupt, and device restore | 0.55 ms | 1.50 ms |
| Resume through guest control wake | 0.50 ms | 1.50 ms |
| Combined authentication, repair, and no-op | 1.50 ms | 3.50 ms |
| Publish receipt | 0.10 ms | 0.30 ms |
| Total | 3.25 ms | 8.90 ms |

The hard create gate remains below 5 ms p50 and 10 ms p99.
The unused margin covers measurement noise and bounded implementation variation rather than unreported work.

## Consequences

SOMA gains a real node-level responsibility and one additional trusted component.
That component requires a smaller attack surface than a fleet control plane and no guest-facing device implementation.

Capacity reservation becomes an explicit prerequisite for the fastest result.
Pool misses, on-demand fallback, paused-Machine leases, and Ready-Machine leases remain separate experiment classes.

The certified host class must reserve physical CPU capacity, host-control cores, NUMA-local memory, warm Generation page cache, and pool depth for the declared burst.
Performance evidence must include allocator saturation, replenishment behavior, and failure injection.

## Verification

Tests must prove single-winner worker acquisition, fresh identity attachment, descriptor ownership transfer, failure rollback, process destruction after assignment, and the impossibility of reassigning a used worker.
Burst tests must measure unsaturated and saturated acquisition, pool misses, replenishment, CPU placement, NUMA locality, and every resource leak.
Security tests must prove that allocator compromise does not grant direct access to guest memory or guest device queues after handoff.
