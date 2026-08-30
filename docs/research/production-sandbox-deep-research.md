# Production sandbox architecture research

- Date: 2026-08-29
- Scope: SOMA's production Linux KVM architecture, fast restore path, authority model, networking, containment, cleanup, capacity, and performance admission
- Method: Primary project documentation, kernel documentation, source repositories, and peer-reviewed systems research
- Status: Architectural recommendation, not implementation evidence

## Executive conclusion

SOMA should build one hardware-isolated sandbox product with multiple preparation classes, not several unrelated sandbox architectures.
The product is one lifecycle and one security contract.
Cold construction, warm restore, prepared worker, and ready pool are different ways of preparing the same Machine.

The correct production shape is a tightly integrated Rust Host runtime and VMM, surrounded by narrow privileged brokers and a jailed per-Instance VMM process.
The fast path should restore an immutable, memory-local snapshot and private writable head, inject fresh per-Instance authority through a nonsnapshot channel, authenticate the guest, repair cloned state, and publish Ready only from authenticated evidence.

Ten milliseconds is a credible engineering objective only for a prepared restore or ready-pool claim on a tuned Linux host with hot metadata and memory pages.
It is not a credible common label for OCI download, image conversion, cold Linux boot, snapshot fetch, guest repair, command execution, and cleanup combined.

SOMA's strongest current decisions align with the literature and mature VMMs:

- A deliberately small device model.
- Hardware virtualization as the primary tenant boundary.
- Per-Instance launch authority outside reusable artifacts.
- A guest agent that repairs cloned state before user execution.
- Content-addressed immutable Generations.
- Private writable storage heads.
- Explicit cleanup ownership.

The largest remaining architectural risk is not the KVM core.
It is authority and lifecycle composition across `soma-hostd`, `soma-netd`, `soma-storage`, `soma-jail`, the VMM, and the guest session.

## Facts established by mature systems

### A microVM is not secure merely because it uses KVM

