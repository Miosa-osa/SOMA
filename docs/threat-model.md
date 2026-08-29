# SOMA threat model

## Status

This threat model defines the intended security architecture.
It does not claim that the pre-alpha implementation currently satisfies it.

## Protected assets

- Host kernel integrity.
- Host and operator credentials.
- Other tenants' memory, disks, network traffic, and metadata.
- Generation integrity and provenance.
- Control-plane authority and lifecycle receipts.
- Availability of the host and its existing machines.

## Adversaries

- Hostile code running with root privileges inside the guest.
- A malicious or compromised guest kernel.
- A malicious OCI filesystem prepared by an untrusted tenant.
- Corrupt or replayed snapshot state.
- Malformed guest-agent messages.
- A caller with permission to launch one Generation but not another.
- A process racing filesystem paths, PIDs, sockets, or cleanup operations.
- A workload attempting resource exhaustion or cross-tenant observation.

## Trust assumptions

- The host kernel, KVM implementation, CPU virtualization extensions, boot chain, and SOMA release are trusted.
- The operator supplies authenticated policy and certified artifact identities.
- The Generation builder runs in a controlled environment and records provenance.
- The root filesystem, guest kernel state, device state, and all guest messages become hostile at the VMM seam.
- A Linux namespace or cgroup alone is not a complete security boundary.

## Hard invariants

- One VMM process owns one tenant machine.
- No mutable memory or disk head is shared across Instances.
- A Generation is immutable after certification.
- A snapshot restores only under an exact compatible runtime identity.
- Guest-controlled memory ranges are validated before every host access.
- The guest receives no operator credential by default.
- The machine cannot reach cloud metadata or another tenant through an unfiltered default route.
- Readiness is impossible before clone Repair succeeds.
- Cleanup cannot target a resource that is not owned by the matching launch receipt.
- A timeout cannot authorize blind retry or deletion.

## Defense layers

1. Hardware virtualization separates guest privilege from host privilege.
2. A minimal device model reduces guest-controlled parser and DMA surface.
3. A dedicated process limits a VMM compromise to one machine identity.
4. User, mount, PID, and network isolation restrict host visibility.
5. Cgroup v2 limits memory, CPU, process, and I/O consumption.
6. Capabilities and privileges are dropped before guest execution.
7. Seccomp restricts each thread to the syscalls required by its role.
8. Immutable artifacts and private mappings prevent cross-instance mutation.
9. Authenticated guest control prevents an unrelated process from forging readiness.
10. An ownership ledger makes recovery and cleanup explicit.

## Explicitly rejected shortcuts

- Embedding the VMM in a BEAM NIF.
- A shared multi-tenant VMM process.
- Writable shared snapshot memory.
- Reusing cloned machine identity or entropy state.
- Disabling containment because a memory file lacks a path.
- Trusting a guest-reported ready string without channel authentication and generation binding.
- Passing registry or cloud credentials into the guest image builder through the launch protocol.
- Treating successful VM resume as successful sandbox creation.

## Required evidence before production use

- Independent code and architecture review.
- Continuous fuzzing for protocol, manifest, snapshot, device, and guest-memory parsers.
- KVM end-to-end tests on every supported host kernel and CPU class.
- Guest escape and cross-tenant network tests.
- Snapshot compatibility and corruption tests.
- Identity uniqueness tests across repeated and concurrent restores.
- Resource exhaustion, timeout, crash, parent-death, and ambiguous-outcome tests.
- Reproducible builds, dependency policy, SBOM, provenance, and signed releases.

## Current pre-alpha gaps

The OCI importer verifies bounded source artifacts and expanded tar structure, but it does not yet extract or normalize a root filesystem, apply whiteouts, certify a Generation, or protect a caller-supplied root whose ambient parent is controlled by an attacker.
Its content store requires exclusive trusted writer authority because portable hard-link publication cannot bind the already verified open handle directly to the final name.
The current tar dependency can allocate a bounded extension body before SOMA observes its effective metadata, so arbitrary hostile-image admission remains prohibited until a certified build profile or parser seam constrains that allocation independently.
The authenticated-session crate binds one Instance secret to an exact launch transcript, but the VMM cannot yet inject that secret after restore and no guest executable consumes the protocol.
Snow does not guarantee complete erasure of every internal key copy, so process destruction remains part of the future one-Machine containment boundary.
No current protocol result satisfies Ready, and no untrusted production workload should be admitted.
