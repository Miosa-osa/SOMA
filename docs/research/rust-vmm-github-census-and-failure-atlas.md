# Rust VMM GitHub census and failure atlas

- Date: 2026-08-29
- Scope: Public GitHub repositories related to custom Rust VMMs, microVMs, hardware-isolated sandboxes, snapshot runtimes, and supporting hypervisor backends
- Search coverage: Repository metadata search and code search across more than 35 query families, followed by source inspection of more than 40 repositories
- Status: Broad public-source census, not a mathematical enumeration of every GitHub object and not a security certification

## Honest scope

GitHub cannot provide a provably complete list of every relevant repository.
Search indexing can omit private repositories, deleted repositories, unindexed branches, code outside the default branch, projects whose language classification is not Rust, and repositories described with unrelated terminology.
The GitHub Search API also caps accessible results for a query.

This census therefore uses overlapping searches instead of claiming impossible completeness.
It includes zero-star projects, archived repositories, wrappers, forks, experiments, teaching VMMs, and projects whose README overstates what the code currently implements.
Every high-value conclusion below comes from source inspection at a pinned commit rather than star count or marketing copy.

## Search families

Repository searches covered:

- `VMM language:Rust`
- `KVM language:Rust`
- `microvm language:Rust`
- `hypervisor language:Rust`
- `HVF language:Rust`
- `Hypervisor.framework language:Rust`
- `WHP language:Rust`
- `virtio VMM language:Rust`
- `snapshot VMM language:Rust`
- `sandbox microvm language:Rust`
- `userfaultfd microvm language:Rust`
- `deterministic VMM language:Rust`
- `rust-vmm language:Rust`
- `Firecracker fork language:Rust`
- `libkrun language:Rust`

Code searches covered:

- `kvm-ioctls` and `kvm-bindings` Cargo dependencies.
- `vm-memory` and `virtio-queue` Cargo dependencies.
- `KVM_CREATE_VM` and `KVM_RUN` call sites.
- `KVM_GET_DIRTY_LOG` call sites.
- `hv_vm_create` call sites.
- `WHvCreatePartition` call sites.
- `userfaultfd` combined with snapshot terminology.
- Raw `KVM_CREATE_VM`, `KVM_SET_USER_MEMORY_REGION`, and `KVM_GET_DIRTY_LOG` constants.
- Raw `VHOST_SET_OWNER` and `VIRTIO_F_VERSION_1` constants.
- Fully qualified `kvm_ioctls::VmFd` and `vm_memory::GuestMemoryMmap` types.
- Apple `hv_vm_create` and Windows `WHvCreatePartition` entry points.

The second adversarial pass deliberately searched implementation fingerprints rather than project descriptions.
This found one-star and zero-star projects that ordinary `VMM` and `microVM` repository searches missed, including Amber, plyvm, and Barista.
It also found Tarit, AgentENV, SigmaOS, Aleph, Yobo, FCVM, and a tenant-networking libkrun fork for classification and source inspection.

The overlapping code searches found projects that repository descriptions did not identify as VMMs.
They also exposed false positives such as software keyboard-video-mouse tools, QEMU management panels, dependency mirrors, and ordinary container runtimes.

## Revised answer: the diamond is a set

The expanded census changes the earlier single-project answer.
There are four different diamonds because each solves a different layer well.