Firecracker describes KVM and processor virtualization as the primary isolation layer, but its production guidance also requires seccomp, a jailer, cgroups, namespaces, dropped privileges, patched kernels and microcode, controlled files, bounded serial output, and host firewalling.
Firecracker explicitly states that it does not filter guest network traffic and that operators must install host firewall rules.
It also recommends one tenant workload per Firecracker process.
[Firecracker production host setup](https://github.com/firecracker-microvm/firecracker/blob/main/docs/prod-host-setup.md)

The implication for SOMA is direct.
`soma-kvm` is necessary but insufficient.
The real security product is the composition of KVM, the device model, the VMM jail, privileged-broker authorization, storage ownership, network policy, guest authentication, and cleanup.

### Minimal device models reduce attack surface, but every device remains hostile-input code

Firecracker deliberately exposes a small machine model with virtio block, net, vsock, and rate limiting.
Its design separates an API thread, VMM thread, and vCPU threads within one process.
[Firecracker design](https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md)

Crosvm goes further in privilege separation by allowing a separate jailed process for each virtual device.
Its control sockets are capability-shaped so a compromised device with one class of authority cannot perform unrelated VM mutations.
Its seccomp policies are per device and per architecture.
[Crosvm architecture](https://github.com/google/crosvm/blob/main/ARCHITECTURE.md)

The implication is not that SOMA must copy crosvm's process-per-device model.
That model increases process, IPC, scheduling, and lifecycle cost.
The lesson is that device authority must be narrow even when devices remain in one VMM process.
Every virtqueue parser needs hostile-input validation, bounded work per notification, fuzzing, and a precise resource interface.

### Integrated Rust VMMs can reduce control-plane overhead

Kata Containers 4.0 integrates its Rust runtime, Dragonball VMM, and related infrastructure more tightly than older multi-process designs.
Its documented goal is to remove unnecessary IPC and reduce resource consumption and lifecycle latency while retaining a pluggable hypervisor interface.
[Kata 4.0 architecture](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture_4.0/architecture.md)

Libkrun similarly embeds a deliberately small VMM behind a compact library interface and supports KVM on Linux and HVF on Apple Silicon.
It explicitly does not attempt to become a general-purpose VMM.
[libkrun](https://github.com/containers/libkrun)

The implication for SOMA is to keep `soma-vmm` tightly coupled to the Host implementation through typed in-process plans and descriptor handoff, while retaining process isolation around the resulting VMM execution.
Do not turn every internal action into an RPC.
Do not merge privileged network and storage mutation into the unprivileged VMM merely to remove RPC.

### Snapshot restore requires state repair, not only memory mapping

Firecracker warns that network and vsock connections can be lost across restore.
It resets virtio-vsock transport state so existing connections close and listening sockets can adopt the new context identifier.
Snapshot performance varies with memory size, vCPU count, and emulated-device count.
[Firecracker snapshot support](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md)

AWS SnapStart documents that clones from one snapshot can duplicate IDs, secrets, entropy-derived state, cached timestamps, connections, and other initialized state.
It requires uniqueness to be established after restore and provides before-checkpoint and after-restore hooks.
[AWS SnapStart uniqueness](https://docs.aws.amazon.com/lambda/latest/dg/snapstart-uniqueness.html)
[AWS snapshot contents](https://docs.aws.amazon.com/lambda/latest/dg/microvms-images-snapshots.html)

The research literature reaches the same conclusion.
Restoring uniqueness is part of the cold-start design, not application cleanup that can be deferred.
[Restoring Uniqueness in MicroVM Snapshots](https://arxiv.org/abs/2102.12892)

The implication for SOMA is that `Ready` must mean the current Instance, not the captured machine, is trustworthy.
Fresh entropy, identity, time assumptions, vsock generation, network configuration, credentials, writable storage identity, and guest-session authority must be repaired or replaced before user code or network forwarding is admitted.

### Snapshot speed is primarily a data-locality problem

AWS describes initialized snapshots as immutable, encrypted, chunked, cached artifacts and uses multiple cache layers to reduce retrieval latency.
[AWS SnapStart internals](https://aws.amazon.com/blogs/compute/under-the-hood-how-aws-lambda-snapstart-optimizes-function-startup-latency/)

SnapFaaS and related systems show that snapshot layering, working-set loading, and avoiding unnecessary page transfer are central to low cold-start time.
[SnapFaaS](https://www.tan-yue.com/assets/papers/snapfaas.pdf)
[SEUSS](https://arxiv.org/abs/1910.01558)

Recent work such as Sabre targets memory-snapshot compression and prefetch coverage because snapshot bytes and page locality dominate restore cost at scale.
[Sabre, OSDI 2024 proceedings](https://www.usenix.org/system/files/osdi24_proceedings_interior.pdf)

The implication for SOMA is that optimizing VMM instruction count alone will not produce a reliable 10 ms system.
The Host must know whether snapshot metadata, working-set pages, writable heads, kernel structures, VMM process slots, network bundles, and cgroups are already prepared and local.

### Guest agents are a standard control mechanism, but their authority differs by design

Kata runs one agent inside each VM and communicates with it over vsock using ttRPC.
The agent creates and manages the workload environment inside the VM.
[Kata architecture](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/README.md)

The useful lesson is the separation between Host lifecycle and guest workload lifecycle.
The dangerous lesson would be to trust a transport merely because it is vsock.
SOMA is correct to add a cryptographically authenticated session and operation identities because a CID or socket endpoint alone is routing, not proof of Instance identity.

### Networking must have separate production and portability adapters

Firecracker uses host TAP devices and leaves routing and filtering to the operator.
[Firecracker design](https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md)

Libkrun supports conventional virtio-net through `passt` or `gvproxy` and a custom transparent socket impersonation mode over vsock.
Its security documentation says the guest and VMM share the VMM's effective network context when the VMM proxies sockets.
[libkrun networking and security model](https://github.com/containers/libkrun)

The implication for SOMA is to define one guest-visible network intent but two real Host adapters:

- A Linux production adapter using TAP, a private network namespace, nftables, conntrack zones, and explicit ingress and egress policy.
- A portability adapter using a userspace proxy for macOS and unprivileged development.

These adapters do not provide identical semantics.
The capability ledger and receipts must record which adapter ran.
The portability adapter must never be used as evidence for Linux production networking performance or isolation.

### Capacity is multidimensional and must be admitted atomically

Linux cgroup v2 exposes independent CPU, memory, process, I/O, and pressure controls.
CPU quota and weight are not vCPU ownership.
Pressure stall information exposes time in which runnable work is stalled on CPU, memory, or I/O.
[Linux cgroup v2](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html)
[Linux pressure stall information](https://cdn.kernel.org/doc/html/latest/accounting/psi.html)

Kata notes that VMMs, shims, device backends, and associated threads consume Host resources beyond the workload's declared resources.
[Kata Host cgroups](https://github.com/kata-containers/kata-containers/blob/main/docs/design/host-cgroups.md)

The implication for SOMA is that admission cannot be `requested_vcpus <= host_threads`.
It must atomically reserve a vector containing at least resident private memory, expected shared pages, CPU quota or shares, process and thread budget, file descriptors, KVM slots, vsock IDs, TAP and network leases, conntrack zones, storage heads, I/O budget, and cleanup reserve.

### Honest latency measurement requires explicit boundaries and open-loop pressure

HdrHistogram documents coordinated omission, where a stalled system causes a closed-loop load generator to stop issuing work and therefore hide most affected requests.
Its corrected recording methods account for the missed expected intervals.
[HdrHistogram](https://github.com/HdrHistogram/HdrHistogram)

The implication for SOMA is to keep raw per-operation monotonic timestamps and run both closed-loop lifecycle tests and open-loop arrival-rate tests.
Median, P95, P99, and P99.9 must be computed over declared cohorts that include failures and admission delay according to the published metric definition.

## Recommended SOMA architecture

```text
PUBLIC LIFECYCLE INTERFACE
Create | Execute | Inspect | Stop | Destroy | Snapshot | Restore
                         |
                         v
+---------------------------------------------------------------+
| Instance Lifecycle module                                     |
| Owns state machine, operation identity, deadlines, receipts,  |
| compensation, recovery, and terminal cleanup proof            |
+---------------------------------------------------------------+
       |            |             |            |           |
       v            v             v            v           v
  Admission     Generation     Storage      Network      VMM Launch
  module        resolver       module       module       module
       |            |             |            |           |
       |            |             |            |           v
       |            |             |            |    soma-jail + soma-vmm
       |            |             |            |           |
       |            |             |            |           v
       |            |             |            |       soma-kvm
       |            |             |            |           |
       |            |             |            |           v
       |            |             |            |    Linux guest Machine
       |            |             |            |           |
       |            |             |            |           v
       |            |             |            | authenticated guest agent
       |            |             |            |           |
       +------------+-------------+------------+-----------+
                         receipt ledger
```

The Instance Lifecycle module should be the deepest product module.
Its external interface should be small.
Callers should not coordinate storage, networking, jail, VMM, or guest transitions themselves.
Those mechanisms belong behind internal seams.

### External interface

The public interface should expose lifecycle intentions and immutable observations, not subsystem steps.

```rust
trait SandboxBackend {
    fn create(&self, request: CreateRequest) -> Result<InstanceReceipt, SandboxError>;
    fn execute(&self, request: ExecuteRequest) -> Result<ExecutionReceipt, SandboxError>;
    fn inspect(&self, id: InstanceId) -> Result<InstanceView, SandboxError>;
    fn stop(&self, request: StopRequest) -> Result<StopReceipt, SandboxError>;
    fn destroy(&self, request: DestroyRequest) -> Result<CleanupReceipt, SandboxError>;
}
```

Snapshot and restore can join this interface when their production semantics are stable.
They should not force callers to learn VMM memory slots, TAP descriptors, snapshot files, or guest protocol frames.

### Internal modules and their authority

| Module | Owns | Must not own |
| --- | --- | --- |
| Instance Lifecycle | State machine, deadline, compensation, receipts, recovery | Raw KVM or nft implementation |
| Admission | Atomic resource vector and cleanup reserve | Launch side effects |
| Generation | Immutable verified launch material | Instance secrets or leases |
| Storage | Private writable head and release proof | Guest readiness |
| Network | Namespace, TAP, policy, lease, conntrack, forwarding | Ability to assert guest repair |
| Jail | Host-process containment and descriptor allowlist | Lifecycle truth |
| VMM Launch | Machine assembly and VMM process ownership | Privileged host mutation beyond granted descriptors |
| Guest Session | Authentication, repair, readiness, execution, shutdown | Host network or storage mutation |
| Evidence | Append-only receipts and redacted measurements | Authority to perform lifecycle transitions |

This design creates leverage and locality.
A caller makes one lifecycle request.
The implementation handles complex ordering, rollback, timeouts, and proof behind that interface.

## Authority design

SOMA should use capabilities for commands and receipts for facts.
They are not interchangeable.

```text
Control plane intent
       |
       v
Operation capability, single use
       |
       v
Lifecycle transition
       |
       v
Authenticated or kernel-derived evidence
       |
       v
Receipt recorded in ledger
       |
       v
Capability for the next transition
```

Required properties:

- Every capability is unforgeable outside its owner module.
- Every capability binds Instance, operation, lifecycle generation, intended transition, and expiration.
- Every capability is consumed once.
- A receipt cannot be constructed from caller assertion.
- Kernel peer identity can authenticate a local process, but application capability still authorizes the operation.
- A daemon restart cannot turn a stale request into fresh authority.
- Uncertain reply delivery never repeats a non-idempotent effect without ledger reconciliation.

For network activation, the guest-session owner should validate authenticated repair and produce a single-use activation capability.
`soma-netd` should verify and consume that capability before enabling forwarding.
The network daemon must not manufacture the proof it is supposed to verify.

## Fast-path design

### Preparation classes

| Class | Work completed before timer | Honest use |
| --- | --- | --- |
| Cold Generation build | Nothing | Image pipeline and reproducibility |
| Cold machine boot | Generation exists | Kernel and guest boot regression |
| Warm snapshot restore | Snapshot and storage template local | General low-latency sandbox creation |
| Prepared worker | VMM process, jail, cgroup, network bundle, and files prepared | Lower process and Host setup latency |
| Paused pool | Restored Machine exists but lacks fresh authority | Very low activation latency with bounded resident cost |
| Ready pool | Authenticated Instance already Ready | Lowest assignment latency with the highest idle cost |

The product may support every class while retaining one sandbox lifecycle.
The receipt must state the actual class used.

### Recommended 10 ms path

```text
0.0 ms   Claim reserved Host slot and prepared bundle
         |
         v
1.0 ms   Clone or map private writable storage head
         |
         v
2.0 ms   Map snapshot memory and restore compact machine state
         |
         v
4.0 ms   Attach fresh CID, launch page, TAP, block, and eventfds
         |
         v
5.0 ms   Resume vCPU and wake reset devices
         |
         v
7.0 ms   Guest repairs identity, entropy, time, and network
         |
         v
9.0 ms   Authenticated probe completes
         |
         v
10 ms    Ready receipt commits and forwarding may activate
```

This is a target budget, not a current claim.
It assumes hot local pages, no OCI work, no remote snapshot fetch, no cgroup creation, no nft process spawn, no VMM binary load, and no queueing.

The design consequence is important.
If `nft`, cgroup creation, VMM process startup, filesystem formatting, or remote artifact fetch remains on the timed path, the 10 ms target will be fragile or impossible at tail latency.
Those resources must be prepared safely or replaced with lower-overhead kernel and library interfaces.

### Memory restore strategy

Use immutable shared snapshot backing with private copy-on-write semantics.
Do not copy the entire guest RAM before resume.
Track the measured working set per Generation and host class.
Prefault only pages that repeatedly block the readiness path.
Keep cold pages demand-paged when tail latency permits it.

The Host cache needs explicit states:

- Metadata present.
- Snapshot file local.
- Page cache warm.
- Readiness working set prefaulted.
- Prepared process slot available.
- Paused Machine available.

Never call all of these states simply warm.

## Network design

### Production Linux adapter

The production adapter should use:

- One private network namespace per Instance or one equivalently isolated prepared bundle.
- TAP passed by descriptor to the jailed VMM.
- A unique conntrack zone or an explicitly collision-free replacement.
- nftables rules installed from structured, validated policy.
- Default-deny ingress.
- Egress policy that blocks Host, metadata, control-plane, private-service, and disallowed destination ranges.
- Optional DNS proxying with policy and bounded caching.
- Forwarding disabled until authenticated guest repair.
- Complete release and reconciliation proof.

Do not spawn `nft` and `conntrack` on the readiness critical path if 10 ms is the objective.
Prefer direct netlink or a long-lived, authenticated privileged broker with bounded operations.
Prepared bundles can hold sterile namespaces and links, but cannot hold tenant identity, active leases, or permissive forwarding.

### Portable adapter

The portable adapter can use `passt`, `gvproxy`, or an equivalent userspace network implementation.
It exists for macOS, local development, and unprivileged environments.
It must expose capability differences explicitly, including unsupported raw sockets, packet semantics, ingress behavior, observability, and performance.

### Ingress and egress are separate capabilities

Egress is not an optional afterthought.
It is a primitive because an agent sandbox must have a defined default network authority, including none.
Ingress is optional because many sandboxes never accept inbound connections.

The primitive is `NetworkPolicy`.
Specific egress, DNS, port publication, and proxy features are configured capabilities within it.

## Cleanup and recovery design

Cleanup must be designed as a production operation rather than a destructor side effect.

Every acquired resource needs:

- A durable owner identity.
- A lifecycle generation.
- A creation receipt recorded before or atomically with the effect.
- An idempotent release operation.
- A bounded release deadline.
- A reconciliation probe based on kernel truth.
- A tombstone proving terminal cleanup or naming residual resources.

The Host must reserve CPU, memory, process, and descriptor capacity for cleanup even under overload.
Admission that consumes the final cleanup capacity is unsafe.

Crash recovery should replay compensation from the ledger and then reconcile with kernel truth.
It must never assume that a missing success reply means the effect did not occur.

## Capacity and density design

Overcommit should be a declared workload policy rather than a hidden ratio.

### Safe admission vector

```text
admit = min(
  CPU policy capacity,
  resident private-memory capacity,
  shared-page cache capacity,
  storage-head capacity,
  I/O budget,
  process and thread budget,
  file-descriptor budget,
  KVM and memory-slot limits,
  CID and network-lease capacity,
  conntrack and nft capacity,
  cleanup reserve
)
```

Two hundred 1-vCPU sandboxes can reside on an 80-thread Host when the workload class permits CPU overcommit.
That does not mean 200 sandboxes can simultaneously consume one full vCPU without queueing.
The admission contract should distinguish resident, runnable, active, and guaranteed CPU.

Use cgroup v2 `cpu.max`, `cpu.weight`, `memory.max`, `memory.high`, `pids.max`, I/O controls, and per-cgroup pressure metrics.
Use pressure and tail latency to reduce admission before the Host reaches hard failure.

## Benchmark and evidence design

SOMA should publish five different numbers rather than one boot number:

1. Generation build latency.
2. Cold Machine boot to authenticated Ready.
3. Warm snapshot restore to authenticated Ready.
4. Prepared-worker or paused-pool claim to authenticated Ready.
5. Ready Instance Execute round trip.

For each class, retain:

- Intended and actual admission time.
- Every lifecycle milestone from one monotonic clock domain.
- Queue delay.
- Success, typed failure, and cleanup outcome.
- Host hardware, kernel, microcode, mitigations, KVM, cgroup mode, filesystem, NUMA topology, and power settings.
- Source commit, build profile, binary digests, Generation, snapshot, and configuration identities.
- Raw samples or mergeable histograms.
- Median, P95, P99, P99.9, maximum, failure rate, and cleanup failure rate.

Run both:

- Closed-loop correctness cohorts that wait for complete cleanup.
- Open-loop burst cohorts that preserve intended arrival times and expose overload.

A 10 ms claim should specify its boundary exactly.
The recommended claim boundary is accepted prepared claim to authenticated Ready receipt.
Do not exclude queueing when describing user-observed latency.

## What SOMA should borrow and reject

| Source | Borrow | Reject or constrain |
| --- | --- | --- |
| Firecracker | Minimal devices, jail, per-VM process, snapshot discipline, rate limits | Operator-dependent networking without SOMA-owned policy proof |
| Crosvm | Capability-shaped device authority, per-architecture seccomp, hostile-device thinking | Process per device on the default fast path unless evidence justifies it |
| Kata and Dragonball | Integrated Rust runtime, guest-agent separation, pluggable Host resources | Broad container-orchestrator scope before one SOMA lifecycle works |
| libkrun | Small embedded VMM interface, KVM plus HVF portability, userspace networking option | Assuming VMM and guest share an acceptable security context in multi-tenancy |
| AWS SnapStart | Immutable cached snapshots, restore hooks, uniqueness repair, explicit lifecycle classes | Treating application hooks alone as sufficient for arbitrary hostile workloads |
| SnapFaaS and SEUSS | Layering, working-set focus, snapshot locality | Applying unikernel assumptions to a general OCI Linux guest without proof |

## Concrete changes to the current plan

### Do immediately

1. Fix the portable test gate.
2. Replace forgeable network activation with a guest-session-produced, single-use capability.
3. Authorize the `soma-netd` socket using kernel peer identity plus operation capability.
4. Replace blocking network tool subprocesses with bounded supervision, then move them off the readiness path.
5. Make restore readiness consume an authenticated receipt.
6. Resolve duplicate ADR 0024 and regenerate current snapshot evidence.

### Build next

1. Define the small public `SandboxBackend` lifecycle interface.
2. Implement one deep Instance Lifecycle module behind that seam.
3. Compose real storage, network, jail, VMM, KVM, guest session, and cleanup through it.
4. Make the receipt ledger the source of lifecycle truth and recovery.
5. Run one Instance end to end before adding pools.
6. Run 100 concurrent Instances with injected failures and proven cleanup.

### Optimize after correctness

1. Measure the sequential warm-restore critical path.
2. Move resource creation into sterile prepared bundles without carrying tenant authority.
3. Remove process spawning from the timed path.
4. Add working-set measurement and selective prefault.
5. Add prepared-worker and paused-pool classes.
6. Compare the embedded-control and external-process costs with the same security profile.
7. Admit a performance class only after security and cleanup gates pass.

## Explicit non-goals for the first production milestone

- Live migration.
- GPU and device passthrough.
- Multiple containers inside one VM.
- General-purpose hardware emulation.
- Cross-architecture snapshot compatibility.
- Transparent compatibility with every OCI runtime annotation.
- One networking implementation that pretends macOS proxying and Linux TAP have identical semantics.
- A public 10 ms claim before an admitted KVM cohort exists.

These may become later modules when a real second adapter or product requirement creates a justified seam.

## Final recommendation

Do not redesign SOMA around a different VMM.
Do not add another subsystem.
Deepen the Instance Lifecycle module until one call safely realizes the complete sandbox and one terminal receipt proves its cleanup.

Then optimize the prepared restore path around data locality, sterile resource pools, fresh per-Instance authority, and removal of process creation from the timer.
That path offers the best combination of strong isolation, modular implementation, operational recovery, and a credible route toward 10 ms.
