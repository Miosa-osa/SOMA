# SOMA module map

Read [From hardware to an agent sandbox](beginners-guide.md) first if the distinction between a sandbox, Machine, Instance, VMM, KVM, Template, Generation, and Snapshot is not already clear.

## Purpose and status

This document assigns responsibilities and dependency direction for the initial pre-alpha workspace.
It prevents lifecycle, KVM, protocol, and provider concerns from accumulating in one god file.
It is a code ownership map rather than a claim that the complete VMM, restore path, device model, or production security architecture already exists.

The current workspace contains thirteen implemented crates:

```text
crates/
  soma/
  soma-cli/
  soma-generation/
  soma-guest/
  soma-guest-agent/
  soma-hostd/
  soma-jail/
  soma-kvm/
  soma-local/
  soma-macos/
  soma-mcp/
  soma-netd/
  soma-storage/
  soma-template/
  soma-vmm/
```

The current alpha contains a portable use-case facade, durable local lifecycle state, a semantic Machine-contract slice, Linux KVM capability probes, explicit-fixture ARM64 KVM cold-boot and challenge-bound direct-command proofs, a development-only macOS VM-per-OCI backend, a verified bounded local OCI-layout importer, a deterministic normalized logical rootfs artifact, portable authenticated-session primitives, a statically linked Linux PID 1 guest agent that boots inside a SOMA virtual machine and reaches an authenticated first command, a Template compiler slice that parses one versioned Template document, composes built-in modules, applies the required validation classes, and emits a canonical Template Lock, a Linux XFS reflink storage profile with sterile ext4 templates, descriptor-only head cloning, single-use leases, and a retained clone-latency matrix, a node-local host allocator library with bounded sterile-worker pools, single-winner idempotent claims, exactly-once authority transfer, a durable lifecycle ledger, restart reconciliation, and multi-dimension capacity admission behind launcher and broker seams, a Linux x86_64 VMM jail launcher with retained privileged-container evidence that does not yet wrap the real VMM, a command-line adapter, and a bounded stdio MCP adapter.
It does not yet contain the production x86_64 guest boot path, snapshot restore implementation, production device model, jail launcher behind the host allocator, complete Generation builder, a Generation built from a Template Lock, or remote transport.

The long-term direction is a state-of-the-art hardware-isolated sandbox engine across clouds, resource shapes, and disk sizes.
The only initial production target is Ubuntu 24.04 x86_64 with KVM.
Additional targets earn support through conformance and do not expand the initial crate graph speculatively.

## Dependency direction

```text
human, agent, SDK, or operator
               |
               v
       soma portable facade
                 |
                 v
          soma-local adapter
           /             \
          v               v
     soma-macos       soma-kvm

    soma-vmm semantic lifecycle prototype

    soma-generation -> soma identity types
    soma-template   -> soma request types
    soma-guest       independent protocol foundation
    soma-guest-agent -> soma-guest
    soma-netd        -> soma request types, soma-guest launch identity
    soma-storage     independent Linux storage mechanism
    soma-kvm live tests -> soma-guest, soma-generation, soma (dev-dependencies only)
    soma-jail        -> libc only; the launcher that will exec soma-vmm
    soma-hostd       -> soma-storage leases, soma-netd bundle types, soma-guest launch identity
```

The portable `soma` facade owns use-case orchestration and execution-receipt construction.
`soma-cli` owns only command-line parsing, human and JSON rendering, and process exit behavior over that facade.
`soma-mcp` maps bounded MCP tools onto that same facade.
`soma-local` owns durable local lifecycle state, backend selection, and target-gated local composition.
`soma-vmm` owns the provider-neutral Machine interface and the deep lifecycle implementation.
`soma-kvm` owns target-gated access to Linux x86_64 production KVM capabilities and Linux ARM64 development KVM capabilities.
`soma-macos` owns the development-only Apple VM-per-OCI lifecycle adapter.
`soma-generation` verifies bounded OCI image-layout input, publishes immutable imported and normalized logical-tree artifacts, and compiles uncertified x86_64 machine artifacts plus a `SOMAGEN` manifest without booting, capturing, or certifying a Generation.
`soma-template` owns Template parsing, module composition, required validation, and canonical Template Lock construction in the preparation plane beside `soma-generation`; it depends on the portable `soma` request types only, and the Generation builder consumes its lock rather than the reverse.
`soma-guest` owns the portable authenticated-session and encrypted-record primitives without claiming a live guest agent or readiness.
`soma-guest-agent` is the Linux-only PID 1 executable that consumes those primitives inside the guest; it depends on `soma-guest` and `libc` only and never on the VMM or host crates.
`soma-netd` is the privileged Linux network broker; it consumes the portable network request types from `soma` and produces the `LaunchNetwork` identity from `soma-guest`, and it never depends on the VMM, KVM, or provider crates.
`soma-storage` owns the XFS reflink disk-head profile as a standalone mechanism crate; the host allocator consumes it and it never depends on the VMM, KVM, guest, or provider crates.
`soma-jail` owns the privileged launcher that constrains one VMM process; it depends on `libc` alone and never on `soma-kvm`, `soma-vmm`, or a provider adapter, because it must stay auditable as the last privileged step before the VMM executes.
`soma-hostd` is the node-local allocator accepted by ADR 0006; it consumes `soma-storage` leases, `soma-netd` bundle identities and intents, and the `soma-guest` launch network identity, defines the launcher and broker seams the jail adapter and the live brokers will implement, and never depends on the VMM, KVM, or provider crates.
`soma-kvm` must not depend on `soma-vmm`, provider control planes, OCI clients, or benchmark code.
`soma-kvm` is a public package while `soma-guest` is private, so the sandbox machine exposes a byte-level control channel and launch-page slot, and the `soma-guest` protocol glue that turns them into an authenticated session lives in the crate's live test as a dev-dependency until `soma-vmm` owns it.
No provider adapter belongs below the public Machine seam.
The current low-level crates remain independent while `soma-vmm` uses an unavailable production platform adapter.
The conceptual arrows become Rust dependencies only when a deep implementation requires them.

## External seam

The public interface has three commands:

```text
Launch(Launch)   -> Ready
Execute(Execute) -> Executed
Stop(Stop)       -> Stopped
```

These commands are the accepted seam, while each checked-in pre-alpha slice must document which commands and outcomes it actually implements.
An absent command or outcome is unsupported rather than implied by this map.

This small interface provides leverage because callers receive lifecycle ordering, typed failures, replay, rollback results, and cleanup results without orchestrating those phases.
Real verification, restore, authentication, and resource cleanup remain behind the same seam when production adapters arrive.
The seam lives at one per-Machine `soma-vmm` process.
The certified fast path also uses the constrained node-local allocator accepted in ADR 0006, but that allocator transfers ownership and does not proxy steady-state Machine commands.
Tests must cross the same semantic public interface as external callers.
Encoded-transport conformance is deferred until a protocol codec exists.

The public contract may report milestones inside command responses.
Milestones remain evidence and never become caller-controlled stage commands.
There is no public method to mark an Instance Ready or bypass Repair.

## `soma-vmm` responsibilities

`soma-vmm` owns one Machine lifecycle.
It contains the interface types initially so the project can discover the real contract before splitting a standalone contract crate.
It does not own KVM ioctl details, provider policy, placement, warm pools, OCI acquisition, billing, public sandbox terms, or benchmark orchestration.

The initial source map is:

```text
crates/soma-vmm/src/
  lib.rs
  ids.rs
  spec.rs
  request.rs
  error.rs
  receipt.rs
  machine/
    mod.rs
    launch.rs
    execute.rs
    stop.rs
    fault.rs
  operation.rs
  platform.rs
  tests/
    execute.rs
    launch.rs
    operation_safety.rs
    stop.rs
```

### `soma-vmm/src/lib.rs`

`lib.rs` is the public export map and composition entry point.
It exports the Machine interface and intended contract types while keeping implementation modules private.
It must not contain lifecycle logic, KVM calls, parsing logic, or a second copy of validation rules.

The root remains shallow by design because a composition root wires deep modules rather than pretending to be one.
A reusable behavior does not belong in `lib.rs` merely because several modules need it.

### `ids.rs`

