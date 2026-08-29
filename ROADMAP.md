# SOMA roadmap

SOMA's north star is the state-of-the-art hardware-isolated sandbox engine across clouds, bare-metal operators, workload images, machine shapes, and storage sizes.
This roadmap sequences the evidence required to reach that outcome without treating an architectural document as an implementation claim.

No phase is production-supported until the security policy names a supported release explicitly.

## Phase 0: Contract foundation

Outcome: one provider-neutral lifecycle contract, portable caller surface, and truthful development backend establish the product boundary without claiming production KVM restore.

Exit evidence:

- Validated `Launch`, `Execute`, and `Stop` request types.
- Exact-request idempotency and conflict behavior.
- Monotonic milestones and typed recovery directives.
- Deterministic tests proving Ready is impossible before the complete repair and readiness sequence.
- Target-gated Ubuntu 24.04 x86_64 production KVM probe and Linux ARM64 development KVM probe.
- Test-only Linux ARM64 direct-boot and challenge-bound command tracer bullets with retained teardown evidence.
- Development-only Apple Silicon VM-per-OCI execution for local lifecycle conformance.
- Portable Linux, macOS, and Windows client-library and command-line compilation gates.
- One-shot and managed-Machine use cases with typed backend selection.
- Versioned evidence-carrying execution receipts with redaction and cleanup classification.
- Open-source governance, threat model, architecture decisions, CI, and contribution checks.

## Phase 1: Minimal KVM machine

Outcome: `soma-vmm` direct-boots a managed Linux kernel on Ubuntu 24.04 x86_64 KVM.

Exit evidence:

- One KVM VM file descriptor and one OS thread per vCPU.
- Certified guest memory layout, CPUID policy, interrupt controller, clock, and boot parameters.
- Minimal serial diagnostics with no readiness claim.
- Checked KVM state transitions and bounded hostile-input handling.
- Real-host tests for create, boot, stop, crash, timeout, and repeated cleanup.

## Phase 2: Minimal devices and guest control

Outcome: a managed guest can access its immutable root, receive entropy and networking, authenticate, and execute bounded commands.

Exit evidence:

- Audited virtio block, net, vsock, and rng paths with fuzz and boundary tests.
- Authenticated guest-agent protocol bound to Generation and the globally unique Instance lifetime.
- Direct non-interactive execution with bounded input, output, and time.
- Repair barrier that prevents user work before fresh identity.
- End-to-end `Launch`, `Execute`, and `Stop` through the public transport.

## Phase 3: Certified restore

Outcome: one immutable Generation restores safely into many independent Machines without copying full guest memory or disk state.

Exit evidence:

- Versioned machine, device, and memory snapshot format.
- Exact compatibility fingerprint and fail-closed corruption behavior.
- Immutable memory artifact mapped privately without eager population.
- Sparse private root head with proven copy-on-write isolation.
- Cross-Instance mutation, identity, entropy, and connection-isolation tests.
- Cold-cache and warm-cache working-set measurements.

## Phase 4: Host isolation, prepared resources, and recovery

Outcome: a hostile guest-facing VMM is constrained to one Machine and every failure leaves auditable cleanup evidence.

Exit evidence:

- Dedicated user, mount, PID, network, and IPC isolation.
- Cgroup v2 resource enforcement.
- Capability removal, `no_new_privs`, and thread-appropriate seccomp.
- Descriptor-relative artifact access and immutable-file evidence.
- PID-reuse-safe ownership and parent-death handling.
- Failure injection for every resource-acquisition boundary.
- Sharded single-use prepared-worker pools with exact ownership transfer.
- Sterile cgroup, namespace, network, disk-head, and control-channel resource bundles.
- Pool-miss, saturation, replenishment, and allocator-crash behavior.
- Independent security and architecture review.

## Phase 5: Generation pipeline

Outcome: an OCI digest can be converted reproducibly into a certified SOMA Generation outside Launch latency.

Exit evidence:

- Digest-pinned OCI acquisition and provenance.
- Reproducible root normalization and kernel selection.
- Controlled boot, quiesce, capture, repair-contract binding, and certification.
- Shape-family compatibility evidence.
- Signed manifest, SBOM, build provenance, revocation identity, and retained test results.
- No registry credential or mutable tag in the VMM protocol.

## Phase 6: Burst performance

Outcome: SOMA meets every internal and exact external latency target without weakening correctness or hiding preparation.

Exit evidence:

- 100 concurrent `node:22` Machines with 100 successful commands and cleanups.
- One valid execution receipt per sample with exact workload, backend, isolation, preparation, timing, result, and cleanup evidence.
- Raw median, p95, p99, errors, cleanup, and stage samples.
- Prepared worker acquisition and dispatch p50 below 0.10 ms and p99 below 0.50 ms.
- Additive server-side create budget below 3.25 ms p50 and 8.90 ms p99.
- Complete server-side create p50 below 5 ms and p99 below 10 ms.
- First bounded command p50 below 10 ms and p99 below 20 ms from accepted Launch.
- Exact ComputeSDK Burst TTI median below 50 ms and p99 below 90 ms.
- Separate cold-cache, warm-cache, prepared-resource, paused-pool, and ready-pool experiments.
- Unique identity, private mutable state, and network isolation across the full cohort.
- No silent retry, cold fallback, omitted sample, or shifted timer boundary.
- At least 100 complete engineering bursts and 10,000 retained samples in addition to the exact external benchmark cohort.

## Phase 7: Portable substrate matrix

Outcome: clients on Linux, macOS, and Windows can use the same local or remote contract across certified clouds, host classes, architectures, filesystems, and shape families.

Exit evidence:

- Published host and Generation conformance matrix.
- Authenticated bounded remote transport with idempotent disconnect recovery and receipt preservation.
- Linux, macOS, and Windows client behavior conformance with target-gated local engines.
- Provider-neutral compute, storage, and network leases.
- Additional adapters isolated behind the same security invariants.
- ARM64 engine support and new host support only after architecture-specific interrupt, timer, boot, restore, isolation, and burst evidence beyond the development capability probe.
- Performance claims reported per exact substrate rather than blended into one universal number.

## Admission rule

Every roadmap change must preserve or strengthen hardware isolation, private mutable state, authenticated readiness, exact compatibility, idempotent recovery, and honest measurement.
A faster result that weakens one of those properties does not advance SOMA toward the north star.
