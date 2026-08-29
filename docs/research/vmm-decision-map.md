# SOMA custom VMM decision map

This map is the canonical sequence for turning SOMA's existing research into a Linux KVM implementation.
Each unresolved ticket is sized for one focused agent session and must produce the linked asset before dependent implementation begins.

## #1: What performance boundary does SOMA optimize?

Blocked by: none
Type: Research

### Question

Does the 10 ms objective measure a VMM ioctl, VM resume, authenticated readiness, or a successful first command?

### Answer

Resolved.
SOMA targets complete server-side create below 5 ms p50 and 10 ms p99 on a certified warm host.
The stricter user-value boundary is the first bounded command below 10 ms p50 and 20 ms p99 from accepted Launch.
Image download, OCI conversion, first disk construction, and cold kernel boot are separate measurements.
See [fast path](../architecture/fast-path.md), [benchmark contract](../benchmark-contract.md), and [ADR 0006](../adr/0006-prepared-worker-allocation.md).

## #2: What owns one sandbox?

Blocked by: #1
Type: Research

### Question

Should one process own one VM, should one daemon own many VMs, or should device processes be split immediately?

### Answer

Resolved.
One native `soma-vmm` process owns exactly one VM and one dedicated host thread enters `KVM_RUN` for each vCPU.
A node-local allocator may prepare sterile workers, but an assigned worker is single-use and destroyed after its tenant.
See [topology](../architecture/topology.md), [ADR 0001](../adr/0001-direct-per-machine-interface.md), and [ADR 0006](../adr/0006-prepared-worker-allocation.md).

## #3: What is the first certified host profile?

Blocked by: #1
Type: Research

### Question

Which operating system, architecture, virtualization interface, and filesystem form the first production contract?

### Answer

Resolved.
The first profile is Ubuntu 24.04 x86_64 with Linux KVM on a host that exposes readable and writable `/dev/kvm` to the exact VMM identity.
XFS reflink is the initial private disk-head candidate.
Apple virtualization and Docker remain development adapters and do not certify this profile.
See [portability](../architecture/portability.md), [deployment portability](../operations/deployment-portability.md), and [Linux handoff](../operations/linux-vmm-handoff.md).

## #4: What exact x86_64 machine contract boots first?

Blocked by: #2, #3
Type: Research

### Question

What guest-physical layout, Linux boot protocol, vCPU register state, interrupt controller state, clock state, kernel command line, and initramfs layout define the smallest supported x86_64 machine?

### Answer

Open.
Produce `docs/research/x86_64-machine-contract.md` with primary-source citations, a byte-level memory map, required KVM capabilities and ioctls, restore ordering, failure behavior, and an explicit list of unsupported PC devices.
The first prototype must boot a pinned kernel directly without BIOS, UEFI, PCI, ACPI, graphics, USB, or hotplug.

## #5: What is the minimal device surface?

Blocked by: #4
Type: Research

### Question

Which devices are required for immutable root storage, private writable state, networking, entropy, shutdown, and authenticated control?

### Answer

Open.
Produce `docs/research/minimal-device-surface.md` comparing virtio-mmio and PCI transport for x86_64 and selecting only the required block, net, vsock, rng, and control mechanisms.
Every selected device must include queue bounds, interrupt behavior, hostile-input validation, snapshot state, and restore order.

## #6: How is an OCI image turned into a bootable Generation?

Blocked by: #4, #5
Type: Prototype

### Question

How do the existing verified OCI import and normalized rootfs become a deterministic read-only guest filesystem, kernel, initramfs, guest agent, and certified compatibility manifest?

### Answer

Partially resolved.
OCI selection, content verification, and deterministic logical rootfs normalization exist.
Produce `docs/research/generation-compiler.md` and a prototype deterministic filesystem compiler with reproducibility tests.
The output must bind kernel, command line, filesystem identity, machine contract, guest protocol, and snapshot format into one content-addressed `GenerationId`.
See [ADR 0018](../adr/0018-content-addressed-oci-import.md) and [ADR 0019](../adr/0019-deterministic-normalized-rootfs.md).

## #7: What snapshot format and memory restore mechanism meet the target?

Blocked by: #4, #5, #6
Type: Research

### Question

How are memory, vCPU, interrupt, clock, and device states captured, authenticated, versioned, mapped privately, and restored without copying guest RAM?

### Answer

Partially resolved.
Immutable file-backed `MAP_PRIVATE | MAP_NORESERVE` memory is the first design, while `userfaultfd` remains experimental.
Produce `docs/research/snapshot-format-v1.md` defining canonical metadata, artifact digests, compatibility rejection, restore ordering, private writable state, crash consistency, and rollback.
See [ADR 0002](../adr/0002-private-copy-on-write-memory-restore.md) and [fast path](../architecture/fast-path.md).

## #8: How does the guest become a fresh authenticated Instance?

Blocked by: #6, #7
Type: Prototype

