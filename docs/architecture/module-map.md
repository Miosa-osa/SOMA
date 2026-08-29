# SOMA module map

Read [From hardware to an agent sandbox](beginners-guide.md) first if the distinction between a sandbox, Machine, Instance, VMM, KVM, Template, Generation, and Snapshot is not already clear.

## Purpose and status

This document assigns responsibilities and dependency direction for the initial pre-alpha workspace.
It prevents lifecycle, KVM, protocol, and provider concerns from accumulating in one god file.
It is a code ownership map rather than a claim that the complete VMM, restore path, device model, or production security architecture already exists.

The current workspace contains nine implemented crates:

```text
crates/
  soma/
  soma-cli/
  soma-generation/
  soma-guest/
  soma-kvm/
  soma-local/
  soma-macos/
  soma-mcp/
  soma-vmm/
```

The current alpha contains a portable use-case facade, durable local lifecycle state, a semantic Machine-contract slice, Linux KVM capability probes, explicit-fixture ARM64 KVM cold-boot and challenge-bound direct-command proofs, a development-only macOS VM-per-OCI backend, a verified bounded local OCI-layout importer, a deterministic normalized logical rootfs artifact, portable authenticated-session primitives, a command-line adapter, and a bounded stdio MCP adapter.
It does not yet contain the production x86_64 guest boot path, snapshot restore implementation, production device model, host allocator, complete Generation builder, authenticated guest agent, snapshot-safe secret injection, or remote transport.

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
    soma-guest       independent protocol foundation
```

The portable `soma` facade owns use-case orchestration and execution-receipt construction.
`soma-cli` owns only command-line parsing, human and JSON rendering, and process exit behavior over that facade.
`soma-mcp` maps bounded MCP tools onto that same facade.
`soma-local` owns durable local lifecycle state, backend selection, and target-gated local composition.
`soma-vmm` owns the provider-neutral Machine interface and the deep lifecycle implementation.
`soma-kvm` owns target-gated access to Linux x86_64 production KVM capabilities and Linux ARM64 development KVM capabilities.
`soma-macos` owns the development-only Apple VM-per-OCI lifecycle adapter.
`soma-generation` verifies bounded OCI image-layout input and publishes immutable imported and normalized logical-tree artifacts without minting a certified Generation.
`soma-guest` owns the portable authenticated-session and encrypted-record primitives without claiming a live guest agent or readiness.
`soma-kvm` must not depend on `soma-vmm`, provider control planes, OCI clients, or benchmark code.
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
Its current depth includes a checked capability probe, an x86_64 machine floor that maps one private memory slot, enters one protected-mode vCPU, captures port-I/O exits and `hlt`, and enforces a watchdog deadline with proven cleanup, plus explicit-fixture ARM64 direct-boot and command paths with checked memory layout, vCPU initialization, GICv3, timer and device-tree description, separate diagnostic and control UARTs, strict challenge-bound frames, direct guest execution, and bounded teardown.
Its current depth includes a checked capability probe plus explicit-fixture ARM64 direct-boot and command paths with checked memory layout, vCPU initialization, GICv3, timer and device-tree description, separate diagnostic and control UARTs, strict challenge-bound frames, direct guest execution, and bounded teardown.
It also contains a target-independent, `unsafe`-free modern virtio-mmio version 2 transport and split-virtqueue implementation under `virtio/` that is exercised only by host-side tests and is not yet wired to an MMIO bus, KVM exit, device model, or event loop.
As real restore work arrives, the crate will own KVM VM creation, vCPU creation, memory-slot registration, register restoration, interrupt-controller state, clock state, and the target-specific execution loop.
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
    error.rs
    guest.rs
    kick.rs
    layout.rs
    memory.rs
    mod.rs
    run.rs
    vcpu.rs
    watchdog.rs
  virtio/
    mod.rs
    device.rs
    device/
      test_device.rs
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
  fixtures/
    arm64_agent.c
    arm64_init.S
    arm64_probe.c
    arm64_process.c
    arm64_process.h
    arm64_process_test.c
    build_command_initramfs.py
    build_initramfs.py
```

### `soma-kvm/src/lib.rs`

`lib.rs` provides the target-gated capability interface and stable result types.
It selects supported and unsupported platform behavior at compile time without claiming that successful compilation proves KVM support.
It must remain free of provider policy and per-Machine public contract types.

### `linux.rs`

`linux.rs` owns target-gated Linux KVM capability calls and architecture-specific probe requirements.
The `x86_64/` modules own the machine-contract layout, PVH boot-page encoding, private guest RAM, bootstrap vCPU state, the bounded run loop, and the deadline watchdog for the halt-guest floor; they boot no kernel and expose no device.
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
`device.rs` is the `VirtioDevice` seam that a block, network, vsock, or entropy model implements; the only implementation today is a crate-private echo test device.

The module does not implement an MMIO bus, ioeventfd or irqfd registration, a device backend, an event loop, or the versioned snapshot container, and passing its tests proves none of those.
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
```

The public results are `ImportedOci` and `NormalizedRootfs`.
`NormalizedRootfs` identifies a canonical logical tree and retains its import provenance, but it is not a mounted or bootable root filesystem, `GenerationId`, kernel, guest agent, snapshot, compatibility certificate, or sandbox.
Registry authentication, tag resolution, host extraction, disk-filesystem compilation, signing, SBOM generation, reachability garbage collection, internal store quotas, and certification remain outside this slice.
The crate depends on the portable `soma` identity types and must not depend on `soma-vmm`, a provider adapter, or launch-time policy.

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
  launch_page/wire.rs
  record.rs
  resolver.rs
  secret.rs
```

The crate is an independent protocol and ownership foundation so the future host VMM adapter and static guest agent share one encoding, lifecycle, deadline contract, and test corpus.
It does not contain a guest executable, VMM device transport adapter, trusted Generation-manifest verifier, physical snapshot-safe secret injection, real clone repair, process executor, C ABI, or attestation mechanism.
Its owned authenticated probe state is necessary but cannot authorize a Machine Ready result until those external repair and execution effects are wired and evidenced.

## Future node and protocol modules

ADR 0006 reserves `soma-host` for node-local admission, unassigned single-use worker allocation, sterile resource bundles, descriptor transfer, and asynchronous replenishment.
That module does not execute guest device logic and does not retain tenant workers for reuse.

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
The implemented importer and normalizer establish bounded input and canonical logical-tree workflows but deliberately stop before disk construction or Generation identity.
Later Generation stages remain behind the same independently tested module rather than entering Launch latency.

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
