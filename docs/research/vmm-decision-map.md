# SOMA custom VMM decision map

This map is the canonical sequence for turning SOMA's existing research into a Linux KVM implementation.
Each unresolved ticket is sized for one focused agent session and must produce the linked asset before dependent implementation begins.

Every status sentence below uses one of the five terms defined in [the engineering standard](../standards/sota-engineering-standard.md#status-vocabulary): designed, component-tested, live-proved, integrated, production-admitted.
A live-proved sentence names the commit the run was made on and links its evidence, and a run whose code has since changed is called historical.
[The claim ledger](../claim-ledger.md) carries the same statuses in one table.

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

Resolved; the cold-boot machine contract was live-proved on x86_64 at `0b43bc6` and `45d031c`, and both runs are historical because the boot path changed after them.
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
Status: the five device models, the bus, ioeventfd and irqfd wiring, and the bounded device thread are component-tested.
Four of the five are live-proved at `71161ea`, where a real x86_64 guest discovered all five devices, mounted the EROFS root and the ext4 overlay, drew entropy, and ran an authenticated vsock session on a cold boot; that run is historical because it predates initramfs layout v3 and launch-page schema 3.
The network device is component-tested only, having run behind the link-down loopback backend, and the hostile-input, forced-cleanup, and latency evidence items remain designed.
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
Status: phases 1 through 3 and 6 are component-tested, and phase 4 is partial.
A compiled Generation Candidate cold-booting on KVM to an authenticated guest agent and one bounded command is live-proved at `71161ea` and historical, because that run used initramfs layout v2 and its recorded `GenerationId` values are no longer reproducible.
Certification and the ready manifest are designed only; `certify_candidate` fails closed as unimplemented.
ADR 0026 keeps that incomplete work in the Candidate namespace, so nothing resolvable as a Generation exists yet.
Under [ADR 0024, per-Instance guest responder authority](../adr/0024-per-instance-guest-responder-authority.md) no responder key belongs in the manifest at all: the Generation carries public identity only and the Host generates fresh responder authority for every Instance.
See [Generation compiler](generation-compiler.md), [ADR 0018](../adr/0018-content-addressed-oci-import.md), [ADR 0019](../adr/0019-deterministic-normalized-rootfs.md), and [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md).

## #7: What snapshot format and memory restore mechanism meet the target?

Blocked by: #4, #5, #6
Type: Research

### Question

How are memory, vCPU, interrupt, clock, and device states captured, authenticated, versioned, mapped privately, and restored without copying guest RAM?

### Answer

Resolved architecturally; capture and restore are live-proved at `5d71524` on the current per-Instance authority design.
Version 1 uses one immutable page-aligned memory object mapped `MAP_PRIVATE | MAP_NORESERVE`, one canonical typed state manifest, separately managed disks, exact compatibility rejection, authority exclusion, quiescent capture, and fixed fail-closed restore ordering.
A real `node:22` Generation is booted to its disconnected repair point, captured before any launch material exists, and restored into independent authenticated Instances that execute a command. The current retained result is [the capture and restore on the per-Instance authority design](../evidence/2026-08-30-x86_64-snapshot-restore-current-authority.md) at `5d71524`, whose object scan shows no Instance responder identity in `memory.raw`, `overlay.raw`, or `state.somasnap`; [the `7c1127d` run](../evidence/2026-08-29-x86_64-snapshot-restore.md) is retained as historical because it predates ADR 0024.
That observation stands as recorded, but it cannot certify current bytes: the captured Generation still carried a Generation-scoped responder private key in `memory.raw`, which ADR 0024 removed, and the restored ready transition has since been bound to an authenticated readiness receipt.
On the current authority design capture and restore are therefore component-tested, and recapture is finding P1.5 of [the re-audit](../reviews/2026-08-29-implementation-reaudit.md).
See [snapshot format v1](snapshot-format-v1.md), [ADR 0002](../adr/0002-private-copy-on-write-memory-restore.md), [ADR 0030](../adr/0030-pre-launch-snapshot-capture-point.md), and [fast path](../architecture/fast-path.md).

## #8: How does the guest become a fresh authenticated Instance?

Blocked by: #6, #7
Type: Prototype

### Question

How does a cloned guest replace identity, entropy assumptions, time state, network state, stale sessions, and captured authority before user code executes?

### Answer

Designed, with the parts below carrying their own status.
The static guest agent owns early mounts, one-use launch material, entropy and identity repair, fresh Noise-authenticated vsock control, direct bounded execution, shutdown, and the only path to Ready.
Declared environment values, secret delivery, uploads, and workspace attachments occur only after fresh identity and authenticated repair and never become reusable snapshot authority.
Status: the launch-page delivery and retirement, entropy, identity, and network repair, the Noise handshake over vsock, the readiness probe, one bounded Execute, and authenticated shutdown are live-proved on a cold boot at `71161ea`, and that run is historical because it predates initramfs layout v3 and launch-page schema 3.
The same path after a snapshot restore, which is the ticket's actual question, is live-proved at `5d71524`: restores of one captured `node:22` Generation each reached Ready, ran a command, and shut down, including two proved to be independent Instances.
On the current per-Instance authority design and the current readiness-receipt transition, the restored repair path is component-tested and awaits the recapture required by finding P1.5 of [the re-audit](../reviews/2026-08-29-implementation-reaudit.md).
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
The launcher is live-proved at `bd0234e` by fifteen privileged acceptance tests on Ubuntu 24.04 x86_64 inside a privileged container; it constrains the static `jail-probe` stand-in, so a jail around the real `soma-vmm` binary, TAP transfer, and prepared-worker service remain designed.
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
`crates/soma-netd` implements the broker library and a daemon, and the privileged-container run in [the network profile evidence](../evidence/2026-08-29-linux-network-profile-live.md) is live-proved at `bceeb7b` for the down-until-activation, metadata, peer, and DNS policy, complete release, and the 100-way burst; that run is historical because it predates daemon peer authorization and receipt-gated activation.
Peer authorization and the single-use activation capability are component-tested, and no privileged live run of them has been retained.
Proxy attachment, ingress forwarding, jailed VMM transfer, and virtio-net attach are designed.
See [Linux network profile](linux-network-profile-v1.md), [ADR 0012](../adr/0012-fail-closed-networking.md), and [topology](../architecture/topology.md).

## #11: How are writable disks created privately within the tail budget?

Blocked by: #6, #7, #9
Type: Prototype

### Question

Can XFS `FICLONE` create isolated writable heads with acceptable p99 under realistic image size, extent count, concurrency, and cleanup pressure?

### Answer

Resolved by measurement; the storage profile is live-proved at `f91f219`.
Writable state uses certified sterile ext4 size classes cloned privately through XFS `FICLONE`; `crates/soma-storage` implements the profile, template, clone, verify, lease, release, and reconcile modules, proved on a loop-backed `reflink=1` filesystem.
The production launch path does not yet consume prepared heads, so end-to-end use is designed.
The retained matrix of 69 cells with 200 raw samples each and zero failures put the best 100-way complete-clone p99 at 9.9 ms and the worst at 1,868 ms against the 1.00 ms disk share of fresh resource activation, and no single-clone cell fit either, so on-demand cloning is not admitted and prepared sterile heads are mandatory.
Clones of one template serialize on the template inode, the `ioctl` cost scales with the source extent count rather than the template size, and `FICLONE` maps unwritten template extents as holes, so heads come from sterile templates through an asynchronously replenished pool and never carry a capacity reservation.
See [XFS reflink profile](xfs-reflink-profile.md) and [the XFS reflink evidence](../evidence/2026-08-29-xfs-reflink-profile.md).

## #12: How are prepared workers allocated without reusing tenant state?

Blocked by: #7, #9, #10, #11
Type: Prototype

### Question

Which invariant work may move outside Launch, and how does one request atomically claim one sterile worker and resource bundle?

### Answer

Component-tested as a node-local library and daemon skeleton.
`crates/soma-hostd` holds bounded pools of sterile invariant state keyed by the exact host profile, Generation, CPU and memory class, overlay class, and network profile, decides ownership with one compare-and-swap over the worker and its monotonically increasing lease generation, returns the identical outcome to a replayed operation and a typed conflict to a changed intent, transfers identity, deadline, entropy, launch page, disk head, TAP, control, and commit exactly once with every fault destroying the worker, rejects exhaustion and overload without a queue, records every step in a durable checksummed ledger, reconciles every nonterminal entry and rebuilds the committed capacity of every retained Instance before replenishing after a restart, and admits capacity atomically across every visual-atlas dimension with a typed rejection naming the gate on the claim path itself, so no worker is granted for an Instance the host cannot admit.
The jail launcher is pending ticket #9 and the live 100-way proof with a real VMM, XFS heads, and TAP bundles is pending ticket #13; until then the launcher and broker seams are exercised by in-process implementations and the daemon starts only with the explicitly requested development launcher.
See [prepared worker protocol](prepared-worker-protocol.md), [ADR 0006](../adr/0006-prepared-worker-allocation.md), and [the module map](../architecture/module-map.md).

## #13: How is the complete KVM backend wired into SOMA?

Blocked by: #6, #8, #9, #10, #11, #12
Type: Prototype

### Question

How does the Linux implementation satisfy the existing portable Resolve, Launch, Execute, Inspect, Stop, and Destroy contracts without weaker fallback behavior?

### Answer

Designed. The first vertical slices are live-proved; the adapter this ticket specifies is not built.

What runs today, at `08e4d45`: `soma --backend kvm run node:22` resolves a prepared Candidate, cold boots a machine, executes one bounded command inside it, and releases every owned resource. The retained result is [the KVM Backend serving one sandbox through the public command line](../evidence/2026-08-30-kvm-backend-cli-run.md). In the slice order above that is Ubuntu cold boot and file read, authenticated command, and private disk.

What this ticket still requires, and the slice above does not do:

- Launch restores a snapshot. The slice cold boots, which is why it reaches Ready in roughly 646 ms rather than the restore figures measured separately.
- Resolve selects a certified Generation. Certification does not exist, so the slice resolves a Candidate and reports a null `generation_id`.
- Capacity admission, durable OperationId ownership recorded before any external effect, and rejection of a conflicting request fingerprint on retry.
- A claimed prepared worker and sterile bundle, rather than a machine created per request.
- A network lease and activation. The guest device is link down.
- Stop and Destroy as separate operations with independently reported dispositions, and reconcile after restart.
- The eleven adapter modules this document names; the slice is a single lifecycle path.

Two boundaries in the slice are deliberate rather than temporary. The machine and its authenticated session are owned by one thread, because the host adapter borrows the machine to retire the launch page and a structure holding both would refer to itself; a session outliving the process belongs to the daemon in #12. And resolution reads a Generation prepared before demand rather than acquiring an image, because no image acquisition exists in the workspace and the request path is required not to perform one.
The KVM adapter composes Generation resolution, admission, prepared ownership, restore, repair, execution, inspection, shutdown, cleanup, evidence, retries, and reconciliation behind the unchanged portable lifecycle.
Template and module resolution remain outside the adapter, which receives only an exact certified Generation, effective policy, and fresh launch bindings.
See [KVM backend integration](kvm-backend-integration.md).

## #14: What evidence admits the first production host profile?

Blocked by: #13
Type: Research

### Question

Which correctness, isolation, cleanup, adversarial, concurrency, compatibility, and performance results are required before describing SOMA as a working custom VMM sandbox?

### Answer

Designed, except the burst harness, which is live-proved at `ccf7bcf` against the Docker Backend only.
A signed immutable report binds exact provenance and admits a HostProfile only after mandatory correctness, isolation, failure, cleanup, concurrency, compatibility, and raw performance gates pass with no skipped KVM tests.
`benchmarks/local_alpha/burst` runs the exact contract profile the moment a Backend is reachable from the `soma` command line: N iterations at concurrency C with every slot of a burst released by one barrier, the timer starting before the create call and stopping after the workload command succeeded in the sandbox, destruction executed and verified outside the timer, and every attempted sample retained with its stage milestones, exact command output, and typed failure reason.
The contract's anti-gaming rules are enforced in code rather than in prose, and the report generator refuses an incomplete run, a class-mixed run, a successful sample without a zero-exit workload command, and a warm class that recorded no preparation.
The harness is live-proved today only against the Docker Backend, which is a Linux container and not a virtual machine.
[The dry run](../evidence/2026-08-30-burst-harness-dry-run.md) is that proof of the harness and is not a SOMA performance result.
The KVM lifecycle probe at `a4eea45` could not complete, which is recorded honestly in [the blocked burst attempt](../evidence/2026-08-30-burst-against-kvm-blocked.md).
The harness drives Launch, Execute, and Destroy as separate processes, while the development Backend owns its Machine inside the creating process.
[ADR 0031](../adr/0031-persistent-host-runtime-ownership.md) therefore makes `soma-hostd` the persistent owner of managed Instances and makes CLI, MCP, and provider adapters its clients.
That ownership seam is necessary but not sufficient: the admitted campaign also depends on the certified Generation, jail, prepared restore, private resource, authenticated Ready, networking, recovery, and cleanup work in tickets #6 through #13 and Stages 3 through 6 of the audit road.
The failed probe was a cold boot and cannot be classified as cold-cache restore.
The exact ComputeSDK campaign additionally requires a provider adapter and an unmodified upstream run after the local lifecycle passes.
No signed report, admission policy, or revocation state exists yet, and the harness covers the burst performance gate of this ticket only.
See [production admission evidence](production-admission-evidence.md), [benchmark contract](../benchmark-contract.md), and [validation template](../operations/validation-report-template.md).

## #15: How does one certified node become a scalable service?

Blocked by: #12, #14
Type: Research

### Question

How should admission, placement, cells, capacity reservations, Generation distribution, remote authentication, failure containment, and observability scale from one node to 100,000 concurrent sandboxes?

### Answer

Designed.
The fleet uses bounded independent cells, capability-filtered placement, host-authoritative admission, idempotent operations, signed Generation distribution, reserved capacity, explicit overload, reconciliation, and staged scale gates up to 100,000 concurrent sandboxes.
The fleet also distributes immutable Template Locks, Generation readiness and revocation state, compatibility evidence, leases, and cache intent while resolving aliases before placement and keeping mutable Template logic off Hosts.
See [fleet control plane](fleet-control-plane.md).

## #16: How does an authored Template become a canonical Template Lock?

Blocked by: #6
Type: Prototype

### Question

How is one versioned Template document parsed, composed with focused modules, validated against organization policy and Backend capability, and reduced to one deterministic content identity that the Generation compiler consumes?

### Answer

Slice 1 is component-tested; later slices are designed.
`crates/soma-template` parses one `soma.template/v1alpha1` document with unknown-field and unknown-schema rejection, composes a flat ordered module list with transitive requirements from a bounded in-memory registry, rejects duplicate exclusive ownership, conflicting default commands, conflicting sealed environment values, cycles, unpinned inputs, and every other class listed under required validation with the module and field named, pins the OCI platform digest through an `OciResolver` seam, and emits the canonical `SOMALOCK` version 1 lock whose SHA-256 is the `LockId`.
The lock binds the resolved digest and platform, ordered module identities and digests, the effective command, resources, normalized network envelope, lifecycle, environment contract, secret references, the policy ceiling, and the Backend capabilities; it excludes the Template name, description, mutable image text, and every resolved secret value, and conservative secret-literal detection over environment values, secret sources and scopes, command fields, the description, and module sealed values rejects a committed credential of a recognised shape.
Golden bytes, repeated-resolution equality, reordering and renaming identity tests, one test per rejection class, and lock prefix, bit-flip, and garbage sweeps are retained in the crate.
The registry, resolver, and filesystem oracle are seams with deterministic test implementations only, so no registry pull, rootfs inspection, build plan, Generation construction from a lock, publication, or remote resolution has been implemented.
Those are tickets T6 through T18 of the [Template implementation map](template-implementation-map.md); within T1 through T5 the multi-workload golden corpus, module resolution from a content-addressed store, the user, port, process-name, and mount-destination conflict fields, the field-origin `explain` result, the Launch-narrowing proof, and the workspace binding are still open.
Ticket #6 consumes the lock through the `TemplateRevision` view documented in the crate.
See [SOMA template system](../architecture/template-system.md) and [ADR 0022](../adr/0022-compose-templates-into-generation-locks.md).
