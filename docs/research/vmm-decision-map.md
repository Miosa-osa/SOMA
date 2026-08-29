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

Resolved; cold-boot machine contract proven on x86_64.
Version 1 uses a pinned uncompressed x86_64 Linux ELF kernel with a PVH entry note, one bootstrap vCPU, a fixed low-memory layout, no firmware, and no general PC platform.
It defines exact boot structures, initial register state, required KVM capabilities, cold-boot and restore ordering, failure behavior, and unsupported devices.
The memory-slot, PVH boot-page, protected-mode vCPU entry, port-I/O exit, `hlt`, watchdog, and cleanup floor is implemented in `soma-kvm` and retained in [the x86_64 halt-guest evidence](../evidence/2026-08-29-x86_64-kvm-halt-guest.md).
The acceptance test in [the x86_64 PVH kernel-boot evidence](../evidence/2026-08-29-x86_64-pvh-kernel-boot.md) boots the pinned kernel through the PVH entry to a challenge-bound serial sentinel with proven descriptor cleanup.
That proof is a cold-boot machine-contract test only; devices, root filesystem, guest agent, readiness, and snapshot restore remain open under later tickets.
See [x86_64 machine contract](x86_64-machine-contract.md).

## #5: What is the minimal device surface?

Blocked by: #4
Type: Research

### Question

Which devices are required for immutable root storage, private writable state, networking, entropy, shutdown, and authenticated control?

### Answer

Resolved.
Version 1 exposes exactly five modern virtio-mmio version 2 devices at fixed addresses with dedicated interrupts: one immutable EROFS root block device, one private ext4 overlay block device, one network device, one vsock control device, and one entropy device.
It uses split queues with fixed limits, explicit feature allowlists, ioeventfd notifications, irqfd interrupts, hostile descriptor validation, quiescent capture, and a fail-closed restore order.
PCI, legacy virtio, hotplug, vhost, optional high-complexity features, and separate control or shutdown devices are excluded.
Status: the five device models, the bus, ioeventfd and irqfd wiring, and the bounded device thread are implemented, and a real x86_64 guest discovered all five devices, mounted the EROFS root and the ext4 overlay, drew entropy, and ran an authenticated vsock session on a cold boot; the network device has only run behind the link-down loopback backend, and the hostile-input, snapshot-restore, forced-cleanup, and latency evidence items remain open.
See [minimal device surface](minimal-device-surface.md) and [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md).

## #6: How is an OCI image turned into a bootable Generation?

Blocked by: #4, #5
Type: Prototype

### Question

How do the existing verified OCI import and normalized rootfs become a deterministic read-only guest filesystem, kernel, initramfs, guest agent, and certified compatibility manifest?

### Answer

Resolved at the architecture and prototype boundary.
Version 1 compiles the normalized OCI tree into a deterministic immutable EROFS lower filesystem and gives each Instance a private ext4 OverlayFS upper device selected from certified size classes.
The pipeline consumes a canonical Template Lock that has already resolved the exact OCI platform digest, modules, command, declared launch inputs, policy ceiling, resource defaults, lifecycle defaults, and provenance.
Kernel, deterministic initramfs, guest agent, root and overlay artifacts, machine and device contracts, CPU template, command line, guest protocols, snapshot state, repair policy, builder provenance, and artifact descriptors are bound into one canonical `GenerationId` manifest.
The retained prototype proved byte-identical EROFS output from logically identical trees created in opposite insertion orders and recorded the populated-ext4 reproducibility failure that caused the two-device correction.
Status: phases 1 through 3 and 6 are implemented, and phase 4 is partial, because a compiled Generation now cold-boots on KVM to an authenticated guest agent and one bounded command, while the quiesce, memory capture, certification, and launchable-manifest steps remain unimplemented and the responder public key is not yet bound into the manifest.
See [Generation compiler](generation-compiler.md), [ADR 0018](../adr/0018-content-addressed-oci-import.md), [ADR 0019](../adr/0019-deterministic-normalized-rootfs.md), and [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md).