### Question

How does a cloned guest replace identity, entropy assumptions, time state, network state, stale sessions, and captured authority before user code executes?

### Answer

Partially resolved.
The bounded application protocol, launch page, Noise-based authenticated session, repair exchange, readiness probe, and control ownership model exist as portable code.
Produce `docs/research/linux-guest-agent-integration.md` and run the current guest protocol inside the pinned Linux guest.
Ready requires a fresh authenticated session, completed repair, and one successful bounded command.
See [ADR 0017](../adr/0017-authenticated-guest-session.md), [ADR 0020](../adr/0020-launch-page-and-application-wire-contracts.md), and [ADR 0021](../adr/0021-own-authenticated-control-lifecycle.md).

## #9: What host isolation contains a compromised guest-facing VMM?

Blocked by: #4, #5
Type: Research

### Question

What user, namespace, cgroup, capability, seccomp, filesystem, descriptor, resource-limit, and parent-death policy constrains one `soma-vmm` process?

### Answer

Open.
Produce `docs/research/vmm-jail-profile.md` with the exact syscall inventory derived from the implemented fast path, not a copied generic allowlist.
The profile must cover startup, steady state, failure, timeout, cleanup, crash, and diagnostic collection.
See [threat model](../threat-model.md).

## #10: What networking path is both fast and fail closed?

Blocked by: #5, #8, #9
Type: Prototype

### Question

How are namespace, TAP, address, route, DNS, egress, proxy, ingress, metadata protection, and cleanup resources prepared and activated inside the latency budget?

### Answer

Partially resolved.
The portable network contract and fail-closed evidence model exist.
Produce `docs/research/linux-network-profile-v1.md` and a Linux prototype with a preallocated sterile resource bundle, atomic assignment, explicit metadata blocking, and idempotent cleanup.
See [ADR 0012](../adr/0012-fail-closed-networking.md) and [topology](../architecture/topology.md).

## #11: How are writable disks created privately within the tail budget?

Blocked by: #6, #7, #9
Type: Prototype

### Question

Can XFS `FICLONE` create isolated writable heads with acceptable p99 under realistic image size, extent count, concurrency, and cleanup pressure?

### Answer

Open.
Produce `docs/research/xfs-reflink-profile.md`, raw benchmark samples, filesystem and mount identity, isolation tests, crash tests, and a decision between on-demand cloning and sterile precreated heads.
Failure to prove copy-on-write isolation must reject the host profile.

## #12: How are prepared workers allocated without reusing tenant state?

Blocked by: #7, #9, #10, #11
Type: Prototype

### Question

Which invariant work may move outside Launch, and how does one request atomically claim one sterile worker and resource bundle?

### Answer

Partially resolved.
Prepared workers may contain invariant executable, jail, descriptor, allocator, and read-only Generation state only.
Produce `docs/research/prepared-worker-protocol.md` and a bounded node-local prototype proving single-winner assignment, no tenant-state reuse, replenishment, crash recovery, and backpressure.
See [ADR 0006](../adr/0006-prepared-worker-allocation.md).

## #13: How is the complete KVM backend wired into SOMA?

Blocked by: #6, #8, #9, #10, #11, #12
Type: Prototype

### Question

How does the Linux implementation satisfy the existing portable Resolve, Launch, Execute, Inspect, Stop, and Destroy contracts without weaker fallback behavior?

### Answer

Open.
Replace the current unsupported KVM lifecycle with an adapter over `soma-vmm`, `soma-kvm`, the Generation store, authenticated guest control, network ownership, and cleanup evidence.
Add real Linux-only end-to-end tests for Ubuntu and Node 22.
Do not change the portable contract merely to expose KVM sequencing.

## #14: What evidence admits the first production host profile?

Blocked by: #13
Type: Research

### Question

Which correctness, isolation, cleanup, adversarial, concurrency, compatibility, and performance results are required before describing SOMA as a working custom VMM sandbox?

### Answer

Partially resolved.
The benchmark boundary and post-deployment checklist exist.
Produce a completed validation report containing raw samples for cold boot, warm restore, authenticated readiness, first command, cleanup, 100-way bursts, failures, and resource leakage.
No skipped KVM test counts as passing evidence.
See [benchmark contract](../benchmark-contract.md), [post-deployment validation](../operations/post-deployment-validation.md), and [validation template](../operations/validation-report-template.md).

## #15: How does one certified node become a scalable service?

Blocked by: #12, #14
Type: Research

### Question

How should admission, placement, cells, capacity reservations, Generation distribution, remote authentication, failure containment, and observability scale from one node to 100,000 concurrent sandboxes?

### Answer

Open.
Produce `docs/research/fleet-control-plane.md` only after the node-local lifecycle passes #14.
The control plane remains outside the VMM and must preserve caller operation identity, host-profile compatibility, bounded admission, cell isolation, and explicit capacity exhaustion.
