# Rust VMM GitHub hidden gems

- Date: 2026-08-29
- Scope: Public Rust repositories that implement a VMM or a technically relevant part of one
- Method: GitHub repository search followed by source inspection at pinned commits
- Status: Architecture research, not a security certification or reproduced performance benchmark

## The diamond under the rug

The expanded GitHub census found additional serious projects and changes this document's original single-project ranking.
Dillo remains the strongest minimal backend-interface example.
Alioth is the strongest newly found low-level custom VMM reference.
Nanvix is the strongest co-designed density architecture.
Clone is the strongest newly found warm-fork mechanism reference, although its inspected snapshot paths also contain important fail-open lessons.
The second adversarial search found Amber, a one-star ARM64 VMM that is now the strongest compact cross-platform disposable-microVM reference in the census.
See [Rust VMM GitHub census and failure atlas](rust-vmm-github-census-and-failure-atlas.md) for the expanded repository inventory, exact source findings, mistakes, and revised SOMA guardrails.

There is no single repository that SOMA should copy.

The strongest hidden architecture reference is [pichi-vm/dillo](https://github.com/pichi-vm/dillo/tree/1fc2eb72862c1abfe921eaee6f5adf4e128eddb2).
At the inspected commit it had no GitHub stars, but it contains the cleanest small cross-platform machine boundary found in this search.
Its `dillo-machine` crate defines host-neutral `Host`, `Memory`, `CpuState`, `Cpu`, and `Machine` traits.
KVM, Apple Hypervisor.framework, and Windows Hypervisor Platform implementations live in separate backend crates.
MMIO, PCI, virtio transports, and individual virtio devices also live in separate crates.
The workspace denies unsafe Rust by default.

The strongest hidden fast-reset technique is in [Panorama](https://github.com/00xc/panorama/tree/221699a87fef503927330327d9dfe9a68f98e5de).
Panorama is explicitly marked broken and must not be treated as production software.
Its important idea is still valid: merge KVM's CPU-dirty bitmap with the device-side dirty bitmap, coalesce adjacent dirty pages, restore only those spans, reset device state, inject the next fuzz input, and immediately re-enter the guest.
That loop is a useful design reference for SOMA's hostile-guest snapshot testing and dirty-only rollback evidence.

The richest mechanism source is [Vibemon](https://github.com/stencil-hq/vibemon/tree/99d323dafb697ac60a33a6544a296ee37494718b).
It implements KVM, HVF, and WHP backends, versioned snapshots, delta memory, copy-on-write fork paths, userfaultfd paging, device-state validation, and platform-specific jailing.
It is not the module-shape reference for SOMA.
Its inspected VMM contains source files of 4,836, 4,363, 2,526, 2,282, 1,548, and 1,480 lines, which conflicts with SOMA's deep-module and no-god-files rules.

The practical synthesis is:

```text
Dillo boundaries
      +
Amber cross-platform disposable-machine flow
      +
Vibemon mechanisms selected one at a time
      +
Panorama dirty-only reset testing
      +
Hyperlight snapshot discipline
      +
SOMA's own minimal Linux KVM machine contract
      =
The best current direction for SOMA
```

## New find: Amber

Pinned source: [commit `54cebed`](https://github.com/lupodevelop/amber/tree/54cebedae733633ceb9f633b8f99c349d81e941e).

Amber had one GitHub star when inspected, but it is not a toy repository.
The pinned tree contains about 13,000 lines of Rust and 133 Rust tests across separate core, HVF, KVM, image, network, guest-agent, and CLI crates.
Its backend-neutral core owns direct Linux boot, device-tree construction, PL011, virtio MMIO block, RNG, network, balloon, vsock, snapshot format, and the vCPU run loop.
Separate crates implement Apple Hypervisor.framework and Linux KVM behind the same narrow `Hypervisor` and `Vcpu` traits.

The most important Apple-specific idea is a serializable software GICv2.
Amber documents that the Apple in-kernel interrupt-controller path could not restore a working timer, so it moved the GIC model into userspace and added periodic vCPU exits for timer injection.
That is concrete evidence that a portable VMM cannot assume interrupt-controller snapshot semantics are equivalent across HVF and KVM.

Its disposable-sandbox path is also directly relevant to SOMA:

- Pull and flatten an OCI image into a read-only squashfs base with an ephemeral tmpfs overlay.
- Boot a trimmed ARM64 Linux kernel to a tiny guest agent.
- Capture RAM, every vCPU, interrupt-controller state, device queues, and machine metadata at an agent marker.
- Restore RAM through a private file mapping so unchanged pages share the page cache.
- Keep pre-restored workers paused in a warm pool.
- Resume one worker, establish a fresh host connection over virtio-vsock, stream a framed command, and discard the worker afterward.
- Recreate userspace network state instead of serializing live host sockets.

Its committed M1 Pro result reports five warm `exec` runs of 30, 30, 31, 31, and 32 ms from CLI invocation through exit code, plus about 16 MiB resident memory per idle 512 MiB-cap fork.
This is useful third-party evidence, not a SOMA benchmark and not evidence of 10 ms readiness.
The sample has only five latency runs, uses a trivial `echo` workload, contains no cold-cache distribution, and has no real-hardware Linux KVM result.

The failure lessons matter as much as the mechanism:

- The KVM GIC restore loop logs and skips failed device attributes instead of rejecting the restore.
- Several guest-agent writes, joins, and cleanup operations discard errors.
- The daemon, virtio model, CLI, VM loop, software GIC, and vsock implementation each exceed SOMA's 500-line source-file target, with the largest file at 1,395 lines.
- Its benchmark record explicitly leaves sustained churn, heavier workloads, and Linux KVM performance unmeasured.
- Its security design is promising but source inspection is not a security certification.

SOMA should borrow Amber's software-interrupt-controller lesson, host-neutral core shape, OCI-to-template flow, fresh post-restore vsock connection, and end-to-end measurement boundary.
SOMA must retain its stricter fail-closed restore contract, smaller deep modules, authenticated repair state machine, complete evidence bundle, and Linux-first production proof.

## Classification matters

GitHub results frequently use `VMM`, `microVM`, `sandbox`, and `hypervisor` for different products.
The following classification prevents SOMA from copying the wrong layer.

| Class | What it actually owns | Examples | Value to SOMA |
| --- | --- | --- | --- |
| Custom type-2 VMM | Creates memory, vCPUs, interrupts, devices, and run loops through KVM, HVF, or WHP | Dillo, Vibemon, Ignition, Microcosm, Kitsune | Direct architecture evidence |
| Embedded function VMM | Runs a deliberately smaller guest ABI instead of a general Linux machine | Hyperlight, alvm | Latency and narrow-interface lessons |
| Existing-VMM runtime | Orchestrates Firecracker, Cloud Hypervisor, libkrun, or another VMM | forkd, Bake, microsandbox | Pool, fork, control-plane, and user-experience lessons only |
| Snapshot or fuzz research VMM | Optimizes deterministic execution, reset, or fuzzing rather than a product lifecycle | Panorama, deterministic-vmm | Testing and state-model lessons |
| Teaching VMM | Demonstrates direct boot and basic devices with limited hardening | Microcosm, FerrumVM, Kitsune, ToyVMM | Clear implementation examples, not production foundations |
| Type-1 hypervisor | Runs directly below guest operating systems rather than as a Linux KVM process | hvisor, libvmm | Security and verification ideas, but a different deployment model |

## Ranked source findings

### 1. Dillo: the architecture diamond

Pinned source: [commit `1fc2eb7`](https://github.com/pichi-vm/dillo/tree/1fc2eb72862c1abfe921eaee6f5adf4e128eddb2).

Dillo's most valuable file is its [host-neutral machine interface](https://github.com/pichi-vm/dillo/blob/1fc2eb72862c1abfe921eaee6f5adf4e128eddb2/deps/dillo-machine/src/lib.rs).
The interface does not expose KVM file descriptors, HVF handles, WHP partitions, virtqueue descriptors, or device registers.
The launcher supplies launch facts and attaches backend-owned memory and CPU state.
The backend owns how those facts become a running machine.

Its workspace separates:

- `dillo-machine` from `dillo-machine-kvm`, `dillo-machine-hvf`, and `dillo-machine-whp`.
- MMIO from PCI.
- virtio transport from virtio devices.
- block, console, filesystem, network, and vsock devices from one another.
- production code from the `snuffler` and fuzz harnesses.

What SOMA should borrow:

- A narrow backend-neutral machine contract.
- Backend-owned memory and CPU types.
- Separate host adapters rather than conditional branches spread through the machine core.
- One cohesive crate for each transport or device family only when the boundary remains deep.
- Workspace-wide `unsafe_code = "deny"`, with narrowly isolated exceptions only where unavoidable.

What SOMA should not copy blindly:

- Its PMI boot-image format, because SOMA already owns an OCI-to-Generation contract.
- Its broad device set, because every additional device enlarges the attack and snapshot surfaces.
- Its cross-platform target as an excuse to weaken the Linux KVM fast path.

### 2. Vibemon: the mechanism mine

Pinned source: [commit `99d323d`](https://github.com/stencil-hq/vibemon/tree/99d323dafb697ac60a33a6544a296ee37494718b).

The inspected `vmm` crate uses `kvm-bindings`, `kvm-ioctls`, `virtio-queue`, `vm-memory`, `vmm-sys-util`, `linux-loader`, and `vm-superio` on Linux.
It provides separate host hypervisor adapters for KVM, Apple HVF, and Windows WHP.

The valuable mechanisms are:

- A pause gate that kicks vCPUs out of the host run call, waits for every vCPU to park, and keeps backend-local vCPU state on the owning thread.
- A versioned snapshot envelope containing architecture, backend, memory layout, vCPU state, machine state, serial state, device state, and optional delta-memory metadata.
- Fail-closed restore checks for snapshot version, architecture, backend, memory layout, queue layout, and device consistency.
- Delta memory and guest-write tracking.
- Linux userfaultfd paging with explicit failure when the host denies the facility.
- Platform-specific isolation using Landlock and seccomp on Linux and a separate macOS path.
- Real end-to-end tests for snapshot, restore, fork isolation, guest state, network state, and command execution.

The architectural warnings are equally valuable:

- `vmm.rs` is 4,836 lines at the inspected commit.
- `vmm_windows.rs` is 4,363 lines.
- `snapshot/mod.rs` is 2,526 lines.
- `config.rs` is 2,282 lines.
- Control, paging, TAP, jail, and sandbox files also exceed SOMA's 500-line architecture rule.
- The repository combines VMM, daemon, cloud, SDK, UI, mesh, and deployment concerns, so it is evidence for mechanisms rather than a small product core.

SOMA should reproduce individual behavior behind its own interfaces and tests.
SOMA should not import Vibemon's top-level topology or treat repository claims as benchmark evidence.

### 3. Panorama: dirty-only rollback as a testing primitive

Pinned source: [commit `221699a`](https://github.com/00xc/panorama/tree/221699a87fef503927330327d9dfe9a68f98e5de).

Panorama takes a complete baseline snapshot and then restores only pages made dirty after that point.
Its memory restore merges KVM's dirty log with a guest-memory bitmap that accounts for userspace device writes.
It groups neighboring dirty pages into spans before copying baseline bytes back.
Its block backend independently records modified file ranges and restores those ranges from its baseline.

The design lesson is larger than fuzzing.
KVM dirty logging does not automatically account for every host-side device write into guest memory.
A correct dirty-only reset mechanism therefore needs one authoritative merged dirty view across vCPU writes, emulated DMA, and storage state.

SOMA should use this pattern for a dedicated reset-and-fuzz harness, not as an immediate production snapshot implementation.
The repository labels itself broken, has no declared license in GitHub metadata, and contains unfinished code.

### 4. Hyperlight: an intentionally smaller guest contract

Pinned source: [commit `b9266a8`](https://github.com/hyperlight-dev/hyperlight/tree/b9266a8e61a5f9636bf64dc03dfaaad7789f28a6).

Hyperlight is an embedded VMM for calling functions inside small hardware-isolated guests.
It is not a full OCI Linux agent sandbox and should not replace SOMA's Linux Machine product.

The important lessons are:

- A smaller guest ABI can remove kernel boot, general device discovery, and unrelated operating-system work from the measured path.
- The embedding process should receive a small sandbox builder and call interface rather than virtualization internals.
- Snapshot schemas need golden compatibility tests, explicit versioning, and complete architectural state such as MSRs and extended register state.
- Restore must reseed per-instance identity and entropy instead of cloning the source guest's identity.

Hyperlight is the best reference here if SOMA later adds a distinct function-isolation profile.
That profile must remain explicit and must not be described as the same execution environment as a general Linux sandbox.

### 5. Ignition: the Apple HVF research reference

Pinned source: [commit `2bc5272`](https://github.com/vadika/ignition/tree/2bc5272aabcd4ba5f7c65824eedcabd3b6b4a61d).

Ignition is a custom Apple Silicon VMM that boots Linux through Hypervisor.framework.
Its source separates architecture, HVF, devices, and VMM crates.
It documents snapshot restore using `clonefile` and shared mappings, dirty tracking that includes device writes, Seatbelt isolation, SMP, and a broad virtio set.

This is useful for SOMA's macOS development backend and for understanding HVF snapshot constraints.
It does not prove SOMA's Linux KVM production latency, jail, networking, or density targets.
Its published latency statements remain third-party claims until SOMA reproduces a comparable measurement boundary on retained hardware.

### 6. deterministic-vmm: exact execution is a different optimization axis

Pinned source: [commit `0b5ba86`](https://github.com/hashbrowncipher/deterministic-vmm/tree/0b5ba868ddaa6a3b0bd110cfb0a4fbe63009ae06).

This small VMM replaces guest-visible time with retired-instruction counts, emulates the local APIC timer in userspace, and uses PMU-driven preemption to land interrupts at deterministic instruction boundaries.
It requires a custom host KVM patch and trades performance and compatibility for exact replay.

SOMA should not place this mechanism in the default fast sandbox.
It is a valuable future diagnostic profile for reproducing races and making security failures replayable.
The reusable idea is to define determinism as a separately certified machine profile instead of quietly changing the production clock model.

### 7. ai-vmm: prove hostile-input bounds, not marketing sentences

Pinned source: [commit `50b8c88`](https://github.com/SO2304/ai-vmm/tree/50b8c88993bca7ff74ba1b3aa73bdab2c4c425a3).

This repository contains real Kani proofs for validation helpers, memory-size arithmetic, vCPU caps, network-interface names, storage offsets, request bounds, and registry limits.
That makes it a useful demonstration of proving small pure functions around a VMM.

The source inspection does not support interpreting its phrase "every limit is formally proven" as proof of the complete VMM.
The KVM ioctls, virtual-device semantics, concurrency, guest-memory aliasing, kernel behavior, and end-to-end isolation boundary remain outside those local harnesses.

SOMA should add Kani or an equivalent bounded model checker where the proof boundary is narrow and explicit:

- Guest-address range arithmetic.
- Virtqueue index and descriptor-chain bounds.
- Snapshot section offsets and lengths.
- Resource-cap calculations.
- Network and disk request bounds.
- State-machine transition completeness.

### 8. alvm: remove the guest kernel only when the contract permits it

Pinned source: [commit `ea6d3a1`](https://github.com/mathetake/alvm/tree/ea6d3a125d4c34653c2c936fea7bafba19de0eb5).

alvm runs static AArch64 Linux ELF programs on macOS HVF without a guest Linux kernel and handles trapped syscalls in Rust.
This can reduce startup work, but the host-side syscall implementation becomes a large compatibility and security boundary.

It is not a drop-in way to run arbitrary OCI images, Node.js installations, package managers, or normal Linux agents.
It is useful evidence for a future narrow binary-sandbox profile only.

### 9. Microcosm, Kitsune, FerrumVM, and MiniHype: teaching references

[Microcosm](https://github.com/mosmeh/microcosm/tree/104cf14ef22d413bee0210eb72d97c1dbd52a6d7) is a small direct-boot KVM VMM supporting Linux, PVH, and Multiboot with a minimal legacy device set.
[Kitsune](https://github.com/lapla-cogito/kitsune/tree/146ce16a600ecdc51a1e5a25c17d949e1310d0fc) adds multi-vCPU, virtio block, virtio net, ACPI, and reset handling.
[FerrumVM](https://github.com/milosilo-dev/FerrumVM/tree/a4ba316eb20bf356b5ee59be6cb5ca6cd4a671e8) is useful for studying the complete path from reset vector and custom firmware to Linux.
[MiniHype](https://github.com/64bit/miniHype/tree/57215bf7b0e38bc71e71452cc50a9b669fb4b963) is a very small KVM and HVF teaching example.

These projects can make individual mechanisms understandable.
They do not provide the complete snapshot, isolation, hostile-device, compatibility, lifecycle, or benchmark evidence required by SOMA.

## Projects that look relevant but are a different layer

### forkd

[forkd](https://github.com/deeplethe/forkd/tree/e2fd1a6e12522b05c95d85953f6b97b8e1fcaa1e) is based on a Firecracker fork.
Its valuable work is live branching, userfaultfd write protection, memory copy-on-write, and high-fanout orchestration.
It is not evidence for how to write SOMA's custom VMM core.

### microsandbox

[microsandbox](https://github.com/superradcompany/microsandbox/tree/288ef7c89fe3048abff44521db2ef5ec330e4b1c) is a sandbox runtime built around existing virtualization components.
It is useful for API and developer-experience research, not for KVM machine ownership.

### Bake

[Bake](https://github.com/losfair/bake) uses Firecracker as its machine mechanism.
It belongs in restore-pipeline research rather than custom-VMM architecture research.

### libvmm and hvisor

[libvmm](https://github.com/libvmm/libvmm/tree/ef5c54b020ea799cd4a5f8163fae1fe65ed6e19e) experiments with a Rust library VMM below KVM's abstraction level using VMX and EPT directly.
[hvisor](https://github.com/syswonder/hvisor) is a type-1 hypervisor.
Both can teach hardware virtualization and formal verification, but adopting their layer would replace SOMA's Linux KVM deployment model rather than improve it incrementally.

## What changes in the SOMA design

### Keep one product Machine

SOMA should keep one hardware-isolated Linux Machine contract.
KVM, HVF, and any future WHP implementation are Host backends for that contract, not different public sandbox products.
Preparation classes such as cold boot, warm restore, prepared worker, and ready pool are lifecycle states, not different VMM architectures.

### Contract the machine interface

The public or cross-crate surface should resemble this intent-level shape:

```rust
trait MachineBackend {
    type Machine;
    type Error;

    fn create(&self, generation: &Generation, shape: Shape) -> Result<Self::Machine, Self::Error>;
    fn restore(&self, image: &CertifiedState, identity: FreshIdentity) -> Result<Self::Machine, Self::Error>;
}

trait Machine {
    fn start(&mut self, deadline: Instant) -> Result<Started, MachineError>;
    fn pause(&mut self, deadline: Instant) -> Result<Paused, MachineError>;
    fn snapshot(self, target: SnapshotTarget) -> Result<CertifiedState, MachineError>;
    fn stop(self, deadline: Instant) -> Result<CleanupEvidence, MachineError>;
}
```

KVM descriptors, memory slots, vCPU registers, irqfds, ioeventfds, virtqueue cursors, and snapshot byte layouts stay private.
Separate internal modules may expose narrower traits to each other, but the lifecycle caller must not assemble a machine from raw parts.

### Add four focused evidence tracks

1. Differential virtqueue testing against `virtio-queue` and the Virtio specification.
2. Dirty-reset testing that merges KVM writes with every device-originated guest-memory write.
3. Bounded-model proofs for guest-controlled arithmetic and state transitions.
4. Snapshot golden tests that pin schema versions, reject incompatible state, and prove fresh identity after restore.

### Do not combine every clever mechanism into the first fast path

Userfaultfd paging, delta chains, deterministic execution, cross-platform restoration, live migration, and function-only guests are distinct complexity multipliers.
Each must enter behind a capability and evidence gate.
The initial sub-10-ms target should remain one prepared Linux KVM Generation on one certified Host profile with immutable shared memory, private copy-on-write mappings, a minimal fixed device set, and authenticated readiness.

## Recommended implementation order

1. Replace `soma-kvm`'s broad exported surface with one private machine aggregate and one narrow adapter used by `soma-vmm`.
2. Write down exclusive ownership for VM, vCPU, guest memory, device, event, snapshot, and teardown state.
3. Decide whether `vm-memory` and `virtio-queue` replace or differentially validate SOMA's custom implementations.
4. Add Kani harnesses for address arithmetic, queue bounds, snapshot offsets, and resource calculations.
5. Build the dirty-reset test harness using merged CPU and device dirty tracking.
6. Implement a versioned snapshot manifest with golden compatibility fixtures and restore typestate.
7. Implement the prepared restore path and measure every phase independently.
8. Evaluate userfaultfd only after the ordinary private file-backed restore path is correct and measured.
9. Keep HVF as a development conformance backend and report its evidence separately from Linux KVM.
10. Admit production traffic only after the jail, authenticated readiness, hostile guest, cleanup, and burst gates all pass together.

## Bottom line

Dillo is the hidden minimal-interface diamond.
Alioth is the low-level VMM engineering diamond.
Nanvix is the co-designed density diamond.
Clone is the warm-fork mechanism diamond, but its snapshot failure handling must not be copied.
Panorama contains the hidden dirty-reset testing gem.
Vibemon contains the broadest collection of production-shaped mechanisms but also demonstrates why SOMA must enforce strict module boundaries.
Hyperlight shows how much faster the system can become when the guest contract is intentionally narrower, but that is a different profile from a general Linux agent sandbox.

SOMA should not copy a competitor wholesale.
It should preserve its own minimal Linux KVM machine, reshape it around a Dillo-like backend boundary, import only source-proven mechanisms, and require independent evidence for every claimed latency or isolation property.