`ids.rs` owns opaque validated identities such as `GenerationId`, `InstanceId`, and `OperationId`.
The Phase 0 types reject all-zero fixed-width values and expose their exact byte representations for comparison.
It does not generate provider tenant identifiers or interpret billing identity.

IDs must not be interchangeable string aliases.
An identity that crosses the public seam must have explicit length, character, serialization, and redaction behavior.

### `spec.rs`

`spec.rs` owns the immutable Generation reference and validated Machine dimensions.
The initial dimensions include `VcpuCount`, `MemoryBytes`, and `DiskBytes`.
The Phase 0 dimension constructors reject zero and otherwise preserve exact values without rounding.
Checked arithmetic, architectural alignment, Generation compatibility, and host limits belong to later target adapters and conformance gates.

The types are technical dimensions rather than hardcoded product tiers.
Provider plans, quota policy, host placement, NUMA selection, and billing remain outside the VMM.
Generation and host compatibility constrain valid dimensions without silently rounding to another shape.

### `request.rs`

`request.rs` owns `Launch`, `Execute`, and `Stop` plus their validated fields.
It binds every mutating request to an `OperationId`.
Phase 0 replay compares the complete Rust request structurally because no canonical wire encoding or request fingerprint exists yet.
Phase 0 accepts logical identities, an exact Generation and Machine specification, and bounded command values rather than host paths, TAP names, device names, and provider credentials.
Future resource authority must use constrained capabilities or certified references rather than adding those host details as casual strings.

Request types state caller obligations but do not implement lifecycle transitions.
Changing a public request requires protocol compatibility review and an ADR when semantics change.

### `receipt.rs`

`receipt.rs` owns immutable outcomes, ordered milestones, execution status and retained output, and cleanup evidence.
The Phase 0 Ready receipt contains the `OperationId`, `InstanceId`, `GenerationId`, the Generation's exact `MachineSpec`, and ordered milestones.
Request fingerprints, timestamps, and authenticated command evidence require later encoded-protocol and production-adapter work.

Receipts do not expose mutable internal state or host implementation details.
Serialization changes require compatibility tests and explicit versioning.

### `error.rs`

`error.rs` owns typed public failures, lifecycle phases, recovery directives, and cleanup evidence.
Phase 0 distinguishes operation conflict and capacity, invalid lifecycle, verification and restore stages, guest authentication and Repair stages, readiness, Instance mismatch, Execute, and Stop.
It does not leak guest secrets, host paths, raw descriptors, or provider credentials through error text.

Every Phase 0 failure states a recovery class and whether cleanup is complete.
The operation ledger, rather than the failure value alone, determines whether the admitted Stop remains in Reaping.
Internal error chains may be retained for operator evidence while the public fault remains stable and redacted.

### `machine/`

`machine/` is the deep implementation behind `Launch`, `Execute`, and `Stop`.
It owns the lifecycle state machine, legal transition ordering, failure conversion, rollback initiation, and the rule that Ready follows authenticated Repair and a no-op Execute probe.
It is the only module allowed to advance the terminal Machine lifecycle.

`machine/mod.rs` owns shared state, while `launch.rs`, `execute.rs`, `stop.rs`, and `fault.rs` keep cohesive orchestration and failure conversion local.
This authority does not make the `machine/` module a dumping ground.
It coordinates behavior supplied by focused internal modules and must not absorb KVM ioctls, protocol codecs, host-path handling, device emulation, or general utilities.
When a cohesive responsibility becomes deep enough to stand alone, it moves behind a private interface rather than becoming another public lifecycle method.

Expected extraction candidates include Generation verification, private guest memory, guest authentication and Repair, resource ownership, and virtual devices.
An extraction occurs only when the responsibility has its own invariants and at least two meaningful adapters or an independently testable deep implementation.
The project does not pre-create empty crates or shallow pass-through modules for these names.

### `operation.rs`

`operation.rs` owns structural request comparison, operation replay, conflict detection, bounded Execute retention, and one dedicated Stop record within the per-Machine lifecycle.
The same `OperationId` and structurally equal request returns the recorded terminal outcome.
The same `OperationId` with a structurally different request returns an operation-conflict fault and performs no mutation.
An admitted Stop remains in Reaping after incomplete cleanup, so replaying that exact Stop continues cleanup instead of replaying a terminal failure.
No other Phase 0 operation repeats work under the same `OperationId`.

This module does not become a host-wide durable journal.
The node-local allocator owns unassigned worker and resource inventory, while cross-process operation durability and fleet recovery remain operator responsibilities.

### `platform.rs`

`platform.rs` is the private seam through which the Machine lifecycle obtains host behavior.
The production adapter will delegate Linux x86_64 KVM capability work to `soma-kvm` when that adapter is implemented.
A deterministic adapter may exercise the same Machine interface in platform-neutral tests without pretending to certify KVM behavior.

The seam must expose capabilities and owned resources rather than raw provider policy.
It must not grow one method for every KVM ioctl or lifecycle milestone.
If deleting the adapter merely moves pass-through calls into `machine/`, the seam is too shallow and must be redesigned.

### `tests/`

Focused files under `tests/` drive the public `Launch`, `Execute`, and `Stop` interface.
They cover successful ordering, failure before Ready, idempotent replay, operation conflict, Execute rejection before Ready, authenticated no-op readiness, and idempotent Stop.
They use deterministic host behavior only to test provider-neutral semantics.

Linux KVM evidence belongs in `soma-kvm` integration tests and later full target-host tests.
A deterministic test passing on Apple Silicon must never be labeled a KVM restore result.

## `soma-kvm` responsibilities

`soma-kvm` is the target adapter for Ubuntu 24.04 x86_64 production KVM host access and Linux ARM64 development proofs.
Its current depth includes a checked capability probe, an x86_64 machine that maps one private memory slot, enters one protected-mode vCPU, boots the pinned PVH kernel to a challenge-bound serial sentinel through a diagnostic 16550 model, captures port-I/O exits and `hlt`, and enforces a watchdog deadline with proven cleanup, plus explicit-fixture ARM64 direct-boot and command paths with checked memory layout, vCPU initialization, GICv3, timer and device-tree description, separate diagnostic and control UARTs, strict challenge-bound frames, direct guest execution, and bounded teardown.
It also contains a target-independent, `unsafe`-free modern virtio-mmio version 2 transport and split-virtqueue implementation under `virtio/`, the five v1 device models under `virtio/devices/`, and the fixed five-slot MMIO bus under `virtio/bus.rs`.
The `x86_64/` modules wire that bus to KVM: `KVM_EXIT_MMIO` dispatch on the vCPU thread, one ioeventfd per queue-notify address, one irqfd per slot, a bounded epoll device thread, a shared range-checked guest-memory view, the dedicated launch-page slot, a deadline-bounded byte channel over the vsock host endpoint, and the test-only sandbox machine that cold-boots a compiled Generation for the static guest agent; the retained proofs are [the x86_64 PVH kernel-boot evidence](../evidence/2026-08-29-x86_64-pvh-kernel-boot.md) and [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md).
As real restore work arrives, the crate will own register restoration, interrupt-controller state, clock state, and the snapshot-driven execution loop over the same seams.
It also owns the platform-neutral snapshot format v1 codec under `snapshot/`: the `SOMASNP` manifest, bounded digest-covered sections, SOMA-owned byte layouts for every x86_64 KVM state group, per-device state, the memory-object descriptor, the fail-closed compatibility check, and the capture and restore ordering contracts.
That codec performs no KVM ioctl; live capture and restore remain a later slice.

The initial source map is:

```text
crates/soma-kvm/src/
  lib.rs
  linux.rs
  machine.rs
  x86_64/
    boot_info.rs
    channel.rs
    cmdline.rs
    cpuid.rs
    devices.rs
    elf.rs
    elf/
      header.rs
      note.rs
      synthetic.rs
      tests.rs
    error.rs
    event_loop.rs
    events.rs
    guest.rs
    halt.rs
    kernel.rs
    kernel/
      config.rs
    kick.rs
    launch_page.rs
    layout.rs
    loader.rs
    memory.rs
    memory/
      tests.rs
    mmio.rs
    mod.rs
    ports.rs
    run.rs
    sandbox.rs
    sandbox/
      evidence.rs
      launch.rs
      teardown.rs
    serial.rs
    serial/
      tests.rs
    timing.rs
    vcpu.rs
    watchdog.rs
  virtio/
    mod.rs
    bus.rs
    bus/
      slots.rs
      table.rs
      tests.rs
    device.rs
    device/
      test_device.rs
    devices/
      mod.rs
      harness.rs
      segments.rs
      service.rs
      block.rs
      block/
        backend.rs
        execute.rs
        hostile_tests.rs
        identity_tests.rs
        request.rs
        state.rs
        tests.rs
      net.rs
      net/
        backend.rs
        frame.rs
        hostile_tests.rs
        rx.rs
        state.rs
        tests.rs
      rng.rs
      rng/
        backend.rs
        state.rs
        tests.rs
      vsock.rs
      vsock/
        connection.rs
        credit.rs
        guest_driver.rs
        hostile_tests.rs
        lifecycle_tests.rs
        outbound.rs
        packet.rs
        rx.rs
        state.rs
        tests.rs
        tx.rs
    guest_memory.rs
    guest_memory/
      tests.rs
    queue.rs
    queue/
      chain.rs
      chain/
        tests.rs
      layout.rs
      state.rs
      state/
        tests.rs
      tests.rs
      violation.rs
    transport.rs
    transport/
      driver_model_tests.rs
      host.rs
      lifecycle_tests.rs
      registers.rs
      restore_tests.rs
      state.rs
      status.rs
      tests.rs
      violation.rs
      write.rs
  snapshot/
    mod.rs
    capture.rs
    compatibility.rs
    compatibility/reason.rs
    device_state.rs
    device_state/queue.rs
    device_state/specific.rs
    digest.rs
    kvm_state.rs
    kvm_state/bindings.rs
    kvm_state/bindings/{clock,irqchip,nested,regs,tables,vcpu}.rs
    kvm_state/{clock,cpu_config,events,fpu,irqchip,lapic,nested,regs,routing,vm}.rs
    manifest.rs
    manifest/header.rs
    manifest/host.rs
    memory.rs
    memory/mapping.rs
    restore.rs
    section.rs
    wire.rs
  arm64/
    command.rs
    control_uart.rs
    control_uart/
      tests.rs
    executor.rs
    mod.rs
    fdt.rs
    gic.rs
    layout.rs
    machine.rs
    protocol.rs
    protocol/
      tests.rs
    response.rs
    response/
      tests.rs
    uart.rs
    vcpu.rs
    watchdog.rs
    watchdog/
      signal.rs
crates/soma-kvm/tests/
  kvm_probe.rs
  x86_64_halt_guest.rs
  x86_64_kernel_boot.rs
  x86_64_kernel_boot/
    discover.rs
    host_sample.rs
    newc.rs
  x86_64_sandbox_boot.rs
  x86_64_sandbox_boot/
    control.rs
    generation.rs
    session.rs
  fixtures/
    arm64_agent.c
    arm64_init.S
    arm64_probe.c
    arm64_process.c
    arm64_process.h
    arm64_process_test.c
    build_command_initramfs.py
    build_initramfs.py
    x86_64/
      build_x86_64_init.py
      x86_64_init.c
```

### `soma-kvm/src/lib.rs`

`lib.rs` provides the target-gated capability interface and stable result types.
It selects supported and unsupported platform behavior at compile time without claiming that successful compilation proves KVM support.
It must remain free of provider policy and per-Machine public contract types.

### `linux.rs`

`linux.rs` owns target-gated Linux KVM capability calls and architecture-specific probe requirements.
The `x86_64/` modules own the machine-contract layout, PVH boot-page encoding, private guest RAM, the bounded ELF and PVH-note parser, the kernel and initramfs loader, the single command-line composer, the CPUID template, bootstrap vCPU state, the diagnostic 16550 model, the checked port bus, the bounded run loop, and the deadline watchdog.
`memory.rs` shares the private RAM mapping between the loader, the vCPU thread, and the device thread as a range-checked `GuestMemory` view that never forms a Rust reference over guest bytes; `mmio.rs` decodes `KVM_EXIT_MMIO` for the five fixed pages, treats an unmapped address as a typed fatal exit, and counts transport violations instead of stopping the machine; `events.rs` owns the five irqfds and eight queue-notify ioeventfds and deregisters them on drop; `event_loop.rs` is the single epoll device thread with a per-wakeup work budget and pass limit; `devices.rs` binds the preopened root and overlay files, the link-down loopback network backend, the assigned vsock CID, and a fresh entropy source to the bus behind one mutex and condition variable; `launch_page.rs` maps, writes, verifies as erased, and retires the dedicated slot; `channel.rs` is the deadline-bounded byte channel over the vsock host endpoint; `sandbox.rs` and its submodules order creation, start, launch-page runtime, milestone timeline, and teardown for one test-only cold-booted Generation.
`watchdog.rs` separates starting the vCPU thread from waiting for it so a control session can proceed while the guest runs and still reclaim the vCPU through the same kick-and-join path.
The `arm64/` modules own only the explicit-fixture cold-boot and challenge-bound command proofs accepted by ADRs 0014 and 0016.
Those abort-capable proofs are crate-internal and reachable only from ignored live tests run in dedicated test processes.
They do not imply authenticated readiness, OCI execution, networking, snapshot restore, isolation certification, or production-engine support.
Every unsafe operation requires a `SAFETY` explanation and tests that exercise its complete invariant.
Guest-controlled lengths, offsets, addresses, and queue data remain hostile even after KVM validates VM creation.

The file must split by cohesive KVM responsibility before it approaches the repository file-size limit.
Likely deep modules include VM ownership, guest-memory registration, vCPU state, interrupt state, clock state, and run-loop exits.
The names are extraction directions rather than empty modules required in advance.

### `virtio/`

`virtio/` owns the common virtio contract selected in the minimal device surface: the modern virtio-mmio version 2 register file, status lifecycle, explicit feature allowlists, split-virtqueue geometry and cursors, hostile descriptor-chain validation, interrupt status, configuration generation, and serializable transport and queue state.
It is pure Rust with no KVM calls, no `unsafe`, and no target gate, so its tests run on every host.

`guest_memory.rs` is the bounded guest-physical access seam.
Every read and write is range-checked against registered regions before any byte moves, and the in-memory `VecGuestMemory` exists for tests and fuzz targets rather than production mapping.
`queue.rs` owns one split virtqueue, while `queue/chain.rs` exposes `walk_chain` as a pure function over a table, head, size, and limits so a fuzz target can drive it directly.
`queue/layout.rs` validates size, alignment, containment, and ring disjointness; `queue/state.rs` is the fixed-width snapshot record; `queue/violation.rs` is the typed rejection set with saturating counters.
`transport.rs` owns the register file and reads, `transport/write.rs` owns driver writes and lifecycle enforcement, `transport/host.rs` owns device-thread completion and interrupt raising, and `transport/state.rs` owns the snapshot record and fail-closed restore.
`device.rs` is the `VirtioDevice` seam that every device model implements; the crate-private echo test device still exercises the transport alone.

`devices/` holds the five v1 device models behind that seam, each as one parser over validated chains, one backend seam that accepts only validated operations, and one fixed little-endian identity record with a version byte.
`devices/segments.rs` copies bytes between validated chain segments and host buffers through the checked guest-memory seam, and `devices/service.rs` is the budgeted loop that pops chains, hands them to a `ChainHandler`, publishes used lengths, skips hostile chains with a counter, and stops the device with `DEVICE_NEEDS_RESET` on a fault.
`devices/block.rs` is virtio-blk for the immutable root and private overlay roles with a request parser in `block/request.rs`, execution in `block/execute.rs`, and the `BlockBackend` seam with a positional-I/O file backend in `block/backend.rs`.
`devices/net.rs` is virtio-net with the all-zero 12-byte header check in `net/frame.rs`, the `NetBackend` seam with a preopened TAP backend and a shared-queue loopback in `net/backend.rs`, and buffer-first receive delivery in `net/rx.rs` behind a host-controlled link gate.
`devices/vsock.rs` is virtio-vsock with header validation in `vsock/packet.rs`, checked credit accounting in `vsock/credit.rs`, the single-connection `HostEndpoint` in `vsock/connection.rs`, guest-to-host handling in `vsock/tx.rs`, packet selection in `vsock/outbound.rs`, and receive and event delivery in `vsock/rx.rs`.
`devices/rng.rs` is virtio-rng with the `EntropyBackend` seam and the `/dev/urandom` source in `rng/backend.rs`.
`bus.rs` is the checked interval dispatcher over the five transports; `bus/table.rs` is the single source of every address, GSI, device identifier, queue count, and the kernel command-line fragment, and `bus/slots.rs` routes notifications and inbound delivery per slot and captures or restores all five slots with device identity validated before transport state.
The `IrqSink` and `NotifySource` traits are the seams the `x86_64` machine implements with irqfd and ioeventfd later.