| Diamond | Repository | Best idea |
| --- | --- | --- |
| Minimal backend boundary | [Dillo](https://github.com/pichi-vm/dillo/tree/1fc2eb72862c1abfe921eaee6f5adf4e128eddb2) | Tiny host-neutral Machine, Memory, CpuState, and Cpu traits over separate KVM, HVF, and WHP crates |
| Complete low-level type-2 VMM | [Alioth](https://github.com/google/alioth/tree/0fd12118c74b8d3d35a92e331e56df369f8abac7) | From-scratch KVM and HVF interfaces, split and packed virtqueues, confidential computing, vhost-user, and clean subsystem modules |
| Co-designed density architecture | [Nanvix](https://github.com/nanvix/nanvix/tree/65bac84d1019e219c753c725451e4700ca37d97f) | Custom microVM plus guest microkernel and host macro-kernel that remove ordinary device emulation from the guest boundary |
| Warm-fork mechanism | [Clone](https://github.com/unixshells/clone/tree/a9525154846e709bd46a7aeb64ceb1fb43547ee2) | Template VMs, private snapshot mappings, per-instance identity injection, incremental memory, KSM, and ballooning |
| Cross-platform disposable microVM | [Amber](https://github.com/lupodevelop/amber/tree/54cebedae733633ceb9f633b8f99c349d81e941e) | One backend-neutral ARM64 machine core over Apple HVF and Linux KVM, including OCI input, software GIC snapshots, warm CoW workers, vsock exec, and userspace networking |

Panorama remains the most useful obscure dirty-reset test reference.
Vibemon remains the broadest mechanism quarry.
Hyperlight remains the strongest deliberately narrow function-VM reference.

SOMA should combine lessons from these projects rather than adopt any repository wholesale.
For direct judgments against SOMA's current modules and a sequenced adoption plan, see the [competitive module adoption audit](competitive-module-adoption-audit.md).

## Source-inspected candidate inventory

### Custom or substantially custom VMMs

| Repository | Pinned commit | Classification | Most useful lesson | Main caution |
| --- | --- | --- | --- | --- |
| Dillo | [`1fc2eb7`](https://github.com/pichi-vm/dillo/tree/1fc2eb72862c1abfe921eaee6f5adf4e128eddb2) | KVM, HVF, WHP VMM | Small backend-neutral traits and separate device crates | Young project with no production sandbox or snapshot proof |
| Alioth | [`0fd1211`](https://github.com/google/alioth/tree/0fd12118c74b8d3d35a92e331e56df369f8abac7) | KVM and HVF VMM | Low-level hypervisor abstraction, memory buses, packed and split queues | General machine scope is broader than SOMA's minimal first profile |
| Vibemon | [`99d323d`](https://github.com/stencil-hq/vibemon/tree/99d323dafb697ac60a33a6544a296ee37494718b) | KVM, HVF, WHP sandbox VMM | Snapshot validation, delta memory, paging, and lifecycle tests | Multi-thousand-line god files and a very broad product repository |
| Nanvix | [`65bac84`](https://github.com/nanvix/nanvix/tree/65bac84d1019e219c753c725451e4700ca37d97f) | Custom VMM and operating system | Co-design the guest and VMM to remove devices and lower density cost | Not a normal Linux OCI environment and snapshot external state is incomplete |
| Clone | [`a952515`](https://github.com/unixshells/clone/tree/a9525154846e709bd46a7aeb64ceb1fb43547ee2) | KVM VMM | CoW template fork, KSM, balloon, incremental state | Snapshot and migration paths contain fail-open state handling |
| Visor | [`fd43700`](https://github.com/developerinlondon/visor/tree/fd43700158659941c622276c83366a94ebf7bc63) | In-process KVM VMM | Direct in-process VM calls and internal switching | One process holds many tenants, increasing crash and memory-corruption blast radius |
| Ignition | [`2bc5272`](https://github.com/vadika/ignition/tree/2bc5272aabcd4ba5f7c65824eedcabd3b6b4a61d) | Apple HVF VMM | HVF snapshot and dirty tracking, macOS Seatbelt | macOS research path does not certify Linux KVM behavior |
| Hyperlight | [`b9266a8`](https://github.com/hyperlight-dev/hyperlight/tree/b9266a8e61a5f9636bf64dc03dfaaad7789f28a6) | Embedded function VMM | Narrow guest ABI and disciplined snapshot compatibility | Not a general Linux agent machine |
| deterministic-vmm | [`0b5ba86`](https://github.com/hashbrowncipher/deterministic-vmm/tree/0b5ba868ddaa6a3b0bd110cfb0a4fbe63009ae06) | Deterministic KVM research VMM | Instruction-count time and exact interrupt placement | Requires a custom Host kernel and is not the default fast path |
| Panorama | [`221699a`](https://github.com/00xc/panorama/tree/221699a87fef503927330327d9dfe9a68f98e5de) | Snapshot fuzzing VMM | Merge CPU and device dirty state, then restore only dirty spans | Repository declares itself broken and lacks production status |
| ai-vmm | [`50b8c88`](https://github.com/SO2304/ai-vmm/tree/50b8c88993bca7ff74ba1b3aa73bdab2c4c425a3) | KVM VMM plus agent control | Kani proofs for bounded arithmetic and validation | Local proof harnesses do not prove the complete VMM |
| Microcosm | [`104cf14`](https://github.com/mosmeh/microcosm/tree/104cf14ef22d413bee0210eb72d97c1dbd52a6d7) | Minimal KVM VMM | Clear direct boot using several boot protocols | Teaching scope, minimal devices, no production lifecycle |
| Kitsune | [`146ce16`](https://github.com/lapla-cogito/kitsune/tree/146ce16a600ecdc51a1e5a25c17d949e1310d0fc) | KVM VMM | Understandable multi-vCPU, block, net, ACPI, and reset path | Limited hardening and compatibility evidence |
| FerrumVM | [`a4ba316`](https://github.com/milosilo-dev/FerrumVM/tree/a4ba316eb20bf356b5ee59be6cb5ca6cd4a671e8) | KVM VMM with firmware | Reset vector through custom firmware to Linux | Hobby scope and wrong boot path for SOMA's latency target |
| MiniHype | [`57215bf`](https://github.com/64bit/miniHype/tree/57215bf7b0e38bc71e71452cc50a9b669fb4b963) | KVM and HVF example | Tiny comparison of Host virtualization APIs | Demonstration rather than sandbox architecture |
| alvm | [`ea6d3a1`](https://github.com/mathetake/alvm/tree/ea6d3a125d4c34653c2c936fea7bafba19de0eb5) | Kernel-less HVF runtime | Run static Linux AArch64 ELF through trapped syscalls | Syscall emulation becomes a large compatibility boundary |
| Zeroboot | [`87ca9c0`](https://github.com/zerobootdev/zeroboot/tree/87ca9c018a9c2a343ece768eec508e16497753f9) | Firecracker snapshot restorer using raw KVM | Compact CoW KVM restoration | Prototype ignores critical restore errors and times a narrow boundary |
| Amber | [`54cebed`](https://github.com/lupodevelop/amber/tree/54cebedae733633ceb9f633b8f99c349d81e941e) | ARM64 HVF and KVM microVM | Shared machine core, software GIC snapshotability, OCI-to-squashfs path, warm CoW workers, vsock exec, and rootless userspace network | Five-run HVF headline sample, no real KVM performance result, warning-only GIC restore failures, and several god files |
| plyvm | [`9f41d84`](https://github.com/iluxav/plyvm/tree/9f41d84e33df89058c6307841c3130be3cbdbfc9) | Small Apple HVF Linux VMM | Stepwise teaching path from one vCPU to virtio block, userspace networking, and OCI-like images | Zero-star educational project with pervasive unwraps and no production containment proof |
| Tarit | [`81757b5`](https://github.com/instavm/tarit/tree/81757b54fee03fc75c59c73af06da392c8aa164e) | x86_64 KVM VMM and orchestrator | Separate wire-contract crate, one process per VM, explicit device-DMA dirty tracking, live pre-copy bounds, and reflink-aware snapshot policy | Large young codebase whose sub-15ms source comment is not a retained end-to-end benchmark result |

### Existing-VMM runtimes and forks

| Repository | Pinned commit | Actual machine mechanism | Useful lesson | Do not confuse with |
| --- | --- | --- | --- | --- |
| forkd | [`e2fd1a6`](https://github.com/deeplethe/forkd/tree/e2fd1a6e12522b05c95d85953f6b97b8e1fcaa1e) | Firecracker fork | userfaultfd write-protected live branch and CoW fanout | A from-scratch VMM |
| k7d | [`750e2ab`](https://github.com/Katakate/k7d/tree/750e2abe35ea2e52d9c66b48569b211b5f0a0778) | Rust KVM implementation and Kubernetes integration | Large integration surface and VM-backed cluster cloning | A small minimal sandbox engine |
| Hephaestus | [`5d7f031`](https://github.com/hephaestus-vm/hephaestus/tree/5d7f03166963501a863267e31bf56195b9598728) | Firecracker compatibility over Apple frameworks with vendored Firecracker | Compatibility matrix and API mapping | Linux KVM production proof |
| ArcBox | [`55b384b`](https://github.com/arcboxlabs/arcbox/tree/55b384b9193d9e564b33efe208dcc7ad5d63b0ff) | Large custom macOS runtime and VMM | End-to-end OCI, Docker, machine, and sandbox product integration | A small reference architecture |
| microsandbox | [`288ef7c`](https://github.com/superradcompany/microsandbox/tree/288ef7c89fe3048abff44521db2ef5ec330e4b1c) | Existing virtualization components | Local-first developer API | A custom KVM machine core |
| mvm | [`f91c7c4`](https://github.com/tinylabscom/mvm/tree/f91c7c4bdb5af56c59ee9ce2b128893704253e64) | In-house HVF on new macOS, libkrun on older macOS, Firecracker on Linux | No-NIC egress broker and signed execution plans | One portable VMM implementation |
| smolvm | [`5a43fca`](https://github.com/smol-machines/smolvm/tree/5a43fca52dfa34ede5f174a8e5c507488c9f9ac5) | libkrun and extensive runtime machinery | Explicit benchmark controls and GPU state sharing | A small custom VMM |
| BoxLite | [`2e41a58`](https://github.com/boxlite-ai/boxlite/tree/2e41a585a076bbe76593c17e035b156ebbe5e7f2) | Multi-backend runtime | Deep snapshot-clone integration testing | A simple machine-core reference |
| AgentENV | Current source inspected 2026-08-29 | Firecracker-based distributed agent environment | Content-addressed environment storage and userfaultfd-backed restoration are useful control-plane references | A platform around Firecracker rather than a new VMM core |
| Yobo | [`4573707`](https://github.com/ahimsalabs/yobo/tree/4573707135b1e005b985952f6cbc0f73c1f4010f) | libkrun-based OCI runtime | Filesystem and process observation around an embedded VMM | A libkrun integration, not a custom machine monitor |
| FCVM | [`57c3d24`](https://github.com/ejc3/fcvm/tree/57c3d249816f9ebce248fc8b59f012c2ba314796) | Firecracker runtime | Broad clone, disk, nested virtualization, and health-test surface | Firecracker orchestration rather than a from-scratch VMM |
| Barista | [`current source`](https://github.com/mpuig/barista.sh) | Session-compute architecture with a Firecracker adapter | Capability negotiation, restore compatibility keys, hook evidence, and explicit degraded fallback | Zero-star project with broader session-platform goals and no custom VMM core |

### Large production references

The census also inspected the module trees and current source of:

- [Firecracker](https://github.com/firecracker-microvm/firecracker) for minimal KVM machines, jailing, versioned snapshots, and benchmark discipline.
- [Cloud Hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor/tree/bb7cb56067c607c9565a44ca565c232909b4637f) for rust-vmm composition, vhost-user, migration, and modern devices.
- [crosvm](https://github.com/google/crosvm/tree/379428d1e2b6a09b9e05325df112c76a9455867d) for process-isolated devices and cross-platform Host adapters.
- [OpenVMM](https://github.com/microsoft/openvmm/tree/64d9210bd70aa40e2437c59e8a07344c8beed671) for a modular multi-platform VMM and mesh-based process topology.
- Kata Containers and Dragonball for container-runtime integration around VM isolation.
- StratoVirt for a Rust KVM microVM and standard-machine implementation.
- libkrun for an embeddable process-isolation VMM library.

These projects are not hidden, but they provide the control group against which obscure designs must be judged.

## Failure atlas

The purpose of this section is not to ridicule experimental work.
It identifies recurring failure modes that SOMA can prevent structurally.

### Failure 1: fail-open snapshot state

Clone's inspected incremental-snapshot path contains several examples:

- Failure to clear the stale KVM dirty bitmap is logged as a warning and execution continues.
- A missing captured vCPU state is replaced with `VcpuState::empty()`.
- A device-state serialization error becomes an empty byte vector through `unwrap_or_default()`.
- A migration device-restore failure is logged and execution continues.
- An arbitrary 100 ms sleep is used to create a dirtying window before pause.

Those behaviors can produce a snapshot that exists but does not represent a coherent machine.
State-integrity paths must fail closed.

SOMA rule:

```text
Missing or invalid state
          |
          v
Abort capture or restore
          |
          v
Destroy the candidate Instance
          |
          v
Never publish Ready
```

No register, interrupt-controller, queue, device, identity, clock, or memory section may have a default substitute during production restore.

### Failure 2: tracking only vCPU-originated dirty memory

KVM dirty logging observes guest writes handled through KVM.
Userspace virtio devices can also write directly into guest memory.

Panorama explicitly merges KVM's dirty bitmap with a device-side guest-memory bitmap before reset.
Clone's inspected incremental collector uses the KVM dirty log and the reviewed path did not expose an equivalent merged device-write bitmap.
That does not prove every Clone configuration loses writes, but it identifies a correctness obligation its incremental path must demonstrate.

SOMA must attach dirty tracking to the guest-memory write API used by every emulated device.
The capture gate must merge CPU, device, and storage dirty state after all producers are quiescent.

### Failure 3: incomplete external-state snapshots

Nanvix documents that its current snapshots do not contain Host TSC state, repaired wall time, Host file descriptors, network sockets, Host channels, or Host filesystem workers.
It warns that snapshotting after mounting HostFS can leave stale remote descriptors, orphaned operations, indefinite blocking, or undefined behavior.
Its orchestrator source still contains TODOs to stop input delivery and flush I/O before pausing.

The lesson is that guest RAM plus vCPU registers is not a complete sandbox snapshot.

SOMA requires an explicit classification for every resource:

| Resource | Capture | Recreate | Reject at capture |
| --- | --- | --- | --- |
| Guest RAM and architectural CPU state | Yes | No | If unsupported |
| TAP, sockets, and Host descriptors | No | Yes, from sterile claims | If live state cannot drain |
| Identity, entropy, clock, and network lease | Never clone blindly | Fresh repair required | If repair cannot be proven |
| External writable volume | Generation-specific policy | Reattach through ownership token | If fencing is absent |

### Failure 4: measuring object restoration instead of readiness

Zeroboot's `fork_time_us` stops after KVM creation, private memory mapping, CPU-state restoration, and object construction.
It does not include authenticated guest readiness or completion of a first command.
Its README labels the resulting value spawn latency.

This is a useful internal phase measurement but not comparable to ComputeSDK Burst TTI.

SOMA always publishes both boundaries:

- Machine restore time.
- Accepted Launch through authenticated Ready and first bounded command.

Neither may be relabeled as the other.

### Failure 5: ignoring KVM restore errors

Zeroboot's inspected restore code discards results from LAPIC restoration, MSR restoration, and MP-state restoration.
It also falls back to Host CPUID if snapshot CPUID is missing, even though the adjacent comment says that condition should not happen.
Its VM-state parser uses unchecked slices followed by `unwrap()`, and the repository contains no Rust test attributes in the inspected source census.

The project labels itself a working prototype and documents cloned userspace PRNG state, which is appropriately honest.
The mistake would be promoting this restore path into a hostile-input production boundary unchanged.

SOMA must reject missing CPU profiles, check the exact restored count for MSRs, propagate every ioctl failure, parse hostile snapshots with bounded decoders, and exercise truncation and mutation campaigns.

### Failure 6: one process owns every tenant

Visor deliberately runs many Linux VMs as threads in one daemon.
Its own design documents identify the tradeoff: an unsafe memory bug or process crash can affect every VM in that process.
The design later moves macOS VMs to per-VM processes because HVF permits only one VM per process, but it retains the shared Linux daemon for the beta.

Fast in-process calls do not outweigh the blast radius for mutually untrusted tenants.
Shared immutable mappings remain shareable across process boundaries, so process-per-VM does not require duplicating every snapshot page.

SOMA retains one jailed VMM process per Machine.

### Failure 7: plans and targets presented beside dead runtime paths

Visor's planning documents quote sub-5-ms snapshot restore targets.
Another inspected planning document states that snapshot save and restore work in the VMM crate but are dead code in the runtime and that the pool creates fresh VMs instead.

Repositories often contain future architecture, current implementation, and benchmark results in adjacent documents.
Readers can easily combine them into a false present-tense claim.

SOMA documentation must label every statement as one of:

- Measured evidence.
- Implemented but not certified.
- Accepted design.
- Engineering target.
- Hypothesis.

### Failure 8: god files hide state ownership

The inspected Vibemon VMM contains files of 4,836, 4,363, 2,526, 2,282, 1,548, and 1,480 lines.
Clone contains large virtio filesystem, qcow2, and MMIO files.
Several runtime-oriented projects contain individual Rust files above 3,000 to 8,000 lines.

File size is not a security proof, but giant modules make ownership, blocking behavior, error policy, and snapshot completeness harder to review.

SOMA enforces cohesive modules and a 300-line authored source limit.
The deeper rule is one owner per invariant, not mechanical fragmentation.

### Failure 9: cross-platform abstraction leaks false equivalence

KVM, HVF, and WHP expose different interrupt, dirty-memory, vCPU-exit, networking, and process models.
Visor's macOS work discovered the hard one-VM-per-process HVF constraint.
Nanvix snapshots use different KVM and WHP state formats.
Vibemon rejects snapshots captured by a different backend.

A portable API must not imply portable snapshots or identical security evidence.

SOMA uses one lifecycle vocabulary with backend capability contracts.
Certification, snapshot compatibility, and performance evidence remain backend-specific.

### Failure 10: cloned identity and randomness

Zeroboot documents that userspace PRNGs such as NumPy and OpenSSL can inherit cloned state.
Firecracker documents the same class of clone hazard.
Restoring kernel entropy alone does not reset every long-lived userspace generator.

SOMA's prepared Generation must stop before tenant-specific secrets or application PRNG state exist whenever possible.
Every Instance must receive a fresh Instance identifier, guest-control session, network identity, time repair, and entropy contribution before Ready.
Workload runtimes that cache PRNG state need an explicit post-restore hook or must start after repair.

### Failure 11: broader machine scope taxes the fast path

Alioth is an excellent VMM reference, but it supports PCI, ACPI, OVMF, VFIO, confidential computing, packed queues, vhost-user, and multiple boot modes.
Cloud Hypervisor, crosvm, OpenVMM, and ArcBox support still broader environments.

Those capabilities are appropriate for their products.
They should not enter SOMA's first minimal Machine merely because the reference implementation already has them.

SOMA borrows module design and correctness techniques while keeping a fixed direct-boot machine and the smallest necessary device set.

### Failure 12: custom operating systems improve speed by changing the product

Nanvix removes ordinary device emulation and co-designs a guest microkernel with Host services.
Hyperlight removes the general Linux environment in favor of a narrow function ABI.
alvm replaces a guest kernel with Host syscall handling.

These are legitimate ways to reduce latency.
They are not transparent substitutes for arbitrary OCI Linux images.

SOMA may eventually add a narrow function or co-designed guest profile, but it must keep that profile distinct from the general Linux Machine contract.

## Positive lessons worth importing

### From Alioth

- Keep Host hypervisor interfaces separate from board, memory, firmware, device, and virtio layers.
- Test split and packed virtqueues independently.
- Treat memory regions and emulated address spaces as explicit objects.
- Keep blocking backends behind worker interfaces.
- Maintain a test hypervisor implementation for platform-neutral machine tests.

### From Nanvix

- The largest latency and density improvements may require guest and VMM co-design.
- Move optional network and storage services out of the minimal guest machine.
- Benchmark boot, cold start, restore, first echo, and steady-state paths separately.
- Document excluded external state instead of pretending snapshot completeness.

### From Clone

- Private file-backed mappings make snapshot pages physically shareable across processes.
- Template identity injection needs a dedicated, auditable phase.
- KSM, ballooning, and memory overcommit address different layers and need separate admission policy.
- Incremental state is useful only after complete dirty-producer accounting.

### From Visor

- In-process switching can remove unnecessary Host network crossings.
- Write explicit compatibility and operational-recovery matrices.
- Document architecture tradeoffs candidly, including crash blast radius.

### From smolvm

- Benchmark harnesses should detect when the requested control configuration was not actually honored.
- Native and virtualized arms need equivalent CPU pinning.
- Single runs, partial completion, NaNs, leaked GPU contexts, and warm-cache effects must be surfaced rather than averaged away.

### From Dillo

- A portable machine boundary can remain genuinely small.
- Backend-owned types prevent KVM or HVF state from leaking upward.
- Individual devices can be isolated without exposing their registers to the lifecycle layer.

## New SOMA guardrails

The census produces these implementation requirements:

1. Snapshot and restore never warn and continue on missing state.
2. Device-originated guest-memory writes participate in dirty tracking.
3. Every Host external resource is classified as captured, recreated, or forbidden.
4. A performance report names the exact start and end events.
5. Restore checks every ioctl and exact item count.
6. Snapshot parsers are bounded, fuzzed, and safe under truncation.
7. One jailed process owns one untrusted Machine.
8. Documentation separates implementation, evidence, targets, and hypotheses.
9. Backends share lifecycle semantics but not unsupported compatibility claims.
10. Fresh identity, entropy, time, and authenticated control precede Ready.
11. Optional machine features do not enter the minimal profile without measured need.
12. A narrower guest profile is named and certified separately from OCI Linux.

## Recommended next implementation tickets

### CENSUS-01: Snapshot fail-closed audit

Search every SOMA capture and restore branch for ignored results, warning-only failures, default state, partial state, and fallback CPU profiles.
Add a mutation test for each rejected case.

### CENSUS-02: Unified dirty-producer ledger

List every code path that writes guest memory.
Require CPU, block, net, RNG, vsock, control page, loader, and repair writes to enter one dirty-tracking contract.

### CENSUS-03: External-state table in the snapshot schema

Record every non-serialized Host resource and the typed repair action that recreates it.
Reject capture while undrainable work remains.

### CENSUS-04: Restore error completeness

Inject failures into every KVM restore operation and prove that no candidate reaches Ready after one fails.

### CENSUS-05: Benchmark boundary conformance

Emit separate restore, repair, Ready, first-command, and cleanup timestamps.
Reject reports that omit failures or relabel restore-only time as sandbox creation.

### CENSUS-06: Process-blast-radius proof

Run malformed queue, panic, abort, memory-pressure, and seccomp-kill tests against one VMM process while sibling VMs run in separate processes.
Prove that the affected Machine is the only lost tenant boundary.

### CENSUS-07: Narrow-profile decision

Evaluate whether a future SOMA function profile should resemble Hyperlight or a Nanvix-style co-designed guest.
Keep it out of version 1 until the general Linux Machine is correct and measured.

## Bottom line

The broader GitHub pass did uncover additional diamonds.
Dillo is the clean interface diamond.
Alioth is the low-level VMM engineering diamond.
Nanvix is the architectural density diamond.
Clone is the warm-fork mechanism diamond, but its failure handling shows exactly what SOMA must harden.

The most valuable discovery is not a secret code fragment that automatically produces 10 ms sandboxes.
It is the repeated pattern that fast systems become unsafe or misleading when they omit external state, ignore partial restore failure, time only object creation, share one unsafe process across tenants, or blur plans with evidence.

SOMA can beat those designs only by combining their strongest mechanisms with stricter ownership, fail-closed restoration, fresh-instance repair, process isolation, and honest end-to-end measurements.
