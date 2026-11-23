# Rust VMM engineering deep dive

- Date: 2026-08-29
- Scope: The Rust VMM inside SOMA, not the surrounding cloud product
- Repository baseline inspected: `5037e8d`
- Status: Source-backed design and implementation guidance, not performance or security certification

For the source-level comparison of obscure Rust VMM repositories, continue with [Rust VMM GitHub hidden gems](rust-vmm-github-hidden-gems.md).
For the expanded census, revised diamond ranking, and source-backed failure atlas covering Alioth, Nanvix, Clone, Visor, Zeroboot, and the broader ecosystem, continue with [Rust VMM GitHub census and failure atlas](rust-vmm-github-census-and-failure-atlas.md).

## Direct answer

SOMA already contains the beginning of a real custom Rust VMM.
It is not Firecracker with different branding and it is not merely orchestration around Docker.

On Linux x86_64, `soma-kvm` currently imports only these rust-vmm foundations:

- `kvm-bindings` for generated KVM ABI structures and constants.
- `kvm-ioctls` for safer ownership wrappers around `/dev/kvm`, VM, and vCPU file descriptors.
- `vmm-sys-util` for eventfd, epoll, and low-level system helpers.

SOMA currently owns its machine construction, memory mapping, PVH loader, CPUID profile, vCPU runner, interrupt routing, event loop, virtio-mmio transport, virtqueues, block device, network device, RNG device, vsock device, snapshot format, snapshot compatibility rules, restore ordering, and guest control integration.

The inspected source contains approximately:

| Area | Rust lines |
| --- | ---: |
| `virtio` | 10,687 |
| generic `snapshot` | 7,200 |
| `x86_64` | 8,628 |
| `arm64` | 3,098 |

That is a custom VMM implementation.
The remaining question is whether its ownership, interfaces, unsafe code, device semantics, restore invariants, and evidence are strong enough to become a production VMM.

## What the VMM actually is

KVM is the kernel mechanism that executes guest CPU instructions using hardware virtualization.
KVM does not build SOMA's machine for it.

The VMM is the userspace program that:

1. Opens `/dev/kvm`.
2. Verifies the KVM interface and required capabilities.
3. Creates one VM descriptor.
4. Creates and maps guest physical memory.
5. Registers memory slots with KVM.
6. Creates the interrupt controller and timer model.
7. Defines the virtual CPU model.
8. Loads the kernel, initramfs, command line, and boot structures.
9. Creates virtual devices and connects their eventfds and irqfds.
10. Creates vCPU descriptors and enters `KVM_RUN`.
11. Handles exits that KVM returns to userspace.
12. Services virtio queues whose descriptors live in guest memory.
13. Pauses, snapshots, restores, resumes, stops, and reclaims the machine.

```text
USER OR HOST RUNTIME
       |
       v
+-----------------------------+
| SOMA VMM                    |
|                             |
| Machine model               |
| Memory map                  |
| vCPU model and run loops    |
| Interrupt routing           |
| Virtio transport and queues |
| Block, net, RNG, vsock      |
| Snapshot and restore        |
+-----------------------------+
       |
       | KVM ioctls
       v
+-----------------------------+
| Linux KVM                   |
| Hardware CPU virtualization |
+-----------------------------+
       |
       v
Guest Linux kernel and workload
```