The module itself registers no ioeventfd or irqfd, decodes no KVM exit, runs no event loop, and owns no versioned snapshot container; the `x86_64` machine supplies those, and the block, vsock, and entropy models have now served a real guest on a cold boot while the network model has run only with its link down.

### `snapshot/`

`snapshot/` owns the snapshot format v1 encoding and validation policy from `docs/research/snapshot-format-v1.md`.
`wire.rs` is the shared big-endian primitive layer: every read checks availability first, every length prefix is bounded before any slice or allocation, and presence bytes accept only zero or one.
`manifest.rs` owns the `SOMASNP\0` header, schema version, architecture, page size, `GenerationId`, contract digests, host-profile requirements, memory descriptor, machine shape, and the canonical ascending section sequence.
`section.rs` owns the role, version, length, digest, and critical-flag envelope and rejects unknown critical roles, duplicates, reordering, unsupported versions, digest mismatches, and trailing bytes.
`kvm_state.rs` and its submodules own SOMA's explicit byte layouts for CPUID, MSRs, registers, segment descriptors, FPU, XCR, bounded XSAVE, LAPIC, MP state, vCPU events, optional nested state, PIC, IOAPIC, GSI routing, KVM clock, optional PIT, and the memory-slot layout.
`kvm_state/bindings/` compiles only on Linux x86_64 and provides checked conversions to and from the `kvm-bindings` structs; union reads there are the only unsafe code besides the private mapping.
`device_state.rs` owns the five fixed device states and excludes every host descriptor, path, socket, key, credit window, packet, and random byte by construction.
`memory.rs` owns the memory-object descriptor and the Linux `MAP_PRIVATE | MAP_NORESERVE` mapping type accepted by ADR 0002.
`compatibility.rs` compares a `HostProfile` with a decoded manifest using exact equality and typed rejection reasons, header fields before any section payload.
`capture.rs` and `restore.rs` are typed ordering contracts only; they have no KVM or device effect and exist so the later live slice cannot reorder quiesce, capture, restore, or unwind steps silently.

### `tests/kvm_probe.rs`

The KVM probe integration test runs only on a probe-capable Linux x86_64 or ARM64 target with accessible `/dev/kvm`.
It must report an explicit skip or prerequisite failure elsewhere rather than silently becoming a platform-neutral pass.
On a valid host, it proves that the deployment identity can open KVM, verify the expected interface, create a VM, and release owned descriptors without leaks when those operations are implemented.

## `soma-macos` responsibilities

`soma-macos` owns the development-only Apple Silicon adapter to Apple's supported container command contract.
It invokes commands directly without a shell, enforces a pinned compatible runtime range, bounds time and output during ingress, and performs unconditional cleanup for one-shot execution.

The crate owns macOS runtime probing, OCI image references, local Machine names, process supervision, run, create, start, execute, stop, delete, and inspect behavior.
It does not certify Linux KVM, x86_64, snapshot restore, the production jail, density, or production latency.

The crate must remain target-safe when present in a portable workspace.
All Apple runtime behavior fails with a typed unsupported-host result outside macOS rather than invoking a similarly named executable.

## `soma-cli` responsibilities

`soma-cli` owns the `soma` executable's grammar, validation at the process boundary, stable JSON envelope, human rendering, and documented exit codes.
It supports one-shot execution, managed lifecycle commands, backend choice, explicit runtime location, host diagnosis, and version reporting as those capabilities land.

The CLI does not become the reusable client library.
Shared lifecycle transactions, backend selection, portable errors, and execution receipts move into the `soma` facade so agents, SDKs, and the CLI call one implementation.
The CLI never inserts an implicit shell and never reports a requested backend as the effective isolation result.

## `soma`, `soma-local`, and `soma-mcp` responsibilities

The portable `soma` library is justified by multiple independent callers: the CLI, agents, SDKs, and control planes.
It owns use cases rather than hypervisor primitives.

Its initial deep modules are one-shot execution, managed Machine lifecycle, network intent, validation, and evidence-carrying receipt construction.
It depends only on portable contracts and does not select a host implementation.
It does not own KVM ioctls, Apple command construction, provider billing, fleet placement, or user-interface rendering.

`soma-local` composes the facade with target-gated built-in adapters and the durable compare-and-swap lifecycle store accepted by ADR 0011.
An unsupported target fails closed during backend selection and cannot construct a weaker fallback.

`soma-mcp` owns bounded stdio framing, tool schemas, cancellation at the caller boundary, and MCP response rendering.
It retains no second lifecycle implementation and invokes the same facade used by the CLI.

## `soma-generation` responsibilities

`soma-generation` owns deterministic import of one selected image from an existing OCI image layout and normalization of its selected layers into one canonical logical tree in a descriptor-relative content-addressed store.
It verifies the selected manifest, configuration, ordered layer descriptors, gzip expansion, and ordered `diff_ids` under explicit descriptor, blob, aggregate, expansion, and traversal bounds.
It accepts exact immutable identity or unique platform selection, fails closed on ambiguity or disagreement, and records local traversal indexes as provenance rather than canonical registry identity.
Normalization reopens and verifies the imported completion and selected layers, applies the supported bounded OCI filesystem semantics without unpacking guest names into a host tree, streams regular-file bodies into CAS, and publishes the canonical tree manifest last.
Its bounded local PAX profile accepts only exact `path` and `linkpath` values, while global, malformed, duplicate, xattr, timestamp, security, and unknown PAX metadata fail closed.
The importer rejects global PAX and mixed local PAX plus GNU naming extensions before normalization.
The `generation/` modules implement Generation compiler phases 1 through 3 and 6 for x86_64: one `TemplateRevision` plus one `NormalizedRootfs` become an EROFS root, sterile ext4 overlay templates, a verified kernel, a deterministic initramfs, and a canonical `SOMAGEN` manifest whose SHA-256 is the `GenerationId`.
Snapshot capture and certification are absent and appear only as typed absent state, so a compiled Generation is not launchable through the public lifecycle; the `soma-kvm` live test cold-boots one directly and proves the artifacts compose into a working guest.
Initramfs layout v2 carries the console and null device nodes and the Generation-scoped responder private key as a fifth machine input, and `open_artifact` opens one published artifact by descriptor for a launcher.

The source map is:

```text
crates/soma-generation/src/
  lib.rs
  digest.rs
  error.rs
  import.rs
  layer_tar.rs
  layer_tar/budget.rs
  layout.rs
  manifest.rs
  normalize.rs
  normalize/entry.rs
  normalize/error.rs
  normalize/layer.rs
  normalize/node_plan.rs
  normalize/pax.rs
  normalize/source.rs
  normalize/stream.rs
  normalize/tree.rs
  normalize/tree/hardlinks.rs
  normalize/tree/mutation.rs
  normalize/tree_manifest.rs
  normalize/tree_model.rs
  normalize/types.rs
  oci.rs
  publish.rs
  publish/layers.rs
  root.rs
  store.rs
  store/staged.rs
  tar_preflight.rs
  traversal.rs
  types.rs
  verify.rs
  generation/mod.rs
  generation/artifacts.rs
  generation/compile.rs
  generation/compile/inputs.rs
  generation/contracts.rs
  generation/erofs.rs
  generation/erofs_reader.rs
  generation/erofs_reader/dir.rs
  generation/erofs_verify.rs
  generation/error.rs
  generation/identity.rs
  generation/initramfs.rs
  generation/kernel.rs
  generation/kernel_config.rs
  generation/manifest.rs
  generation/manifest/decode.rs
  generation/manifest/decode/primitives.rs
  generation/manifest/encode.rs
  generation/overlay.rs
  generation/overlay/verify.rs
  generation/process.rs
  generation/publish.rs
  generation/request.rs
  generation/tar_stream.rs
  generation/template.rs
  generation/tree_decoder.rs
  generation/verify.rs
```