## #7: What snapshot format and memory restore mechanism meet the target?

Blocked by: #4, #5, #6
Type: Research

### Question

How are memory, vCPU, interrupt, clock, and device states captured, authenticated, versioned, mapped privately, and restored without copying guest RAM?

### Answer

Resolved architecturally.
Version 1 uses one immutable page-aligned memory object mapped `MAP_PRIVATE | MAP_NORESERVE`, one canonical typed state manifest, separately managed disks, exact compatibility rejection, authority exclusion, quiescent capture, and fixed fail-closed restore ordering.
See [snapshot format v1](snapshot-format-v1.md), [ADR 0002](../adr/0002-private-copy-on-write-memory-restore.md), and [fast path](../architecture/fast-path.md).

## #8: How does the guest become a fresh authenticated Instance?

Blocked by: #6, #7
Type: Prototype

### Question

How does a cloned guest replace identity, entropy assumptions, time state, network state, stale sessions, and captured authority before user code executes?

### Answer

Resolved architecturally.
The static guest agent owns early mounts, one-use launch material, entropy and identity repair, fresh Noise-authenticated vsock control, direct bounded execution, shutdown, and the only path to Ready.
Declared environment values, secret delivery, uploads, and workspace attachments occur only after fresh identity and authenticated repair and never become reusable snapshot authority.
Status: the launch-page delivery and retirement, entropy, identity, and network repair, the Noise handshake over vsock, the readiness probe, one bounded Execute, and authenticated shutdown have live x86_64 evidence on a cold boot; the same path after a snapshot restore, which is the ticket's actual question, remains unproven because no snapshot has been captured.
See [Linux guest integration](linux-guest-agent-integration.md) and [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md).
See [ADR 0017](../adr/0017-authenticated-guest-session.md), [ADR 0020](../adr/0020-launch-page-and-application-wire-contracts.md), and [ADR 0021](../adr/0021-own-authenticated-control-lifecycle.md).

## #9: What host isolation contains a compromised guest-facing VMM?

Blocked by: #4, #5
Type: Research

### Question

What user, namespace, cgroup, capability, seccomp, filesystem, descriptor, resource-limit, and parent-death policy constrains one `soma-vmm` process?

### Answer

Resolved and implemented as the `soma-jail` launcher.
One ephemeral UID, complete namespace and cgroup containment, descriptor-only resources, pidfd ownership, no ambient capability, empty root, and phase-derived seccomp constrain each single-use VMM.
The launcher passed its fifteen privileged live acceptance tests on Ubuntu 24.04 x86_64 inside a privileged container on 2026-08-29; it constrains the static `jail-probe` stand-in and does not yet wrap the real `soma-vmm` binary, transfer a TAP endpoint, or serve prepared workers.
See [VMM jail profile](vmm-jail-profile.md), [threat model](../threat-model.md), and [the live evidence](../evidence/2026-08-29-vmm-jail-live.md).

## #10: What networking path is both fast and fail closed?

Blocked by: #5, #8, #9
Type: Prototype

### Question

How are namespace, TAP, address, route, DNS, egress, proxy, ingress, metadata protection, and cleanup resources prepared and activated inside the latency budget?

### Answer

Resolved architecturally; first implementation retained.
The privileged broker owns sterile network bundles, atomic Instance assignment, protected destinations, readiness-gated activation, typed evidence, idempotent release, and crash reconciliation while the VMM receives only one TAP descriptor.
It enforces an effective policy that placement already narrowed against the Template ceiling, organization policy, caller authority, and Backend capabilities.
`crates/soma-netd` implements the broker library and a minimal daemon, and the privileged-container run in [the network profile evidence](../evidence/2026-08-29-linux-network-profile-live.md) proves the down-until-activation, metadata, peer, and DNS policy, complete release, and the 100-way burst.
Proxy attachment, ingress forwarding, daemon peer authentication, jailed VMM transfer, and virtio-net attach remain open.
See [Linux network profile](linux-network-profile-v1.md), [ADR 0012](../adr/0012-fail-closed-networking.md), and [topology](../architecture/topology.md).

