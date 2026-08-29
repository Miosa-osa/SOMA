# Implementation audit handoff

- Date: 2026-08-29
- Repository: `Miosa-osa/SOMA`
- Review fixed point: `e0b894b`
- Last reviewed implementation commit: `4879517`
- Review status: Action required
- Audience: Agent continuing the Linux KVM, guest-agent, and Generation work

## Purpose

This document records actionable feedback from a two-axis review of the recent x86_64 KVM boot, launch-page identity, static guest agent, Generation compiler, and Template-binding work.
It is the implementation handoff for correcting the reviewed work before additional features are layered on top.

The audit reviewed these implementation and evidence commits:

- `2eaff85` - x86_64 PVH kernel boot.
- `45d031c` - retained kernel-boot evidence.
- `60df57c` - launch-page network identity schema.
- `abc7034` - static Linux PID 1 guest agent.
- `858240a` and `181d6e8` - ADR 0023 correction.
- `e19bdde` - Generation compiler phases 1 through 3 and 6.
- `4879517` - Template revision binding in the Generation manifest.

Documentation-only commits from the reviewing agent were not judged as part of the implementation series.

## Executive judgment

The architecture and modularity are strong.
The x86_64 PVH kernel boot is the strongest completed slice and its evidence is appropriately limited.
The OCI normalization, content-addressed storage, protocol ownership, and hostile-input testing provide a serious foundation.

The guest-agent and Generation paths must not be treated as integrated or complete.
They contain security, resource-bounding, compatibility-validation, architecture-portability, and publication-order defects.
New feature work should pause until the Priority 0 defects below are corrected and verified.

## What is good and should be preserved

### x86_64 machine floor

The KVM boot path uses a bounded ELF parser, explicit PVH layout, pinned kernel input, bounded diagnostic serial model, vCPU watchdog, and retained Linux-host evidence.
The evidence distinguishes a kernel-boot proof from a working device, guest-agent, sandbox, restore, or performance claim.
Preserve that evidence discipline.

### Module shape

The implementation separates ELF parsing, command-line construction, CPUID, serial, timing, guest repair, launch-page decoding, EROFS construction, overlay construction, manifest encoding, verification, and publication.
Continue using focused internal modules behind narrow interfaces.
Do not create a Template, Generation, guest-agent, or VMM god module while fixing the findings.

### OCI and immutable-input work

The existing importer and normalizer reject many hostile archive, path, link, whiteout, metadata, budget, and corruption cases.
The Generation work correctly attempts deterministic identities and cross-artifact verification.
Preserve the rule that Launch never pulls an OCI image, invokes a package manager, or resolves a mutable tag.

### Guest protocol

The portable guest protocol provides authenticated sessions, replay resistance, one-use operation identities, bounded messages, exact output accounting, typestate ownership, failure poisoning, and fixed readiness semantics.
Keep cryptographic framing and lifecycle ownership inside the deep `soma-guest` interface rather than duplicating them in the guest executable or VMM.

## Priority 0 blockers

These findings must be fixed before the guest agent or Generation pipeline is integrated further.

### P0.1 Remove private guest identity from published Generation artifacts

#### Finding

`crates/soma-guest-agent/src/boot.rs` expects the responder private key at `/etc/soma/responder.key` inside the initramfs.
`crates/soma-generation/src/generation/compile.rs` stores that initramfs as a content-addressed artifact.

ADR 0017 states that a private key embedded in a publicly retrievable artifact cannot prove exclusive Generation possession.
A retrievable initramfs containing the responder private key therefore breaks the intended authentication foundation unless a different confidential-distribution contract is explicitly designed and proven.

#### Required correction

- A reusable Generation contains public identity only.
- Fresh Instance-specific secret authority crosses a dedicated nonsnapshot launch mechanism after Host ownership is established.
- Snapshot capture excludes live session keys and launch authority.
- Restore creates a new Instance PSK and new authenticated session.
- Failure to erase and retire launch authority destroys the Instance.

#### Acceptance gates

- Artifact inspection proves that no responder private key or Instance PSK exists in the kernel, initramfs, root filesystem, manifest, snapshot, log, receipt, or public store.
- Two Instances from one Generation authenticate with different fresh authority.
- A party possessing every public Generation artifact cannot impersonate either guest.
- ADR 0017, ADR 0020, ADR 0021, and ADR 0023 remain mutually consistent after the correction.

### P0.2 Bound hostile guest output before allocation and queueing

#### Finding

`crates/soma-guest-agent/src/executor.rs::pump` creates unbounded `std::sync::mpsc` channels.
Reader threads allocate a `Vec` for every stdout and stderr chunk before `stream_output` applies the command output allowance.
A hostile process can produce output faster than PID 1 consumes it and exhaust guest memory.

#### Required correction

- Use bounded queues or a single bounded polling loop with backpressure.
- Reserve output budget before allocating or copying a chunk into retained or queued storage.
- Stop reading and terminate the entire process group as soon as the allowance is reached or the sink fails.
- Bound all post-kill draining and joining.
- Preserve exact stdout and stderr accounting for the bytes successfully delivered.

#### Acceptance gates

