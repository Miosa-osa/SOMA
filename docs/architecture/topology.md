# SOMA execution topology

## Decision

SOMA uses one native VMM process per machine.
The certified fastest topology also uses a small node-local allocator for unassigned single-use workers and sterile resource bundles.
After one ownership transfer, the assigned VMM receives commands directly and the allocator leaves the per-Machine control path.

This topology keeps tenant failures local, preserves independent process credentials and seccomp policies, and prevents a corrupt guest-facing device model from owning other tenants' machines.

## System topology

```text
                           outside launch latency

OCI reference
     |
     v
generation builder
     |
     | verify, convert, boot, repair, quiesce, capture, certify
     v
immutable Generation
     | kernel
     | root filesystem base
     | memory snapshot
     | machine and device state
     | guest agent
     | compatibility manifest
     | provenance and integrity evidence
     v
artifact store and host page cache

                            request path

operator control plane
     |
     | one atomic Launch specification
     v
soma-hostd node allocator
     | claims one unassigned single-use worker
     | activates a sterile cgroup, namespace, TAP, disk head, and control channel
     | transfers constrained ownership exactly once
     v
one soma-vmm process
     | main control and device event loop
     | one dedicated KVM_RUN OS thread per vCPU
     | private memory mapping over shared immutable snapshot backing
     | private copy-on-write disk head
     | minimal virtio block, net, vsock, and rng devices
     v
managed Linux guest
     |
     | authenticated generation handshake
     | identity, entropy, time, network, and transport repair
     | first command probe
     v
READY receipt
```

## Process ownership

One `soma-vmm` process owns exactly one KVM VM.
It owns the KVM VM file descriptor, vCPU file descriptors, guest-memory mappings, virtual devices, control socket, and lifecycle state.
It does not own placement, tenant policy, billing, public routing, OCI credentials, or warm-pool policy.

For a one-vCPU machine, the steady state contains:

- One main OS thread for control and device events.
- One dedicated vCPU OS thread that enters `KVM_RUN`.

The main thread uses Linux readiness mechanisms such as `epoll`, `eventfd`, and `timerfd`.
The first implementation does not add an asynchronous runtime or one process per emulated device.
Those changes require measurement and a separate security decision.

## Launch sequence

The launcher and VMM emit these monotonic milestones:

```text
REQUEST_ACCEPTED
RESOURCES_OWNED
PROCESS_STARTED
ARTIFACTS_VERIFIED
MEMORY_MAPPED
KVM_CREATED
KVM_STATE_RESTORED
VCPU_RESUMED
AGENT_AUTHENTICATED
GENERATION_ACKNOWLEDGED
IDENTITY_REPAIRED
NETWORK_REPAIRED
FIRST_COMMAND_SUCCEEDED
READY
```

A milestone is diagnostic evidence rather than permission to expose the machine.
Only `READY` permits the operator to publish the Instance.
Any failure before `READY` triggers idempotent rollback of every resource recorded in the ownership ledger.

## Snapshot memory

Every child maps the same immutable memory artifact with private copy-on-write semantics.
The launch path must not eagerly copy the full guest RAM image.
The launch path must not populate every page, hash every memory byte, or traverse a deep delta chain before resume.
Full integrity verification happens when a Generation is built, installed, or audited.
Launch performs a constant-size fail-closed check against certified filesystem identity and manifest metadata.

Working-set prefaulting is allowed only as a measured policy outside the VMM's compatibility contract.
Userfaultfd and live write-protection are later experiments rather than first-generation requirements.

## Disk topology

The root filesystem base is immutable.
Each Instance receives a private copy-on-write disk head on a filesystem with a proven reflink contract.
The initial MIOSA deployment target uses XFS `FICLONE` for this operation.
SOMA reports an explicit error if the requested storage topology cannot provide the required isolation semantics.

## Network topology

Each Instance receives its own network namespace, point-to-point TAP, host-side lease, route, and policy identity.
A certified fast profile may give every isolated guest link the same internal guest IP and MAC while host eBPF maps bind the unique Instance lease and external identity to its dedicated TAP.
That profile removes guest reconfiguration only when there is no shared layer-2 domain and conformance proves cross-Instance routing and policy isolation.
Another profile may assign fresh guest-visible addresses during Repair.
The execution receipt records the effective network identity strategy.
Inherited live connections are invalid after restore.
The Instance is not ready until the guest acknowledges the new control generation and the host has activated the unique network lease.

## Clone repair

Snapshot fan-out duplicates user-space and kernel state.
SOMA treats the following values as unsafe until replaced or invalidated:

- VM generation identity.
- Guest boot and machine identity.
- Hostname.
- Network lease and guest address assumptions.
- Vsock connection generation.
- Entropy and user-space random state.
- Wall clock and monotonic-time assumptions.
- Stale TCP and vsock connections.
- Cached credentials or one-time tokens captured in memory.

The builder must quiesce the guest at an explicit repair point.
The restored guest must not execute user code before Repair succeeds.

## Burst topology

A 100-way launch shares immutable Generation artifacts and page-cache residency but shares no mutable machine state.
The operator prepares independent IDs, cgroups, namespaces, TAPs, disk heads, and control channels concurrently.
Every VMM maps the same immutable memory inode privately.
The operator applies a bounded concurrency policy to setup work that contends on global kernel locks or storage metadata.

The VMM does not implement a multi-Machine pool.
The node allocator owns unassigned prepared workers and sterile resource bundles, while operators may separately keep already-restored paused machines outside request latency.
Benchmarks must report those paths separately from on-demand restore.

## Failure and cleanup

Resource ownership is established before mutation and recorded before publication.
Cleanup is idempotent and safe after timeouts, parent death, child crash, partial namespace setup, or ambiguous control-channel outcomes.
The launcher uses process identity that cannot be confused by PID reuse.
The operator never assumes a timeout means a launch did not happen.

## Rejected topologies

- A VMM NIF inside the BEAM is rejected because a memory-safety error would corrupt the entire control plane.
- A multi-tenant VMM daemon is rejected because one guest-facing bug would expand across machines.
- Forking the multithreaded BEAM or another managed runtime as a zygote is rejected.
- One process per virtual device is deferred because it increases launch work and is not required for the first minimal device surface.
- A hidden always-hot pool inside the VMM is rejected because it confuses restore mechanics with capacity policy and obscures benchmark boundaries.
- Reusing a worker after it has executed tenant code is rejected because complete scrubbing is harder to prove than process destruction.