The Linux KVM documentation defines the descriptor and ioctl model.
It also documents `kvm_run`, `immediate_exit`, memory-region registration, irqfd, guest memory files, and pre-faulting.
[Linux KVM interface](https://docs.kernel.org/virt/kvm/api.html)

## Current SOMA VMM anatomy

### `soma-vmm`

`soma-vmm` is the platform-independent lifecycle and receipt layer.
It owns `Launch`, `Execute`, `Stop`, identities, limits, milestones, cleanup evidence, and failure classification.

Its private `Platform` interface is directionally strong:

```rust
trait Platform: Send {
    fn verify_and_restore(...);
    fn authenticate_repair_and_ready(...);
    fn execute(...);
    fn stop(...);
    fn rollback(...);
}
```

This is a deep interface because a caller asks for lifecycle outcomes without learning KVM registers, memory slots, virtqueues, TAP descriptors, or snapshot sections.
Keep this seam.

### `soma-kvm`

`soma-kvm` is the Linux machine implementation.
Its internal module structure is mostly sensible, but its external interface is too broad.
The crate root re-exports a large set of queue, transport, device, packet, backend, counter, error, constant, and snapshot details.
The source scan found 681 public or crate-public declarations.

That makes `soma-kvm` look like a general virtualization toolkit rather than one deep SOMA Machine module.
It increases compatibility obligations and lets callers bypass machine invariants.

The external interface should expose machine intentions and evidence only.
Virtio descriptors, queue cursors, register offsets, frame parsers, and device constants should be internal implementation details unless a real second adapter needs them.

### `soma-jail`

`soma-jail` should own Host-process containment around the VMM.
It must not know guest-machine semantics.
It should receive an executable, fixed descriptor manifest, resource profile, and typed launch token, then return process ownership and cleanup evidence.

### `soma-hostd`

`soma-hostd` should compose the lifecycle but never manipulate KVM internals.
Its VMM adapter should translate an admitted Instance plan into one `soma-vmm` lifecycle execution.

## The recommended Rust topology

Do not create a crate for every file or concept.
Crates should correspond to real interfaces and trust domains.

```text
crates/
  soma-vmm/                 Public portable lifecycle interface
  soma-kvm/                 Deep Linux KVM Machine implementation
    src/
      machine/              Ownership and typestate
      arch/x86_64/          CPUID, boot, irqchip, PIT, vCPU state
      memory/               Mapping, slots, COW, launch page
      devices/
        transport/          Internal virtio-mmio transport
        queue/              Internal split-queue implementation
        block/
        net/
        rng/
        vsock/
      event/                Bounded epoll reactor
      snapshot/             Versioned state, capture, restore
      unsafe_boundary/      Audited ABI and mapping operations
  soma-jail/                Per-VMM Host process containment
  soma-guest/               Authenticated Host and guest protocol
  soma-guest-agent/         Guest PID 1 and workload supervisor
```

Do not split queue, bus, block, vsock, or snapshot sections into public crates merely because they are internally modular.
They are internal seams of the `soma-kvm` implementation.

## Recommended external `soma-kvm` interface

The interface should be closer to this conceptual shape:

```rust
pub struct KvmEngine { /* host capability profile */ }

impl KvmEngine {
    pub fn inspect_host(&self) -> Result<KvmHostProfile, KvmError>;
    pub fn build_cold(&self, plan: ColdMachinePlan) -> Result<PreparedMachine, KvmError>;
    pub fn restore(&self, plan: RestoreMachinePlan) -> Result<PreparedMachine, KvmError>;
}

impl PreparedMachine {
    pub fn start(self, launch: FreshLaunchAuthority) -> Result<RunningMachine, KvmError>;
}

impl RunningMachine {
    pub fn pause(self, deadline: Deadline) -> Result<PausedMachine, KvmError>;
    pub fn stop(self, deadline: Deadline) -> Result<StoppedMachine, KvmError>;
}

impl PausedMachine {
    pub fn capture(self, target: SnapshotTarget) -> Result<CapturedMachine, KvmError>;
    pub fn resume(self) -> Result<RunningMachine, KvmError>;
}

impl StoppedMachine {
    pub fn cleanup(self) -> Result<MachineCleanupReceipt, KvmError>;
}
```

The exact names can change.
The essential rule is that invalid lifecycle ordering is unrepresentable or rejected inside the module.
No external caller should directly activate a queue, modify CPUID, register an irqfd, or mark a restored machine Ready.

## What SOMA should reuse from rust-vmm

Rust-vmm exists to share the low-level virtualization work that Firecracker, crosvm, Cloud Hypervisor, and other Rust VMMs would otherwise duplicate.
[rust-vmm community](https://github.com/rust-vmm/community)

The project is currently moving crates into a monorepository.
Some former repositories are archived because development moved, not because the crates are abandoned.
[rust-vmm monorepository](https://github.com/rust-vmm/rust-vmm)
[rust-vmm migration tracker](https://github.com/rust-vmm/community/issues/193)

### Adopt and retain

| Crate | Recommendation | Reason |
| --- | --- | --- |
| `kvm-bindings` | Retain | Generated ABI structures are not SOMA differentiation |
| `kvm-ioctls` | Retain | Descriptor ownership and ioctl wrappers reduce unsafe surface |
| `vmm-sys-util` | Retain | Mature eventfd, epoll, terminal, signal, and ioctl helpers |
| `linux-loader` | Retain on ARM64 | Already used and avoids generic loader duplication |

The currently pinned KVM versions match the latest rust-vmm KVM release found during this research.
[rust-vmm KVM releases](https://github.com/rust-vmm/kvm/releases)

### Evaluate through an adapter and differential tests

| Crate | Recommendation | Reason |
| --- | --- | --- |
| `vm-memory` | Strongly evaluate | Mature volatile-memory semantics, region handling, atomic access, and bitmap support address the hardest unsafe-memory concerns |
| `virtio-queue` | Use as an oracle before replacement | Mature descriptor walking and queue semantics, but SOMA needs strict bounds and exact snapshot behavior |
| `virtio-queue-ser` | Compare state coverage | Useful reference for queue serialization, but SOMA snapshot identity may require a stricter format |
| `event-manager` | Benchmark against current loop | General subscriber model may add interface and dispatch cost without value for a fixed device set |
| `seccompiler` | Use for policy tooling or verification | It can reduce BPF construction risk, but the existing phase-specific jail model may justify a custom compiled filter |

Rust-vmm's `vm-memory` specifically separates memory consumers from memory providers and supports volatile shared-memory access and dirty tracking.
[rust-vmm vm-memory](https://github.com/rust-vmm/rust-vmm/tree/main/vm-memory)

Rust-vmm's virtio workspace provides queue, transport, block, console, and vsock primitives, but leaves device backends and event handling to the VMM.
[rust-vmm virtio](https://github.com/rust-vmm/vm-virtio)

### Do not adopt for version 1

| Crate or mechanism | Reason |
| --- | --- |
| Generic ACPI stack | SOMA's initial x86_64 profile uses PVH and a deliberately tiny fixed machine |
| PCI transport | Virtio-mmio is smaller and already part of the current snapshot contract |
| VFIO | Device passthrough creates a much larger attack, migration, and lifecycle surface |
| vhost-user devices | Additional processes and protocols complicate the first fast path |
| General-purpose VMM reference architecture | Useful for learning, not a production serverless design |
| Live migration frameworks | Not required to create and restore one local sandbox |

The rust-vmm reference VMM explicitly describes itself as a small example with minimal glue.
It is not a production architecture to copy wholesale.
[rust-vmm reference VMM](https://github.com/rust-vmm/vmm-reference)

## Guest memory is the most important unsafe interface

### What guest memory means

The guest kernel believes it owns a physical address space.
The VMM maps Host virtual memory and tells KVM which guest physical addresses correspond to that mapping.
Devices then read descriptor tables and payloads from the same memory while guest vCPUs can modify it concurrently.

```text
Guest physical address 0x1000
              |
              v
KVM memory slot translation
              |
              v
Host mapping base + 0x1000
              |
       +------+------+
       |             |
       v             v
  guest vCPU      VMM device thread
  may write       may read or write
```

The KVM interface permits lazily populated userspace memory regions and newer guest-memory-file mappings.
[Linux KVM memory interface](https://docs.kernel.org/virt/kvm/api.html)

### Current SOMA design

`RamMapping` owns an anonymous or adopted `mmap` range.
It uses checked offsets and raw-pointer copies.
It avoids creating long-lived Rust references into guest memory.
This is a thoughtful design.

However, the current comments permit the guest to mutate bytes concurrently while Host code uses `copy_nonoverlapping` for reads and writes.
The exact Rust language soundness of ordinary raw-pointer copies racing with KVM writes must not be assumed.
Volatile access alone also does not automatically make multi-byte structures atomic.

This is an unresolved soundness and concurrency question until one of these is completed:

1. Adopt `vm-memory`'s established volatile abstractions through a SOMA mapping adapter.
2. Obtain a focused unsafe-code review of the custom mapping and prove the permitted compiler and concurrency model.
3. Restrict concurrent accesses and use atomics or stable snapshot copies where a protocol field requires atomicity.

### Required memory rules

- No Rust reference may outlive a checked access to guest-controlled memory.
- Every `GPA + length` calculation uses checked arithmetic.
- Cross-region access is either deliberately supported or consistently rejected.
- Virtio fields that the guest and device update concurrently use the memory ordering required by Virtio, not only convenient Rust ordering.
- Multi-byte guest structures are copied into owned Host values and validated before use.
- Descriptor chains are walked with cycle, depth, total-length, and segment-count bounds.
- A guest mutation between validation and use cannot produce Host memory unsafety.
- Snapshot capture begins only after every vCPU and device worker has acknowledged quiescence.
- Mapping lifetime strictly exceeds every KVM slot, vCPU, device worker, and raw pointer that can touch it.
- The nonsnapshot launch page is a separate memory slot and cannot alias captured memory.

### Required tests

- Property tests over arbitrary region layouts and address ranges.
- Truncation and bit-mutation tests for every guest structure.
- Concurrent guest mutation tests that alter queue fields during validation and execution.
- Loom tests for Host-owned synchronization where Loom can model the state.
- Miri for pure queue and parsing code, while recognizing that Miri cannot model KVM itself.
- Sanitizer builds for live Linux tests.
- Differential tests between the custom memory adapter and `vm-memory` for equivalent operations.
- A specialist unsafe-code review with every `unsafe` block tied to one documented invariant.

## KVM machine ownership

The safe abstraction is not a collection of file descriptors.
It is one ownership tree.

```text
KvmEngine
  |
  +-- Kvm capability profile
  |
  +-- MachineOwner
        |
        +-- VmFd
        +-- Guest memory mappings and registered slots
        +-- irqchip, PIT, routing, irqfds, ioeventfds
        +-- DeviceSet
        +-- vCPU owners and KVM_RUN mappings
        +-- event-loop owner
        +-- launch-page mapping
        +-- cleanup ledger
```

Destruction must proceed in the reverse of reachability:

1. Prevent new external commands.
2. Stop and join guest-session work.
3. Kick every vCPU out of `KVM_RUN`.
4. Stop and join every device worker.
5. Deassign irqfds and ioeventfds.
6. Remove or retire KVM memory slots.
7. Drop vCPU descriptors and mappings.
8. Drop the VM descriptor.
9. Unmap guest RAM and the launch page.
10. Return all external descriptors and cleanup evidence.

The owner must retain enough state to complete this sequence after any partial construction failure.

## Cold machine construction order

Recommended x86_64 order:

```text
Validate immutable Machine profile
       |
Open and verify KVM capability profile
       |
Create VM
       |
Map guest RAM and separate launch page
       |
Register memory slots
       |
Create in-kernel irqchip and PIT
       |
Build deterministic device set and MMIO table
       |
Register irqfds and ioeventfds
       |
Load kernel, initramfs, command line, and PVH structures
       |
Create vCPUs
       |
Apply normalized CPUID and complete boot register state
       |
Start device reactor
       |
Start vCPU threads
```

CPUID must be fixed before a vCPU first runs.
The KVM documentation warns that modifying CPUID after `KVM_RUN` can destabilize the guest.
[KVM CPUID contract](https://docs.kernel.org/virt/kvm/api.html)

SOMA's custom PVH loader is a valid differentiator because it can enforce one small, exact, hostile-input-checked boot profile.
Do not replace it merely to reduce local code if its evidence is stronger than a generic loader.
Use `linux-loader` as a differential oracle for overlapping inputs.

## vCPU architecture

### One Host thread per active vCPU

For the initial design, one Host thread per running vCPU is the clearest and safest model.
Each thread owns one `VcpuFd`, one mapped `kvm_run`, one kick mechanism, and one bounded exit loop.

```text
loop {
    check stop or pause generation
    enter KVM_RUN
    classify exit
    handle only allowlisted exits
    update bounded counters
    stop on fatal or unknown exit
}
```

Do not place multiple vCPUs on one async executor before evidence shows thread count is the limiting resource.
`KVM_RUN` is a blocking ioctl whose ownership and signal behavior are easier to reason about with a dedicated thread.

### Kicking vCPUs

Use `kvm_run.immediate_exit` with a targeted signal or another documented KVM mechanism.
The kernel documentation notes that `immediate_exit` avoids the less scalable signal-mask approach in the common kick design.
[KVM `immediate_exit`](https://docs.kernel.org/virt/kvm/api.html)

Required properties:

- Kick latency has a measured upper bound under load.
- Pause requires an acknowledgement from every vCPU.
- No snapshot begins while any vCPU can still mutate memory or device state.
- Stop and pause generations cannot be confused after a rapid resume.
- An unknown KVM exit is fatal and bounded, never silently ignored.
- Every thread returns typed terminal evidence and is joined.

### CPU profile

The CPU profile is part of Generation compatibility.
It includes CPUID, MSRs, KVM capabilities, topology, mitigations, clock expectations, and Host kernel constraints.

Firecracker normalizes CPUID and uses CPU templates to present a stable guest model across a fleet.
[Firecracker CPUID normalization](https://github.com/firecracker-microvm/firecracker/blob/main/docs/cpu_templates/cpuid-normalization.md)

SOMA should define a smaller versioned CPU profile rather than passing through every Host feature.
Every admitted Host proves it is a superset of that profile.
Every snapshot binds the exact profile identity.

## Interrupt architecture

For the x86_64 version 1 Machine:

- Use the in-kernel irqchip.
- Use the in-kernel PIT only because the chosen kernel boot contract requires it.
- Allocate fixed, versioned GSIs for the minimal device set.
- Use irqfd to inject device interrupts without returning through a central userspace dispatcher.
- Use ioeventfd for guest queue notification where the transport permits it.
- Bind routing and device slots into the snapshot compatibility profile.

KVM irqfd connects an eventfd directly to a guest interrupt line and supports resampling for level-triggered interrupts.
[KVM irqfd](https://docs.kernel.org/virt/kvm/api.html)

Every eventfd must have one owner, one registration record, and one deassignment path.
Snapshot restore recreates Host descriptors and routes.
It never serializes numeric Host file descriptors.

## Virtio transport and queues

### Why virtio is dangerous code

Virtio descriptors, indices, flags, addresses, lengths, and packets are controlled by the guest.
A malicious guest can change them concurrently, create cycles, point outside RAM, overflow lengths, reuse heads, report impossible cursor movement, or produce infinite work.

Rust prevents many Host memory errors only after the VMM has converted those guest-controlled numbers into safe operations correctly.

The Virtio 1.3 specification defines queue layout, notification suppression, descriptor formats, device status, and memory-ordering requirements.
[Virtio 1.3](https://docs.oasis-open.org/virtio/virtio/v1.3/csd01/virtio-v1.3-csd01.pdf)

### Current custom queue

SOMA's queue implementation already has valuable properties:

- Split queue only.
- Fixed maximum sizes.
- Checked queue geometry.
- Bounded descriptor-chain walking.
- Avail-index overrun rejection.
- Typed violation counters.
- Snapshot-visible cursors.
- Work budgets in the device reactor.

Those are reasons not to perform a blind rewrite.

### What must be added

- Differential property tests against `virtio-queue` for every shared supported case.
- A formal statement of the memory-ordering relationship between guest writes, Host reads, used-ring writes, and interrupt signaling.
- Tests that mutate descriptor and ring fields between every validation and use step.
- Feature-negotiation tests proving unsupported bits never alter behavior.
- Reset tests from every intermediate driver-status state.
- Notification suppression and lost-wakeup tests.
- Cursor wraparound tests covering the full 16-bit space.
- Fuzz targets for queue layout, chain walking, request parsing, and restore state.
- A per-notification bound on descriptors, bytes, backend operations, and interrupts.

### Adopt or keep custom

Do not decide by line count.
Run the following comparison:

| Gate | Custom queue | rust-vmm queue |
| --- | --- | --- |
| Hostile-input rejection | Measure | Measure |
| Snapshot state completeness | Measure | Measure with `virtio-queue-ser` |
| Supported feature surface | Prefer smaller | Disable unused features |
| Unsafe surface | Audit | Audit dependency |
| Performance | Same harness | Same harness |
| Fuzz maturity | Compare corpora | Compare upstream tests |
| Maintenance burden | SOMA-owned | Upstream plus adapter |

Keep the custom queue only if it wins on the required SOMA contract after equivalent testing.

## Device model

### Version 1 device set

SOMA needs only:

- Virtio block for immutable root and private writable overlay.
- Virtio net for production Linux networking.
- Virtio RNG for fresh entropy.
- Virtio vsock for the authenticated control session.
- Minimal serial only for bounded diagnostic boot evidence, disabled or strictly drained in production.

No GPU, sound, USB, PCI, ballooning, filesystem sharing, VFIO, or arbitrary device passthrough belongs in the first production profile.

### Device interface

The existing separation between the common transport and device-specific behavior is good.
The `VirtioDevice` trait should remain internal.

Device state must separate:

```text
Serializable guest-visible state
  features, status, queue geometry, cursors, interrupt state,
  device configuration, protocol state

Recreated Host resources
  files, TAP, eventfds, irqfds, epoll registrations,
  buffers, threads, locks, sockets
```

Host resources are never serialized as raw descriptor numbers or pointers.

### Block

- Immutable root opens read-only by descriptor.
- Writable overlay is one private head with declared size and I/O class.
- Every request validates sector arithmetic, direction, segment total, and file bounds.
- Flush semantics are explicit.
- Discard and write-zeroes remain unsupported until designed.
- Host I/O has deadline and cancellation behavior.
- One guest cannot monopolize the device thread with large chains.

### Network

- The VMM receives an already authorized TAP descriptor.
- It cannot create TAP devices or modify Host firewall state.
- Frame size, header, checksum features, queue work, and buffering are bounded.
- Link remains down until the Host and guest repair contract permits activation.
- Snapshot state excludes stale Host network identity and active connection authority.

### RNG

- Entropy comes from an opened Host entropy source or kernel interface.
- Requests and retained buffers are bounded.
- Snapshot state contains no reusable entropy bytes.
- Restore requires fresh Host entropy before Ready.

### Vsock

- CID is fresh per Instance.
- Snapshot restore resets transport connections and queue state as specified.
- Existing sessions never survive as authenticated sessions.
- Credit arithmetic is checked and bounded.
- The control listener accepts a new connection only after fresh launch authority is installed.

Firecracker and Cloud Hypervisor both reset vsock behavior across snapshot restore to prevent stale half-open state.
[Firecracker snapshot support](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md)
[Cloud Hypervisor releases](https://github.com/cloud-hypervisor/cloud-hypervisor/releases)

## Event reactor

SOMA's fixed epoll reactor is a reasonable fast-path design.
One device thread avoids a process or thread per device and keeps the initial footprint small.
The existing work and pass budgets are valuable fairness mechanisms.

Keep the custom reactor unless measurement proves that `event-manager` improves correctness or performance.
The generic rust-vmm event manager is built around subscribers and epoll, which is useful for variable device sets but can create unnecessary interface surface for SOMA's fixed machine.
[rust-vmm event manager](https://github.com/rust-vmm/rust-vmm/tree/main/event-manager)

Required reactor improvements:

- No global mutex may remain held across Host file or TAP I/O.
- Each wake has descriptor, byte, and wall-time budgets.
- Readiness for one device cannot starve guest control or shutdown.
- Backend blocking is eliminated or moved to bounded workers.
- Stop is always observable even under permanently ready guest queues.
- Lost eventfd writes and counter saturation have defined behavior.
- Reactor reports separate queue delay, service time, backend time, and interrupt time.
- Snapshot quiescence receives an acknowledgement from the reactor after all in-flight work reaches a safe point.

An io_uring rewrite is not justified merely because io_uring is newer.
It adds registration, cancellation, completion, and seccomp complexity.
Adopt it only if block or network backend measurements prove epoll plus synchronous I/O is the limiting path.

## Snapshot architecture

### Snapshot is a protocol, not a struct dump

A snapshot consists of:

- Guest memory bytes or a memory-layer reference.
- KVM VM state.
- Per-vCPU architectural state.
- Interrupt-controller, timer, clock, and routing state.
- Virtio transport, queue, and device state.
- Compatibility identity.
- External resource requirements.
- Integrity and provenance metadata.

Firecracker stores guest memory separately from versioned microVM state and requires external block, TAP, and vsock resources to be recreated.
It explicitly warns that Host kernel differences can change KVM-state semantics.
[Firecracker snapshot versioning](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/versioning.md)

### Capture ordering

```text
Reject new guest operations
       |
Authenticate and complete guest quiesce
       |
Guest flushes writable storage
       |
Kick and acknowledge all vCPUs paused
       |
Drain and acknowledge device reactor quiesced
       |
Capture device and queue state
       |
Capture interrupt, clock, VM, and vCPU state
       |
Capture or seal memory object
       |
Capture storage head identity
       |
Cross-validate all sections
       |
Write manifest last
```

Any failure before the manifest produces no ready snapshot.

### Restore ordering

```text
Verify snapshot envelope and compatibility
       |
Open immutable memory and storage artifacts by descriptor
       |
Create VM and map memory privately
       |
Recreate irqchip, PIT, routing, and device objects
       |
Create vCPUs
       |
Restore CPU model and architectural state
       |
Restore VM clock, irqchip, LAPIC, events, and routes
       |
Restore transport, queue, and device state
       |
Create fresh eventfds, irqfds, ioeventfds, TAP, and backends
       |
Attach fresh nonsnapshot launch authority
       |
Start reactor and vCPUs
       |
Reset vsock and other restore-hostile state
       |
Authenticate guest and repair cloned state
       |
Publish Ready from authenticated evidence
```

Exact KVM ioctl ordering is architecture and kernel dependent.
It must be encoded in one versioned restore module and exercised on the admitted Host matrix.

Cloud Hypervisor's recent restore fixes illustrate the subtlety: it restored KVM clock before vCPU resume, reset vsock connections, signaled activated queues, and handled sparse memory and userfaultfd details.
[Cloud Hypervisor releases](https://github.com/cloud-hypervisor/cloud-hypervisor/releases)

### Snapshot format policy

- Snapshot format version is independent from SOMA binary version.
- Every section has a type, version, bounded length, digest, and required or optional status.
- Unknown required sections fail closed.
- Duplicate sections fail closed.
- Arithmetic and allocation happen only after bounds validation.
- Snapshot compatibility binds architecture, CPU profile, KVM feature profile, kernel, device set, device versions, queue features, memory layout, page size, and security policy.
- Host descriptors, paths, pointers, locks, and thread state are not serialized.
- Deserialization produces validated owned values before any ioctl executes.
- Restore failure unwinds every partially recreated Host resource.

Do not automatically adopt a generic serde struct format.
Firecracker's own snapshot history shows that encoding choices and state changes can force major compatibility breaks.
[Firecracker snapshot versioning](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/versioning.md)

## Unsafe Rust strategy

Rust reduces memory-unsafety risk but does not make KVM, mmap, ioctls, signal handlers, shared guest memory, or C ABIs safe automatically.

The scan found unsafe code in memory mapping, snapshot mapping and capture, KVM state bindings, launch-page handling, vCPU kicking, signal handling, and ARM64 machine code.
These are expected locations.

The correct goal is not zero unsafe code.
It is one small, reviewable unsafe perimeter.

### Required unsafe rules

- Move raw mappings, signal operations, ioctl structures, and pointer conversions under `unsafe_boundary/` or an equivalently obvious internal namespace.
- Every unsafe function states caller obligations in `# Safety` documentation.
- Every unsafe block cites the exact invariant that makes it valid.
- Safe wrappers own the associated descriptor or mapping lifetime.
- No public interface exposes raw pointers, unowned descriptors, or KVM binding structures.
- FFI padding and reserved fields are initialized deterministically.
- Snapshot decoders never deserialize directly into kernel ABI structures without field validation.
- Signal handlers perform only async-signal-safe work.
- `Send` and `Sync` implementations receive standalone soundness tests and review.
- CI denies new unsafe locations outside the allowlist.

## Security architecture inside the VMM

### Trust model

Treat these as hostile:

- Guest RAM.
- Virtio descriptors and indices.
- Device requests and packets.
- Kernel, initramfs, root, overlay, and snapshot bytes until verified.
- Host-provided configuration until validated.
- Restored state from every previous binary version.
- Backend errors, short I/O, cancellation, and descriptor reuse.

Treat these as trusted only after proof:

- Opened and digest-verified Generation artifacts.
- The admitted Host KVM profile.
- Descriptor provenance from the authenticated launcher.
- Per-Instance launch authority from the lifecycle owner.

### VMM jail

The VMM should receive only:

- `/dev/kvm` or a VM descriptor arrangement proven safe for the design.
- Kernel, initramfs, memory, block, TAP, entropy, control, and evidence descriptors required by the exact profile.
- No filesystem path resolution after execution begins.
- No network-administration capability.
- No mount or namespace mutation capability.
- No ability to open arbitrary Host files.

Firecracker's production design similarly relies on seccomp, namespaces, cgroups, dropped privileges, and a jailer around the VMM.
[Firecracker production Host setup](https://github.com/firecracker-microvm/firecracker/blob/main/docs/prod-host-setup.md)

## Testing a custom Rust VMM

### Unit and property tests

- Checked arithmetic and layouts.
- CPUID normalization.
- Kernel and snapshot parsers.
- Queue geometry and descriptor chains.
- Every device request parser.
- Device reset and restore state.
- Typestate transitions.
- Cleanup after every injected construction failure.

### Differential tests

- SOMA queue against rust-vmm `virtio-queue`.
- SOMA PVH loader against `linux-loader` for supported valid images.
- SOMA KVM state capture against read-back from the same KVM object.
- SOMA snapshot round trip against fresh cold-boot behavior.
- Custom event reactor against a deterministic in-memory scheduler model.

### Fuzz targets

- ELF and PVH notes.
- Kernel command line.
- Snapshot envelope and every section.
- KVM-state records.
- Queue layout and descriptor chains.
- Block requests.
- Ethernet frames and virtio-net headers.
- Vsock packets, credit accounting, and connection state.
- MMIO reads and writes of every width and offset.
- Device-state restore.

Fuzzing must include stateful operation sequences, not only isolated byte parsers.
A driver can legally or maliciously perform reset, feature negotiation, partial queue configuration, activation, traffic, pause, restore, and reset in many orders.

### Live KVM tests

- Every admitted kernel and Host profile.
- Intel and AMD separately.
- Supported kernel version matrix.
- Repeated cold boot and restore.
- Hostile guest driver.
- Forced vCPU exits.
- Device backend hangs and short I/O.
- Snapshot during queue pressure.
- Restore with incompatible CPUID, MSR, page size, device, and kernel profiles.
- Kill VMM at every lifecycle milestone and reconcile all Host resources.
- Sanitizers and kernel lock or fault injection where practical.

### Security review

Before production admission:

- Independent unsafe-code review.
- Independent virtio and snapshot review.
- Seccomp trace and policy review using the real VMM binary.
- Threat modeling for every descriptor entering the jail.
- Coverage-guided fuzzing with retained corpora and published duration.
- Dependency advisory and provenance checks.
- Reproducible release artifacts and SBOM.

## Performance architecture

### What can actually make this VMM fast

- One fixed machine profile avoids general-purpose discovery and configuration.
- PVH avoids BIOS and firmware boot.
- Virtio-mmio avoids PCI enumeration and configuration.
- A single fixed device reactor avoids process-per-device startup.
- irqfd and ioeventfd avoid unnecessary userspace exits.
- Snapshot memory uses private COW mapping rather than full copying.
- Prepared eventfds, cgroups, network namespaces, TAPs, and storage heads move Host setup off the timer.
- The readiness working set is local and selectively prefaulted.
- A small static guest agent avoids systemd and general userspace boot.
- Fresh authority is one fixed launch page rather than a metadata service.
- The guest performs a bounded repair path and one fixed readiness probe.

### What will destroy the 10 ms tail

- Remote snapshot or OCI reads.
- Full guest-memory copy.
- Page faults across a large unmeasured working set.
- Spawning `nft`, `conntrack`, formatters, or other tools.
- Creating cgroups, namespaces, and VMM processes on the timed path.
- Global Host locks.
- One device holding a shared mutex during blocking I/O.
- Guest output or packet floods without work budgets.
- CPU migration across NUMA nodes.
- Memory reclaim, swap, or Host pressure.
- Synchronous evidence writes before Ready.
- Closed-loop benchmarks that hide queueing.

### Required VMM metrics

Record monotonic nanoseconds for:

- Open and verify artifacts.
- Map memory.
- Register KVM slots.
- Create platform devices.
- Create vCPUs.
- Restore each state class.
- Register event routes.
- Start reactor.
- Enter first `KVM_RUN`.
- First guest exit or signal.
- Launch-page consumption.
- Guest authentication.
- Repair completion.
- Readiness completion.
- Reactor queue delay and service time per device.
- vCPU exit count and time by reason.
- Page faults and major faults during readiness.
- Pause and kick latency.
- Cleanup duration and residual resources.

Never emit secrets, guest payloads, raw memory, or unbounded labels in metrics.

## Concrete assessment of current SOMA choices

### Good choices

- Rust with `unsafe_code` denied by default.
- Narrow target gating for KVM.
- Pinned KVM and system dependencies.
- Custom minimal PVH profile.
- Separate nonsnapshot launch page.
- Fixed virtio-mmio device set.
- Bounded queue service and typed violations.
- Device-specific snapshot state separated from Host handles.
- Dedicated vCPU and device threads.
- Explicit pause and teardown stages.
- Authenticated guest protocol above vsock.
- Candidate and certification distinction outside the VMM.

### Choices requiring proof

- Custom guest-memory abstraction instead of `vm-memory`.
- Custom virtqueue instead of `virtio-queue`.
- Custom snapshot codec and KVM-state bindings.
- One shared device mutex around the entire device bus.
- One device reactor for block, net, RNG, vsock, and Host control.
- Public export of internal queue and device details.
- Test-only naming around the most complete `SandboxMachine` path.
- Separate older `KvmMachine` abstraction that may duplicate the real machine ownership model.

### Likely architectural corrections

1. Make `soma-kvm` a deep module and hide nearly all current re-exports.
2. Select one production Machine owner and remove duplicate shallow machine abstractions.
3. Put all raw memory and ABI operations behind one unsafe perimeter.
4. Build a `vm-memory` compatibility adapter and differential suite before deciding whether to replace custom memory.
5. Build a `virtio-queue` differential harness before deciding whether to retain the custom queue.
6. Remove shared-bus locks from blocking backend I/O.
7. Make authenticated guest readiness the only transition that can release execution and network capabilities.
8. Exercise the real `soma-vmm` binary inside `soma-jail` before refining seccomp further.

## Implementation tickets

### RVMM-01: Contract the `soma-kvm` interface

Inventory every external caller and replace low-level re-exports with Machine plans, typestates, terminal evidence, and errors.
Keep device and queue interfaces private.

Acceptance:

- No external crate names a virtqueue, MMIO register, KVM binding, eventfd, or device parser.
- Cold build, restore, start, pause, capture, resume, stop, and cleanup remain testable through the Machine interface.

### RVMM-02: Guest-memory soundness decision

Implement a `vm-memory` adapter and compare it with `SharedRam` under unit, property, differential, fuzz, sanitizer, and live KVM tests.
Obtain an independent unsafe review.

Acceptance:

- One written soundness model covers guest concurrency, raw copies, volatile access, atomics, mapping lifetime, and snapshot quiescence.
- Every unsafe memory operation maps to that model.

### RVMM-03: Virtqueue differential campaign

Run identical generated operation traces against SOMA and rust-vmm queues.
Compare acceptance, rejection, cursor movement, used-ring publication, notification behavior, and restore state.

Acceptance:

- Every behavioral difference is either corrected or documented as a deliberate stricter SOMA contract.
- Stateful fuzzing runs continuously with retained corpora.

### RVMM-04: Single production Machine owner

Unify or clearly separate `KvmMachine`, x86_64 `Machine`, and `SandboxMachine` so there is one production ownership tree.

Acceptance:

- Every descriptor, mapping, thread, route, and slot has one owner.
- Failure injection at every construction step returns Host resources to baseline.

### RVMM-05: Reactor lock and blocking-I/O audit

Measure lock hold time and backend blocking for every device path.
Refactor so guest-controlled work cannot stall stop, control, or unrelated devices.

Acceptance:

- Every wake and backend operation has declared work and time bounds.
- Stop and pause meet their deadlines during hostile block, network, and vsock traffic.

### RVMM-06: Restore-order state machine

Encode capture and restore ordering as explicit typestates and one architecture-specific state machine.

Acceptance:

- Every KVM, interrupt, clock, vCPU, transport, queue, and device state has a defined capture and restore point.
- Permutation tests reject invalid ordering.
- Current Host matrices pass repeated restore without drift.

### RVMM-07: Real VMM jail integration

Launch the real production VMM with its exact descriptor manifest inside `soma-jail`.

Acceptance:

- Seccomp is derived from real traced and reviewed behavior.
- The VMM cannot open paths, administer networking, mutate namespaces, or obtain undeclared descriptors.
- Crash cleanup is complete.

### RVMM-08: Hostile guest driver

Build a tiny guest kernel module or userspace driver capable of malformed MMIO, queue, block, net, RNG, and vsock sequences.

Acceptance:

- No panic, hang, unbounded allocation, Host memory error, lifecycle bypass, or cleanup leak.
- Every violation is bounded and observable.

### RVMM-09: Fast restore profiling

Instrument the complete restore path and collect page-fault, vCPU-exit, reactor, and readiness profiles.

Acceptance:

- The measured critical path identifies every operation above 100 microseconds.
- The 10 ms budget is assigned to measured steps rather than estimates.

### RVMM-10: Production VMM admission

Run the integrated VMM through correctness, compatibility, hostile-input, failure-recovery, density, and performance gates.

Acceptance:

- One signed evidence bundle binds source, binary, Host, kernel, microcode, Machine profile, Generation, snapshot, configuration, raw samples, failures, and cleanup.
- The VMM receives production-admitted status only after independent review.

## Exact recommended order

```text
RVMM-01  Contract the interface
    |
    +----> RVMM-02  Resolve memory soundness
    |
    +----> RVMM-03  Prove queue semantics
    |
    v
RVMM-04  Establish one Machine owner
    |
    +----> RVMM-05  Bound reactor behavior
    |
    +----> RVMM-06  Prove restore ordering
    |
    v
RVMM-07  Run the real VMM in the jail
    |
    v
RVMM-08  Attack it with a hostile guest
    |
    v
RVMM-09  Profile and optimize restore
    |
    v
RVMM-10  Admit the production VMM
```

RVMM-02, RVMM-03, and RVMM-04 are more important than adding another virtual device.

## Final recommendation

Keep building the custom Rust VMM.
The current code already owns enough of the machine to justify that direction.

Do not rewrite mature rust-vmm foundations.
Continue using its KVM bindings, ioctl wrappers, and system utilities.
Evaluate `vm-memory` and `virtio-queue` through adapters and differential evidence rather than ideology.

SOMA's differentiated implementation should remain:

- The minimal fixed Machine profile.
- The snapshot and restore contract.
- Fresh per-Instance authority.
- The authenticated guest repair path.
- The bounded event and device behavior.
- The prepared-resource fast path.
- The lifecycle and cleanup evidence.

The next most important engineering work is not more VMM breadth.
It is proving that the custom guest-memory, virtqueue, Machine ownership, and restore state are sound under a malicious guest and remain fast under the exact production lifecycle.