- A hostile infinite-output process cannot increase resident guest memory beyond a declared constant bound plus fixed buffers.
- Both stdout and stderr competing at full speed remain bounded.
- Output-limit, sink-failure, timeout, and disconnect tests prove process-group termination and bounded completion.
- PID 1 remains alive and able to accept or reject the next valid lifecycle operation according to the protocol contract.

### P0.3 Make Generation build deadlines terminate complete process trees

#### Finding

`crates/soma-generation/src/generation/process.rs` kills only the direct child on timeout or feed failure.
Descendants may retain stdout or stderr descriptors, causing scoped reader-thread joins to block forever.
`wait_bounded` also reports `CompilePhase::FormatRoot` for failures originating in other phases.

#### Required correction

- Start every external tool in an isolated process group or equivalent containment unit.
- Terminate the complete group on timeout, feed failure, capture failure, or cancellation.
- Bound termination grace, drain, wait, and join operations.
- Return the actual invocation phase in every error.
- Ensure interrupted work publishes no ready or complete artifact.

#### Acceptance gates

- A fixture that forks descendants holding both output pipes cannot exceed the declared deadline plus bounded grace.
- A fixture that ignores the first termination signal is forcibly removed.
- Root, overlay, verification, and version-probe failures retain their correct phases.
- No process or descriptor survives a failed build operation.

### P0.4 Separate Generation Candidate from certified Generation publication

#### Finding

`crates/soma-generation/src/generation/compile.rs::compile_generation` calls `publish_manifest` while returning `BootAndCapture` and `Certification` as unimplemented.
The Generation compiler design requires boot and capture, certification, then manifest-last publication.

The current object is honestly marked non-launchable, but its publication and naming still permit incomplete material to be discovered as a Generation.

#### Required correction

- Store incomplete work under an explicit Candidate identity and namespace, or keep it private to the build transaction.
- Publish a ready Generation manifest only after boot, capture, compatibility, security, and certification gates succeed.
- Make ready publication atomic and manifest-last.
- Ensure failure leaves no discoverable ready Generation.

#### Acceptance gates

- Registry and Host resolution cannot return a Candidate for Launch.
- Every failure point before certification leaves no ready Generation identity.
- Concurrent identical builders publish one identical ready object after complete certification.
- Revoked or failed Candidates cannot be promoted without rerunning the required gates.

### P0.5 Validate every hostile Generation compatibility field

#### Finding

`crates/soma-generation/src/generation/verify.rs::require_profile` verifies selected constants but accepts invalid or inconsistent security-critical fields.
Missing validation includes memory size and alignment, memory-slot and launch-page versions, TTL bounds, overlay minimum and maximum consistency, network-policy bindings, and workload-probe semantics.

#### Required correction

- Treat every decoded manifest field as hostile until validated.
- Validate numeric bounds with checked arithmetic.
- Validate version support explicitly.
- Validate cross-field relationships rather than fields independently.
- Reject unknown security-critical contracts.
- Return one typed redacted rejection reason per failed invariant.

#### Acceptance gates

- Every manifest field has a positive, negative, boundary, corruption, and cross-field test.
- A truncation sweep and targeted bit mutations never panic or bypass validation.
- The Host rejects an otherwise correctly signed manifest that is incompatible with its exact HostProfile.
- Verification cannot produce `launchable = true` before certification and compatibility evidence are complete.

## Priority 1 correctness and portability

### P1.1 Correct entropy crediting

`crates/soma-guest-agent/src/entropy.rs` combines 64 bytes read from `/dev/hwrng` with a 64-byte launch seed and credits all 1,024 bits through `RNDADDENTROPY`.
The launch seed may be mixed into the kernel pool but must not receive entropy credit unless its entropy source and delivery are independently proven.

Credit only the trusted fresh entropy contribution.
Add tests for a zero seed, repeated seed, unavailable HWRNG, short HWRNG read, failed reseed, and post-repair nonblocking `getrandom`.

### P1.2 Gate or replace architecture-specific ioctl layouts

`crates/soma-guest-agent/src/network_repair/encoding.rs` manually encodes classic x86_64 Linux `ifreq` and `rtentry` layouts while the crate is gated only to Linux.
Do not pass these layouts to an ARM64 Linux kernel.

Gate the implementation and binary to the exact supported target or provide verified per-target bindings and layout tests.
Cross-compile checks must prove unsupported targets fail at build or capability detection rather than reaching an unsafe ioctl.

### P1.3 Reject unusable subnet addresses

`crates/soma-guest/src/launch_page/network.rs` checks common unicast properties and subnet equality but does not reject the subnet network or broadcast address.

Reject unusable guest, gateway, and resolver values according to the declared IPv4 profile.
Add explicit `/32`, `/31`, network-address, broadcast-address, gateway, multicast, loopback, link-local, unspecified, and equality tests.
Document any deliberate point-to-point exception.

### P1.4 Bind provenance to the exact tools that execute

`crates/soma-generation/src/generation/process.rs` hashes a binary by path and later executes that path.
The path can refer to different bytes after verification.
Several secondary filesystem tools are recorded by name or version rather than exact executable identity.

