# SOMA fast path

## Objective

SOMA optimizes the complete interval from an accepted Launch request through a successful authenticated command.
The design does not optimize an isolated VMM resume number while leaving resource setup, clone repair, networking, or command transport outside the measurement.

All values in this document are engineering budgets rather than measured claims.

## Work kept outside Launch

The Generation pipeline performs expensive work once:

- Resolve an OCI reference to an immutable digest.
- Pull and verify content.
- Normalize the root filesystem.
- Select and verify the guest kernel and command line.
- Boot the managed guest to its repair point.
- Capture memory and machine state.
- Produce compatibility, integrity, provenance, and guest-contract metadata.
- Certify and install immutable artifacts on compatible hosts.

Launch never pulls an OCI image, unpacks layers, boots a general distribution from power-on, or hashes every artifact byte.
Those operations remain observable as Generation build and installation stages rather than disappearing from product accounting.

## Warm on-demand restore

The intended warm request path is:

```text
validate fixed-size Generation identity
        |
        +--------------------+
        |                    |
        v                    v
open certified files    reserve logical resources
        |                    |
        v                    v
private memory map      cgroup, network, disk head
        |                    |
        +----------+---------+
                   |
                   v
             create KVM machine
                   |
                   v
             restore compact state
                   |
                   v
                resume
                   |
                   v
       repair and authenticate guest
                   |
                   v
          execute readiness command
```

Independent preparation may run concurrently, but every owned resource is recorded before it can leak.
Concurrency is bounded at host-global contention points rather than serialized behind one fleet-wide mutex.

## Prepared workers

The host runtime may prepare unassigned VMM worker processes before a request arrives.
A prepared worker may open invariant host interfaces, install its immutable jail policy, reserve bounded bookkeeping, and map read-only Generation metadata.
It must not contain tenant identity, writable guest state, a reusable authenticated session, or a Machine that has ever been assigned to another tenant.

Launch transfers exactly one prepared worker through a single-winner ownership exchange.
The worker receives fresh Instance, operation, network, disk-head, entropy, and authentication material only after that transfer.
If no compatible worker exists, the request uses the separately measured on-demand path rather than hiding a fallback inside prepared-path results.

Prepared workers remove executable loading, allocator growth, plugin discovery, invariant descriptor setup, and jail initialization from the request path.
They do not weaken the one-process-per-Machine ownership rule after assignment.

## Memory

The certified snapshot memory file is immutable and opened read-only.
The VMM maps it writable and private with `MAP_PRIVATE | MAP_NORESERVE` so guest writes become private anonymous pages while untouched pages can share the host page cache.
The VMM does not allocate and copy the entire guest RAM image before vCPU resume.
The VMM does not request eager population of every page.

Certification and installation establish artifact integrity before Launch.
Launch performs constant-size identity and compatibility checks against retained filesystem evidence.
Periodic audit may rehash full files outside the request path.

Userfaultfd demand paging remains an experiment until measurements show that it improves the real workload without weakening isolation or introducing tail-latency stalls.
The first implementation favors the kernel's ordinary file-backed page-fault path because it is smaller, easier to audit, and already shares clean pages.

## Disk

The certified root base remains immutable.
Each Instance receives a private writable head created through a copy-on-write primitive such as XFS reflink.
The filesystem interface proves shared-extent and copy-on-write semantics, but it does not guarantee constant p99 latency under arbitrary extent count or metadata contention.
The certified fast path therefore uses sterile disk heads prepared outside Launch unless measured on-demand clone latency passes the complete 100-way tail budget.
A filesystem or mount that cannot prove the required isolation semantics is incompatible rather than a reason to copy a full disk in Launch.

## Guest repair

The snapshot is captured at a narrow guest-agent repair point.
The agent performs no user work until it receives a fresh authenticated challenge bound to the globally unique `InstanceId`.
Repair replaces or invalidates cloned identity, entropy-derived state, time assumptions, network configuration, transport sessions, and captured credentials.

Repair uses one bounded request and one bounded response rather than a sequence of host polling loops.
Ready requires an authenticated no-op execution result over the repaired channel.
The user-visible `Execute` operation then runs the benchmark command through the same channel.

## Avoided request-path costs

- No OCI registry access.
- No layer extraction.
- No general-purpose BIOS or UEFI boot.
- No full memory copy.
- No full artifact hash.
- No deep snapshot delta traversal.
- No general-purpose request broker on the per-Machine control path after ownership transfer.
- No runtime plugin discovery.
- No per-device process startup in the first topology.
- No silent cold-boot fallback.
- No readiness polling interval.
- No reuse of an already assigned tenant machine under a new identity.

## Latency budget

The working warm-host budget is:

| Boundary | p50 target | p99 target |
|---|---:|---:|
| Decode, authentication, and admission | below 0.10 ms | below 0.30 ms |
| Prepared worker acquisition and dispatch | below 0.10 ms | below 0.50 ms |
| Fresh resource activation | below 0.30 ms | below 1.00 ms |
| Private mapping and KVM memory-slot binding | below 0.10 ms | below 0.30 ms |
| KVM, vCPU, interrupt, and device restore | below 0.55 ms | below 1.50 ms |
| Resume through guest control wake | below 0.50 ms | below 1.50 ms |
| Combined authentication, repair, and no-op | below 1.50 ms | below 3.50 ms |
| Receipt publication | below 0.10 ms | below 0.30 ms |
| Additive server create budget | below 3.25 ms | below 8.90 ms |
| Complete server-side create | below 5 ms | below 10 ms |
| First bounded command from accepted Launch | below 10 ms | below 20 ms |
| Complete ComputeSDK create-through-`node -v` | below 50 ms | below 90 ms |

The remaining public budget covers same-region transport, node selection, request scheduling, the user command, and response transport.
Any stage that consumes the budget receives direct monotonic instrumentation before optimization.
The external target is valid only for a recorded same-region route, persistent connection state, certified host class, declared pool depth, and declared cache state.
It is not a universal global round-trip guarantee because geographic latency can exceed the complete budget.

## Capacity classes

SOMA reports these mechanisms independently:

- Warm-page-cache on-demand restore creates a new KVM machine for the request.
- Prepared-worker restore reserves invariant process and host work before the request but still restores a fresh Machine with fresh mutable state.
- Paused-pool lease assigns a never-before-assigned restored machine.
- Ready-pool lease assigns a never-before-assigned machine that already passed repair and readiness.

A result never changes classes because one path happened to be faster.
The exact ComputeSDK comparison must name the class and disclose every operation performed before the timer.

## Development evidence

Apple Silicon macOS can validate domain logic, protocol validation, deterministic lifecycle tests, formatting, and dependency policy.
Only a Linux x86_64 host with accessible KVM can validate machine creation, memory mappings, ioctls, vCPU execution, namespaces, cgroups, TAP, seccomp, reflink behavior, and end-to-end latency.
SOMA does not convert a cross-platform unit test into a Linux performance claim.