The public results are `ImportedOci`, `NormalizedRootfs`, and `CompiledGeneration`.
`CompiledGeneration` carries a published manifest, its `GenerationId`, formatter and checker evidence, and the typed list of unimplemented phases; `verify_generation` re-verifies every artifact and reports the Generation as not launchable while the snapshot binding is absent.
`NormalizedRootfs` identifies a canonical logical tree and retains its import provenance, but it is not a mounted or bootable root filesystem, `GenerationId`, kernel, guest agent, snapshot, compatibility certificate, or sandbox.
Registry authentication, tag resolution, host extraction, disk-filesystem compilation, signing, SBOM generation, reachability garbage collection, internal store quotas, and certification remain outside this slice.
The crate depends on the portable `soma` identity types and must not depend on `soma-vmm`, a provider adapter, or launch-time policy; its tests additionally depend on `soma-template` so `tests/template_boundary.rs` proves that the `TemplateRevision` view builds this crate's revision and records where the two contracts disagree.

## `soma-template` responsibilities

`soma-template` is the first slice of the Template compiler accepted by ADR 0022, covering tickets T1 through T5 of the [Template implementation map](../research/template-implementation-map.md) in part; the open T1 through T5 deliverables are listed below with T6 through T18.
It parses one `soma.template/v1alpha1` TOML document with unknown-field and unknown-schema rejection, resolves a flat ordered module list plus transitive requirements from a bounded in-memory registry of data-defined modules, composes exclusive ownership, sealed environment values, and one default command, validates the ten rejection classes from the template system design against a policy ceiling, Backend capabilities, an `OciResolver`, and a `FilesystemOracle`, and emits the canonical `SOMALOCK` version 1 lock whose SHA-256 is the `LockId`.
Every rejection names the module and the exact dotted field responsible.
The `TemplateRevision` view projects a lock onto the input contract of the Generation compiler and fails closed where the portable network contract cannot yet state the locked envelope.

The source map is:

```text
crates/soma-template/src/
  lib.rs
  compose.rs
  compose/digest.rs
  compose/graph.rs
  error.rs
  identity.rs
  lock.rs
  lock/decode.rs
  lock/encode.rs
  lock/fields.rs
  lock/verify.rs
  module.rs
  module/builtin.rs
  module/digest.rs
  module/path.rs
  module/reference.rs
  module/registry.rs
  module/spec.rs
  rejection.rs
  rejection/display.rs
  resolve.rs
  revision.rs
  revision/network.rs
  schema.rs
  schema/choice.rs
  schema/command.rs
  schema/parse.rs
  schema/reader.rs
  validate.rs
  validate/backend.rs
  validate/checks.rs
  validate/cidr.rs
  validate/contract.rs
  validate/network.rs
  validate/policy.rs
  validate/secret.rs
  validate/syntax.rs
  wire.rs
crates/soma-template/tests/
  composition.rs
  lock_golden.rs
  lock_hostile.rs
  lock_identity.rs
  lock_shapes.rs
  modules.rs
  network_shapes.rs
  parse.rs
  parse_hostile.rs
  rejections_graph.rs
  rejections_policy.rs
  rejections_secrets.rs
  rejections_targets.rs
  rejections_values.rs
  revision.rs
  support/mod.rs
  fixtures/example-lock.hex
  fixtures/example-lock.id
```

`lib.rs` is the export map only.
`schema.rs` and its submodules own the document model: `schema/reader.rs` is the claim-tracking table reader whose `finish` reports the first unclaimed key with its full path, `schema/parse.rs` maps tables to typed fields with bounds only, `schema/choice.rs` holds the closed choice sets with their stable wire discriminants, and `schema/command.rs` is the bounded default command; unknown keys are rejected during parsing, before validation, with a missing or mistyped required field of the same table reported first.
`module.rs` and its submodules own the common module contract: `module/spec.rs` is the data contract and its builder, `module/reference.rs` parses `soma://<kind>/<name>@<version>`, `module/path.rs` validates guest paths and environment names, `module/registry.rs` is the bounded lookup seam a later content-addressed store implements, `module/builtin.rs` holds the four example modules as data, and `module/digest.rs` is the canonical module encoding whose SHA-256 enters the lock.
`compose.rs` and `compose/graph.rs` own deterministic ordering with cycle, unpinned-input, unknown-module, and duplicate detection, then exclusive-field, owned-path, sealed-environment, and default-command conflict rules, and `compose/digest.rs` is the content digest of the composed selection that the lock binds and `TemplateRevision::with_provenance` recomputes.
`validate.rs` and its submodules run the fixed-order checks: `validate/checks.rs` for platforms, resources, lifecycle, command shape, module values, and the executable check, `validate/contract.rs` for environment, secret, exclusive delivery-target, and required-environment contracts, `validate/network.rs` for envelope normalization and ceiling comparison including domain-pattern and CIDR containment, `validate/cidr.rs` for canonical CIDR text and containment, `validate/policy.rs` for the `PolicyCeiling` input, `validate/backend.rs` for `BackendCapabilities`, `validate/secret.rs` for conservative secret-literal detection over every bound literal, and `validate/syntax.rs` for domain, CIDR, user, mode, path, port, and timeout shapes.
`resolve.rs` owns the `OciResolver` seam, the deterministic `TestResolver`, and the `resolve` entry point that composes, pins, validates, and assembles the lock.
`lock.rs` documents the fixed field order, `lock/encode.rs` and `lock/decode.rs` are the canonical encoder and the hostile bounded decoder, `lock/verify.rs` re-applies the validator's shape rules to every decoded record, `lock/fields.rs` holds the typed locked records, `identity.rs` is `LockId`, `wire.rs` is the shared big-endian primitive layer, and `rejection.rs` is the typed rejection vocabulary with its class mapping and display.
`revision.rs` carries the documented field-by-field mapping onto `soma_generation::generation::template::TemplateRevision`, and `revision/network.rs` is the exact envelope-to-`NetworkPolicy` projection.

The crate depends on `soma`, `toml`, and `sha2` only.
It does not pull an OCI registry, inspect a normalized rootfs, plan a build, construct a Generation, publish anything, or hold a resolved secret value; the resolver and oracle are seams with deterministic test implementations, and the lock records secret references, delivery, and scope only.
Within tickets T1 through T5 the multi-workload golden corpus (T1), module resolution from a content-addressed store (T2), the user, port, process-name, and mount-destination conflict fields and the field-origin `explain` result (T3), the proof that a Launch override narrows but cannot widen the locked ceiling (T4), and the workspace binding (T5) remain open alongside T6 through T18.

## `soma-guest` responsibilities

`soma-guest` owns a fixed portable Noise handshake profile, canonical session binding, bounded encrypted records, canonical application messages, one-use launch-page material, the authenticated control lifecycle, absolute transport deadlines, operation replay protection, redacted errors, and secret wrapper boundaries.
Its public typestate prevents callers from obtaining transport mode before their side of the two-message handshake completes or reusing an in-flight owner.
The first handshake, transport, semantic, lifecycle, accounting, or deadline failure consumes the owner and irreversibly poisons its byte adapter.

The source map is:

```text
crates/soma-guest/src/
  lib.rs
  application/command.rs
  application/frame.rs
  application/guest.rs
  application/host.rs
  application/mod.rs
  application/operation.rs
  application/output.rs
  application/terminal.rs
  binding.rs
  control/channel.rs
  control/deadline.rs
  control/error.rs
  control/exchange.rs
  control/guest.rs
  control/guest_connect.rs
  control/guest_state.rs
  control/host.rs
  control/host_connect.rs
  control/io.rs
  control/mod.rs
  control/operation_ledger.rs
  control/outcome.rs
  control/request.rs
  error.rs
  handshake.rs
  launch_page.rs
  launch_page/network.rs
  launch_page/session.rs
  launch_page/wire.rs
  record.rs
  resolver.rs
  secret.rs
```