Execute a previously opened verified descriptor or use a sealed pinned builder environment whose complete artifact identity is bound into evidence.
Bind every material formatter, verifier, inspector, and helper tool.
Set and verify the required builder-image digest instead of retaining `None`.

### P1.5 Use structured workload commands

The Template revision currently represents a workload probe as opaque command-line bytes.
This leaves shell selection, quoting, argument splitting, and executable resolution ambiguous.

Represent the probe as one exact absolute executable plus an ordered bounded argument list, user, working directory, timeout, and output allowance.
No implicit shell parsing is permitted.

## Priority 2 integration gates

The following work is incomplete rather than defective.
Do not describe it as implemented until these gates pass.

### P2.1 Complete Host launch-page integration

The guest currently maps, consumes, wipes, and rereads the launch page locally.
The Host still must:

- Create a dedicated nonsnapshot KVM memory slot.
- Write fresh launch material only after restore.
- Resume the guest in the required order.
- Observe complete zeroing from the Host.
- Retire the memory slot.
- Prevent access after retirement.
- Destroy the Instance if any step is ambiguous.

### P2.2 Run the static guest agent inside the real KVM machine

Unit tests do not prove the PID 1 executable reaches Ready.
Integrate the required virtio block, vsock, entropy, and optional network devices, then boot the real Generation.

Retain evidence for early mounts, writable overlay, launch-page repair, entropy repair, identity repair, authenticated control, fixed readiness probe, direct execution, shutdown, and cleanup.

### P2.3 Complete Generation compiler ticket #6

The current compiler does not yet prove:

- Reproducible Node 22 construction.
- Real guest EROFS lower mount.
- Real private ext4 upper mount and write behavior.
- Cross-Instance writable-state isolation.
- Builder isolation.
- Boot and capture.
- Snapshot compatibility.
- Certification.
- Complete failure cleanup.
- Raw retained end-to-end evidence.

### P2.4 Keep Template claims narrow

The current `TemplateRevision` is an initial immutable input binding.
It is not the complete Template system from ADR 0022 and `docs/research/template-implementation-map.md`.

Do not claim module resolution, conflict detection, policy authorization, canonical Template Lock, SBOM, signing, revocation, registry lifecycle, agent modules, or Launch-input delivery until those slices exist and pass their gates.

### P2.5 Do not make performance claims yet

The retained kernel-boot sample is useful diagnostic evidence.
It is not Launch-to-Ready, first-command, 100-way burst, cleanup, or ComputeSDK evidence.

Performance claims require the exact boundaries, raw samples, preparation class, HostProfile, Generation, command result, failures, and cleanup evidence defined by `docs/benchmark-contract.md`.

## Required work order

```text
authentication provisioning
        |
        v
bounded guest execution
        |
        v
bounded Generation tools
        |
        v
Candidate versus ready publication
        |
        v
complete manifest validation
        |
        v
entropy, ABI, network, and provenance corrections
        |
        v
Host launch-page integration
        |
        v
real virtio device integration
        |
        v
real guest-agent boot and Ready
        |
        v
Generation certification
        |
        v
complete sandbox lifecycle
        |
        v
performance evidence
```

Do not reorder this sequence merely because a later feature is easier to demonstrate.
The lower security and ownership seams determine whether later evidence is meaningful.

## Validation performed during this audit

The complete portable profile passed on macOS:

- Repository version contract.
- Architecture and source-size rules.
- Workflow policy.
- Release artifact and packager regression tests.
- Shell checks.
- Rust formatting.
- Benchmark harness tests and syntax checks.
- Clippy with warnings denied.
- Workspace unit and integration tests.
- Documentation tests.

Linux-only and ignored live tests did not run on macOS.
Passing portable tests do not prove Linux x86_64 KVM integration, real guest boot, real devices, complete cleanup, isolation, or latency.

## Review scope limitations

This was not a complete audit of every historical repository line.
It did not include:

- A full manual review of every `unsafe` block.
- A complete cryptographic proof or independent cryptography review.
- Dependency advisory, license, or supply-chain tool results beyond the portable profile.
- Linux KVM penetration testing.
- Real device fuzzing through a running hostile guest.
- Performance validation.
- Cloud or fleet deployment validation.

Those reviews remain required before production claims.

## Relevant specifications

- `MISSION.md`
- `ROADMAP.md`
- `docs/research/vmm-decision-map.md`
- `docs/research/generation-compiler.md`
- `docs/research/linux-guest-agent-integration.md`
- `docs/research/template-implementation-map.md`
- `docs/adr/0017-authenticated-guest-session.md`
- `docs/adr/0020-launch-page-and-application-wire-contracts.md`
- `docs/adr/0021-own-authenticated-control-lifecycle.md`
- `docs/adr/0022-compose-templates-into-generation-locks.md`
- `docs/adr/0023-launch-page-network-identity.md`
- `docs/benchmark-contract.md`
- `docs/threat-model.md`

## Completion signal for the next review

Request another review only after every Priority 0 finding has a regression test and a focused fix commit.
The next review should compare those commits directly against this document and rerun the portable profile plus the applicable Linux x86_64 KVM tests.