## #11: How are writable disks created privately within the tail budget?

Blocked by: #6, #7, #9
Type: Prototype

### Question

Can XFS `FICLONE` create isolated writable heads with acceptable p99 under realistic image size, extent count, concurrency, and cleanup pressure?

### Answer

Resolved by measurement.
Writable state uses certified sterile ext4 size classes cloned privately through XFS `FICLONE`; `crates/soma-storage` implements the profile, template, clone, verify, lease, release, and reconcile modules with live proofs on a loop-backed `reflink=1` filesystem.
The retained matrix of 69 cells with 200 raw samples each and zero failures put the best 100-way complete-clone p99 at 9.9 ms and the worst at 1,868 ms against the 1.00 ms disk share of fresh resource activation, and no single-clone cell fit either, so on-demand cloning is not admitted and prepared sterile heads are mandatory.
Clones of one template serialize on the template inode, the `ioctl` cost scales with the source extent count rather than the template size, and `FICLONE` maps unwritten template extents as holes, so heads come from sterile templates through an asynchronously replenished pool and never carry a capacity reservation.
See [XFS reflink profile](xfs-reflink-profile.md) and [the XFS reflink evidence](../evidence/2026-08-29-xfs-reflink-profile.md).

## #12: How are prepared workers allocated without reusing tenant state?

Blocked by: #7, #9, #10, #11
Type: Prototype

### Question

Which invariant work may move outside Launch, and how does one request atomically claim one sterile worker and resource bundle?

### Answer

Resolved architecturally.
Bounded pools hold only sterile invariant state, use one generation-counted single-winner claim, transfer fresh authority exactly once, destroy ambiguous workers, reject overload, and reconcile before replenishment.
See [prepared worker protocol](prepared-worker-protocol.md) and [ADR 0006](../adr/0006-prepared-worker-allocation.md).

## #13: How is the complete KVM backend wired into SOMA?

Blocked by: #6, #8, #9, #10, #11, #12
Type: Prototype

### Question

How does the Linux implementation satisfy the existing portable Resolve, Launch, Execute, Inspect, Stop, and Destroy contracts without weaker fallback behavior?

### Answer

Resolved architecturally.
The KVM adapter composes Generation resolution, admission, prepared ownership, restore, repair, execution, inspection, shutdown, cleanup, evidence, retries, and reconciliation behind the unchanged portable lifecycle.
Template and module resolution remain outside the adapter, which receives only an exact certified Generation, effective policy, and fresh launch bindings.
See [KVM backend integration](kvm-backend-integration.md).

## #14: What evidence admits the first production host profile?

Blocked by: #13
Type: Research

### Question

Which correctness, isolation, cleanup, adversarial, concurrency, compatibility, and performance results are required before describing SOMA as a working custom VMM sandbox?

### Answer

Resolved architecturally.
A signed immutable report binds exact provenance and admits a HostProfile only after mandatory correctness, isolation, failure, cleanup, concurrency, compatibility, and raw performance gates pass with no skipped KVM tests.
See [production admission evidence](production-admission-evidence.md), [benchmark contract](../benchmark-contract.md), and [validation template](../operations/validation-report-template.md).

## #15: How does one certified node become a scalable service?

Blocked by: #12, #14
Type: Research

### Question

How should admission, placement, cells, capacity reservations, Generation distribution, remote authentication, failure containment, and observability scale from one node to 100,000 concurrent sandboxes?

### Answer

Resolved architecturally.
The fleet uses bounded independent cells, capability-filtered placement, host-authoritative admission, idempotent operations, signed Generation distribution, reserved capacity, explicit overload, reconciliation, and staged scale gates up to 100,000 concurrent sandboxes.
The fleet also distributes immutable Template Locks, Generation readiness and revocation state, compatibility evidence, leases, and cache intent while resolving aliases before placement and keeping mutable Template logic off Hosts.
See [fleet control plane](fleet-control-plane.md).