The crate is an independent protocol and ownership foundation so the host VMM adapter and the static guest agent share one encoding, lifecycle, deadline contract, and test corpus.
It also fixes the machine-contract constants shared by both peers: the launch-page guest-physical address, the vsock control port, and the schema 2 non-secret `LaunchNetwork` identity accepted by ADR 0023.
It does not contain a guest executable, VMM device transport adapter, trusted Generation-manifest verifier, physical snapshot-safe secret injection, real clone repair, process executor, C ABI, or attestation mechanism.
Its owned authenticated probe state is necessary but cannot authorize a Machine Ready result until those external repair and execution effects are wired and evidenced.

## `soma-guest-agent` responsibilities

`soma-guest-agent` is the statically linked Linux executable that runs as PID 1 inside a SOMA microVM.
It performs the early-init sequence from the Generation compiler contract, waits at the disconnected repair point, consumes the launch page, repairs entropy, transport, identity, and network state in the fixed order, authenticates the host over vsock, runs the fixed readiness probe through the production executor, serves bounded direct commands, and performs authenticated shutdown.
It consumes `soma-guest` for every byte of the launch page, handshake, record, application message, and lifecycle state and reimplements none of them.

The source map is:

```text
crates/soma-guest-agent/src/
  main.rs
  boot.rs
  boot/superblock.rs
  console.rs
  control.rs
  descendants.rs
  entropy.rs
  environment.rs
  executor.rs
  identity.rs
  ioctl.rs
  launch_page.rs
  lifecycle.rs
  mounts.rs
  network_repair.rs
  network_repair/encoding.rs
  output.rs
  pid1.rs
  repair.rs
  shutdown.rs
crates/soma-guest-agent/tests/fixtures/README.md
```

`main.rs` is a composition root only; `repair.rs` owns the typestated controller whose markers make an out-of-order or duplicate transition unrepresentable and whose runtime ledger re-checks the same order.
`boot.rs` owns the bounded early-init sequence with fixed device names, mount options, superblock identity checks, a sterile-head check, and the `switch_root` style move into the composed root.
`launch_page.rs` maps the fixed guest-physical page through `/dev/mem`, copies it once into locked zeroizing memory, erases and verifies the mapping, and only then decodes it.
`entropy.rs`, `identity.rs`, and `network_repair.rs` own one repair effect each over narrow `libc` calls with `SAFETY` comments.
`control.rs` adapts a vsock stream to the protocol crate's `ControlIo` with absolute deadlines; `lifecycle.rs` sequences the probe, Execute, and Shutdown exchanges; `executor.rs`, `output.rs`, `environment.rs`, and `descendants.rs` own direct `execve`, exact output accounting, the fixed environment policy, and complete descendant reaping.
`pid1.rs` and `console.rs` own the never-exit, orphan-reaping, panic-to-poweroff, and bounded-diagnostic duties of init.

The crate compiles on every workspace target but only the Linux modules do work; other targets exit with an unsupported result.
Host tests cover the state machine, page consumption and erasure, output accounting, invocation bounds, transport deadlines, kernel structure layouts, and the executor against host binaries.
Booting as PID 1, composing the root, consuming and erasing the launch page, kernel entropy credit, identity and network installation, the vsock handshake, the readiness probe, one Execute, and authenticated shutdown have each run once inside a cold-booted SOMA machine on x86_64, recorded in [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md); the same path after a snapshot restore remains unproven.
The stop path uses the restart command because the version 1 machine has no ACPI or paravirtual power-off, and `reboot=k` turns it into the reset pulse the VMM observes as the orderly exit.

## `soma-jail` responsibilities

`soma-jail` is the launcher from the [VMM jail profile](../research/vmm-jail-profile.md): it records ownership, creates one cgroup v2 leaf with `memory.max`, `memory.swap.max=0`, `memory.oom.group=1`, `cpu.max`, and `pids.max`, clones the child directly into fresh user, mount, PID, network, IPC, and UTS namespaces and into that leaf with a pidfd, writes single-entry identity maps, and releases the child only after namespace, interface, and membership evidence is read from the parent side.
The pre-exec child sets the parent-death signal, drops to the ephemeral identity, clears dumpable, applies rlimits, enters an empty read-only tmpfs root through `pivot_root` with the old root detached, seals a fixed descriptor table with `dup3` and `close_range`, verifies every slot by `fstat` and device number, installs `no_new_privs` and the startup seccomp filter, and executes the VMM from an open descriptor with `execveat(AT_EMPTY_PATH)`.
Seccomp filters are hand-assembled classic BPF with no libseccomp dependency: the default action is kill-process, `ioctl` is filtered on the request number to exactly the KVM requests the implementation uses, every table entry names whether it was measured or reserved, and the steady-state filter drops the setup-only syscalls and ioctls.

The source map is:

```text
crates/soma-jail/src/
  lib.rs
  bin/jail-probe.rs
  cgroup.rs
  cgroup/error.rs
  cgroup/files.rs
  descriptors.rs
  descriptors/inspect.rs
  evidence.rs
  manifest.rs
  namespaces.rs
  namespaces/root.rs
  process.rs
  process/child.rs
  process/failure.rs
  process/handle.rs
  process/launch_error.rs
  process/prepare.rs
  process/spawn.rs
  process/wait.rs
  reconcile.rs
  report.rs
  seccomp/mod.rs
  seccomp/bpf.rs
  seccomp/denied.rs
  seccomp/install.rs
  seccomp/ioctls.rs
  seccomp/policy.rs
  spec.rs
crates/soma-jail/tests/jail_live.rs
crates/soma-jail/tests/jail_live/containment.rs
crates/soma-jail/tests/jail_live/control.rs
crates/soma-jail/tests/jail_live/failure.rs
crates/soma-jail/tests/jail_live/harness.rs
scripts/jail-live-tests.sh
```

`spec.rs` validates the `JailSpec`; `manifest.rs` fixes the typed descriptor slot order; `descriptors.rs` seals and verifies the table; `namespaces.rs` writes the identity maps, reads parent-side namespace evidence, and owns the `pivot_root` sequence; `cgroup.rs` creates, limits, reads back, kills, and removes one leaf with typed errors when cgroup2 is unavailable or undelegated; `seccomp/` holds the policy tables, the portable BPF assembler with golden-byte tests, and the Linux installer; `process/` is the launcher with its allocation-free pre-exec child, twelve-byte failure report, pidfd-only handle, and typed launch failures; `reconcile.rs` is the idempotent ledger that also recovers after a crashed launcher; `evidence.rs` and `report.rs` are the parent-side evidence type and the probe's line codec.
`jail-probe` is the test stand-in for the VMM: it reports what it can see from inside the jail and executes containment commands over the control socket.
The portable types compile on every target; the Linux mechanisms and the live tests compile only on Linux x86_64, and the live tests are ignored unless run as root inside the privileged container that `scripts/jail-live-tests.sh` prepares.
The crate does not yet wrap the real `soma-vmm` binary, transfer a TAP endpoint, or serve prepared workers, and the allowlist is measured against the musl probe plus `soma-kvm` code rather than against a traced VMM.

## `soma-netd` responsibilities

`soma-netd` is the privileged Linux network broker accepted by ADR 0012 and specified in [the Linux network profile](../research/linux-network-profile-v1.md).
It owns network namespaces, TAP and veth devices, IPAM, routes, nftables rulesets, conntrack zones, resolver policy, ingress port reservations, the durable ownership ledger, repair-gated activation, idempotent release, and reconciliation.
The unprivileged VMM receives exactly one already-open TAP descriptor over `SOCK_SEQPACKET` with `SCM_RIGHTS` and can never create or reconfigure a host device.

The source map is:

```text
crates/soma-netd/src/
  lib.rs
  activate.rs
  bundle.rs
  bundle/prepare.rs
  bundle/types.rs
  cidr.rs
  daemon.rs
  dns.rs
  error.rs
  firewall.rs
  firewall/host.rs
  firewall/tests.rs
  ids.rs
  ingress.rs
  ingress/socket.rs
  intent.rs
  intent/codec.rs
  ipam.rs
  ledger.rs
  ledger/record.rs
  link.rs
  namespace.rs
  netlink.rs
  nft.rs
  profile.rs
  protected.rs
  protocol.rs
  protocol/reply.rs
  reconcile.rs
  release.rs
  sysctl.rs
  tap.rs
  transfer.rs
  transfer/scm.rs
  bin/soma-netd.rs
crates/soma-netd/tests/
  live_linux.rs
  live/codec.rs
  live/frames.rs
  live/mod.rs
  live/world.rs
```

`intent.rs` admits one portable `NetworkPolicy` against the served `NetworkProfile` and fails closed on an unspecified egress or DNS dimension, a proxy profile, a static or IPv6 guest address, a resolver inside the protected set, or a foreign profile selector.
`profile.rs` and `protected.rs` own the operator profile, its content digest, and the certified protected destination set that every egress class drops before any accept.
`ipam.rs` carves `/30` guest and transit leases from the profile plans without index reuse inside one cleanup generation and derives the locally administered MAC pair from the bundle identity.
`firewall.rs` renders the per-bundle sandbox and host `inet` tables as text; `nft.rs` is the version 1 mechanism that feeds that text to the pinned `nft` binary and flushes conntrack zones through the pinned `conntrack` binary.
`namespace.rs`, `tap.rs`, `link.rs`, `netlink.rs`, and `sysctl.rs` are the direct syscall mechanisms: `unshare` plus a bind-mounted pin, `TUNSETIFF`, `ifreq` and `rtentry` `ioctl` calls, a minimal `RTM_NEWLINK` and `RTM_DELLINK` encoder, and `/proc/sys/net` writes inside the namespace of a dedicated thread.
`ledger.rs` is the durable append-only ownership ledger of create-exclusive, synced, hard-linked records; `bundle.rs` prepares sterile bundles and assigns them by recording ownership before any intent-specific kernel change and producing the exact `LaunchNetwork` values.
`activate.rs` verifies namespace, links, rulesets, and forwarding against the ledger after the caller attests authenticated repair and only then raises links, installs routes, and enables forwarding.
`release.rs` and `reconcile.rs` tear down in the specification order with per-step dispositions and compare ledger intent with kernel reality without removing unowned objects.
`transfer.rs` and `daemon.rs` own the bounded typed descriptor handoff and the smallest honest single-threaded daemon over one Unix `SOCK_SEQPACKET` socket.

Portable modules compile and test on every workspace target; every kernel mechanism is Linux-only and the daemon exits with a typed message elsewhere.
Live proofs run only inside the pinned privileged Ubuntu 24.04 container through `scripts/netd-live-tests.sh` and are retained in [the network profile evidence](../evidence/2026-08-29-linux-network-profile-live.md).
Proxy attachment, ingress forwarding, peer authentication of the daemon socket, IPv6 guest addressing, and the libnftnl replacement for the `nft` subprocess remain later slices.

## `soma-storage` responsibilities

`soma-storage` owns the writable disk-head mechanism from the XFS reflink profile: published overlay size classes, exact-class admission, sterile ext4 template creation with a pinned `mke2fs` invocation, descriptor-only `FICLONE` head creation with extent-sharing verification, the two-clone isolation conformance proof, single-use head ownership, durable release, directory reconciliation, and the retained clone-latency matrix.
It is a mechanism crate for the future host allocator and prepared-worker path; it does not decide placement, quotas, or tenant policy and does not touch a Machine.

The source map is:

```text
crates/soma-storage/src/
  lib.rs
  head.rs
  profile.rs
  profile/dimensions.rs
  profile/naming.rs
  profile/storage.rs
  profile/storage/probe.rs
  profile/tests.rs
  template.rs
  template/recipe.rs
  template/tests.rs
  fiemap.rs
  clone.rs
  verify.rs
  lease.rs
  release.rs
  reconcile.rs
  bench.rs
  bench/burst.rs
  bench/cell.rs
  bench/identity.rs
  bench/matrix.rs
  bench/pressure.rs
  bench/record.rs
  bench/report.rs
  bench/stats.rs
  bench/templates.rs
  bin/soma-storage-bench.rs
crates/soma-storage/tests/xfs_live.rs
```

`profile.rs` and its submodules own validated class dimensions, the published `OverlayClass` with its template digest and free-space evidence, the exact-size `ClassCatalog`, and the Linux `StorageProfile::probe` that proves XFS plus a working tiny `FICLONE` before any head is created; a mount without reflink is a typed rejection, never a copy fallback.
`template.rs` runs the pinned `mke2fs` and `e2fsck -fn` subprocesses with a closed environment, a private `mke2fs.conf`, derived UUID and hash seed, and a fixed creation time, then records the SHA-256 of the template; it is operator-side build work and the only module that accepts a template store path.
`clone.rs` takes a template descriptor, a capability directory descriptor, and a validated `HeadName`, creates the destination exclusively, issues `FICLONE`, syncs the file and the directory, proves the apparent size and that every extent is shared through `fiemap.rs`, and transfers the open descriptor; any later failure unlinks the destination and the VMM never learns a path.
`verify.rs` is the conformance proof: two clones of one template take different written patterns, `fdatasync` forces allocation, the template and each peer read back unchanged, and copy-on-write is observed through the extent flags; it also proves that exhausting a clone yields `ENOSPC` with the template digest intact.
`lease.rs` is the single-use ledger: one token owns at most one head in its lifetime and one name is assigned at most once; `release.rs` unlinks under the directory descriptor, syncs the directory, and only then retires the token; `reconcile.rs` reports consistent, orphan, missing, and foreign entries and never deletes.
`bench/` and the `soma-storage-bench` executable run the matrix from the research note with raw JSONL samples and nearest-rank percentiles, and `scripts/xfs-reflink-bench.sh` runs it inside a privileged pinned Ubuntu 24.04 container on a loop-backed XFS image because the development host root filesystem has no reflink support.

Every Linux module keeps its `unsafe` blocks behind `SAFETY` comments; the portable types, ledger, release, and reconciliation compile and test on every workspace target.
The retained loop-backed result in `docs/evidence` is a decision input for prepared sterile heads, not a raw-partition or production-host latency claim.

## `soma-hostd` responsibilities

`soma-hostd` is the node-local host allocator accepted by ADR 0006 and specified in [the prepared worker protocol](../research/prepared-worker-protocol.md).
It owns bounded pools of sterile single-use workers and resource bundles keyed by the exact host profile, Generation, CPU and memory class, overlay class, and network profile, the typestated worker state machine, the single-winner idempotent claim, the exactly-once transfer of fresh per-Instance authority, bounded replenishment and explicit backpressure, the durable append-only lifecycle ledger, restart reconciliation, and the multi-dimension capacity admission of the visual atlas.
It never executes guest device logic, never proxies steady-state Machine commands, and never returns an assigned worker to a pool.

The source map is:

```text
crates/soma-hostd/src/
  lib.rs
  ids.rs
  admission.rs
  admission/capacity.rs
  admission/demand.rs
  admission/numa.rs
  admission/profile.rs
  admission/rejection.rs
  admission/reserve.rs
  admission/shape.rs
  admission/usage.rs
  pool.rs
  pool/backpressure.rs
  pool/capacity.rs
  pool/claim.rs
  pool/claim/error.rs
  pool/claim/registry.rs
  pool/inspect.rs
  pool/key.rs
  pool/launcher.rs
  pool/ledger.rs
  pool/ledger/fold.rs
  pool/ledger/record.rs
  pool/ledger/tests.rs
  pool/reconcile.rs
  pool/release.rs
  pool/release/types.rs
  pool/replenish.rs
  pool/replenish/background.rs
  pool/resources.rs
  pool/state.rs
  pool/state/tests.rs
  pool/state/typestate.rs
  pool/transfer.rs
  pool/transfer/run.rs
  pool/transfer/run/frames.rs
  protocol.rs
  protocol/reply.rs
  protocol/tests.rs
  daemon.rs
  testing.rs
  testing/broker.rs
  testing/broker/launch.rs
  testing/launcher.rs
  testing/table.rs
  bin/soma-hostd.rs
  bin/linux/host.rs
crates/soma-hostd/tests/
  admission.rs
  admission_gates.rs
  atlas/mod.rs
  bounds.rs
  capacity.rs
  claim.rs
  latency.rs
  reconcile.rs
  replay.rs
  support/mod.rs
  transfer.rs
```

`pool/key.rs` is the exact [`PoolKey`] digest; nothing is rounded or widened so a sterile worker can never be prepared for a different contract.
`pool/state.rs` owns the `Constructing`, `Sterile`, `Claiming`, `Assigned`, `Running`, `Destroying`, and `Dead` phases, the legal transition table, the packed atomic state word that every ownership change is one compare-and-swap over, and the ledger projection; `pool/state/typestate.rs` makes only legal transitions compile and bumps the lease generation only on the claim.
`pool/claim.rs` and `pool/claim/registry.rs` own the single-winner claim: the registry serializes idempotency per `OperationId`, a replay with the same fingerprint returns the identical outcome, a changed fingerprint conflicts, a concurrent replay waits at most the claim deadline, and an exhausted pool rejects at once instead of queueing.
`pool/transfer.rs` and its `run` module deliver the eight ordered authority frames, identity, deadline, entropy, launch page, disk head, TAP, control, and commit, each acknowledged before the next; any fault, timeout, partial acknowledgement, deadline, or intent mismatch destroys the worker.
`pool/replenish.rs`, `pool/replenish/background.rs`, and `pool/backpressure.rs` bound construction by concurrency, deadline, target, and maximum and return typed `Exhausted` and `Overloaded` results.
`pool/ledger.rs`, `pool/ledger/record.rs`, and `pool/ledger/fold.rs` are the durable ledger of create-exclusive, checksummed, file-and-directory-synced records and the projection that fails closed on a sterile record after an assignment.
`pool/reconcile.rs` treats every nonterminal entry without a live slot as suspect after a restart, probes the launcher and the brokers, terminates or releases, retains a live running Instance, and gates replenishment until it has run.
`pool/release.rs` tears down assigned, running, claimed, and sterile workers by reason and never returns one to the pool.
`pool/launcher.rs` and `pool/resources.rs` are the seams the jail adapter from decision-map ticket #9 and the live `soma-storage` and `soma-netd` brokers will implement; `testing/` is the in-process launcher with per-step fault injection over a shared process table and the in-process broker that leases heads through the real `soma-storage` ledger over socket pairs.
`pool/capacity.rs` binds one pool to the host [`Admission`] and the exact Machine shape its workers are prepared for, so every claim reserves each visual-atlas dimension atomically before it wins a slot, a refusal is the typed `CapacityRejection` naming the gate, the transfer frees the Launch slot at commit, every teardown returns the reservation, and reconciliation rebuilds the committed usage of each retained Instance.
`admission/` is the visual atlas capacity model: a certified host profile with host reserve, labelled measured per-VM overhead, per-dimension limits, and per-class overcommit ratios; checked demand arithmetic; an atomic multi-dimension reservation that rolls back on the first refusing gate; a typed rejection naming the gate and its numbers; explicit guaranteed and elastic memory classes; a single-node NUMA placement hook; and the capacity ladder estimate.
`protocol.rs` and `daemon.rs` are the bounded typed claim, release, inspect, and reconcile protocol and the single-threaded `SOCK_SEQPACKET` skeleton over one pool; the daemon does not authenticate its peer and starts only with the explicitly requested in-process development launcher, and `bin/linux/host.rs` builds the certified host profile and Machine shape it admits against from explicit operator inventory arguments.

Every module compiles on every workspace target except the Unix descriptor-carrying transfer payload, the Unix in-process testing implementations, and the Linux daemon; a non-Unix host cannot construct a transferable authority bundle at all.
The integration tests prove one winner among 100 concurrent claimers fifty times, identical replay, changed-intent conflict, no reuse of an assigned worker, a fault at every transfer step, immediate exhaustion, bounded replenishment storms, 100-way fairness over ten workers, restart reconciliation, every admission gate, rollback, the atlas capacity ladder, and record claim latency over 1,000 claims without asserting it.
No live VMM, jail, XFS head, or TAP bundle is exercised by this crate yet.

## Future node and protocol modules

ADR 0006 reserved a node-local allocator for admission, unassigned single-use worker allocation, sterile resource bundles, descriptor transfer, and asynchronous replenishment; `soma-hostd` now implements that library and daemon skeleton, and the jail adapter, the live broker adapters, and the descriptor-passing transport remain future work behind its seams.

`soma-protocol` becomes justified when local and remote callers share a bounded canonical encoding.

These names are ownership decisions rather than permission to create empty crates.

## Internal seams and adapters

An internal seam is justified when behavior actually varies or when it protects a deep unsafe responsibility.
The initial real variation is target host access through `platform.rs`, the development-only macOS runtime boundary, and deterministic test adapters.
Future seams require evidence from at least two meaningful adapters or a security reason that makes isolation independently valuable.

Mocks that restate each implementation call are rejected.
Tests replace behavior at the deepest stable seam and continue to use the external Machine interface.

## Generation construction

OCI acquisition, logical rootfs normalization, disk-filesystem compilation, guest boot, quiescence, snapshot capture, certification, and provenance occur outside the request-time VMM.
The implemented importer, normalizer, and compiler establish bounded input, canonical logical-tree, immutable disk-artifact, and manifest-identity workflows but deliberately stop before guest boot, snapshot capture, and certification.
Later Generation stages remain behind the same independently tested module rather than entering Launch latency.

The Template compiler in `soma-template` resolves and locks every mutable or composable input before any of that work begins, and its lock is the only Template artifact the builder accepts.

The Launch path consumes a certified immutable Generation.
It never pulls an OCI image, resolves a mutable tag, installs packages, or hides a Generation build inside warm-start latency.

## Contract extraction

The initial contract lives in `soma-vmm` to maximize locality while semantics are still being discovered.
A separate `soma-contract` crate is justified only when at least two independent consumers need the semantic or encoded types without depending on the VMM implementation, or when protocol release cadence must differ from runtime release cadence.
Extraction must move the interface rather than duplicate or wrap it.

## File and dependency guardrails

- Authored source files stay at or below 300 lines.
- Generated files are isolated and clearly marked rather than mixed with authored behavior.
- `lib.rs` files export and compose but do not accumulate implementation logic.
- Generic `utils.rs`, `helpers.rs`, `common.rs`, `manager.rs`, and `core.rs` dumping grounds are prohibited.
- A public module must hide meaningful complexity and pass the deletion test.
- A new dependency requires a documented role, compatible license, maintained provenance, and review of its unsafe surface.
- Target-only dependencies remain gated so Linux, macOS, and Windows clients can compile without another host's runtime.
- `soma-kvm` never depends on `soma-vmm`.
- `soma-guest-agent` depends only on `soma-guest`, `zeroize`, and `libc`, and never on a host, VMM, or provider crate.
- `soma-storage` depends only on `serde`, `serde_json`, `sha2`, `cap-std`, and Linux `libc`, and never on a VMM, KVM, guest, or provider crate.
- `soma-jail` depends only on `libc`, with `kvm-bindings` as a test-only structure-size oracle, and never on `soma-kvm`, `soma-vmm`, or a provider crate.
- `soma-hostd` depends only on `sha2`, `soma-guest`, `soma-netd`, `soma-storage`, and Linux `libc`, and never on a VMM, KVM, or provider crate.
- `soma-template` depends only on `soma`, `toml`, and `sha2`, and never on a VMM, Backend, registry client, or provider crate; the Generation builder consumes its lock rather than the reverse.
- Provider adapters and ComputeSDK integration never become dependencies of either VMM crate.

## Review checklist

Reviewers should ask these questions before accepting a module change:

1. Does the caller learn less than the implementation handles?
2. Does the seam concentrate change and verification in one place?
3. Would deleting the module scatter its invariants across callers?
4. Is the adapter protecting real variation rather than forwarding calls?
5. Can tests exercise the behavior through the same interface as callers?
6. Does the change preserve one process per Machine, constrained host allocation, and the three-command Machine interface?
7. Does any file now own unrelated contract, lifecycle, KVM, guest, device, or provider concerns?
8. Does the documentation distinguish implemented evidence from future direction?
