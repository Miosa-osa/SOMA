# ADR 0013: Use nested ARM64 KVM as a development probe substrate

- Status: Accepted
- Date: 2026-08-28
- Amends: ADR 0005

## Context

SOMA's first production engine remains Ubuntu 24.04 x86_64 on certified KVM hosts.
The primary development computer is Apple Silicon macOS, which does not expose `/dev/kvm` directly.
Apple Container 1.3 can give an ARM64 Linux guest nested virtualization when the host is compatible and the guest uses a KVM-enabled kernel.
The default Apple guest kernel is not sufficient for this path.
Docker Desktop 28.5.1 on the tested host did not expose `/dev/kvm`, including when Docker was asked to pass that device explicitly.

Developers need a real KVM development loop before production x86_64 hosts are involved.
That loop must not turn an ARM64 nested result into a claim about the production architecture, direct-host latency, or a working sandbox lifecycle.

## Decision

The `soma-kvm` capability-probe interface is compiled for Linux x86_64 and Linux ARM64.
`SUPPORTED_TARGET` means that the build can attempt the KVM capability probe.
It does not mean that a complete local sandbox engine or certified host profile exists.

The probe keeps architecture-specific capability requirements behind its existing interface.
The x86_64 requirement set retains the in-kernel irqchip requirement.
The ARM64 probe does not reuse that x86-specific assumption.
Interrupt-controller, timer, boot, snapshot, device, and restore contracts will be admitted separately when each architecture implements them.

Apple Container nested virtualization is an experimental development substrate for the ARM64 KVM implementation.
It is not a SOMA runtime dependency, a production backend, or part of the measured production fast path.
The outer Apple VM must use an explicitly selected runtime and a KVM-enabled kernel whose source revision and configuration are recorded.

Every retained nested result must identify the inner architecture, guest kernel, outer virtualization layer, exact SOMA revision, command, success or failure, and cleanup outcome.
Nested results must never be reported as direct-host x86_64 evidence.

## Alternatives considered

### Use Docker Desktop as the nested KVM host

This option was rejected for the tested host because the Docker daemon VM did not contain `/dev/kvm`.
Docker and Docker Hub remain valid OCI image and ordinary build inputs.

### Keep all real KVM work blocked on x86_64 hosts

This option was rejected because it would delay architecture-neutral KVM ownership, error handling, and descriptor-lifecycle feedback that can be exercised locally.
Production acceptance still requires the exact x86_64 host profile.

### Treat ARM64 probe success as ARM64 engine support

This option was rejected because an empty-VM probe does not boot a guest, enforce isolation, restore a snapshot, execute a command, or prove cleanup of a complete Machine.

## Verification

Both Linux targets must compile with warnings denied.
The live ARM64 gate must run through the public `probe()` interface with a real readable and writable `/dev/kvm`.
It must verify KVM interface version 12, the architecture-specific capability contract, a nonzero vCPU mapping size, successful empty-VM creation, and descriptor cleanup.
The ignored test counts as evidence only when the retained command shows that it was explicitly executed on the real nested target.
The first retained development result is recorded in [`docs/evidence/2026-08-28-arm64-kvm-probe.md`](../evidence/2026-08-28-arm64-kvm-probe.md).

The first stable release still requires complete Ubuntu 24.04 x86_64 guest boot, authenticated readiness, execution, snapshot restore, isolation, cleanup, and performance evidence.

## Consequences

Developers can exercise real ARM64 KVM ioctls from the Apple development machine while keeping the production contract unchanged.
Architecture-specific implementation can evolve behind one small probe interface.
ARM64 nested timing is useful for stage diagnosis and regression work but cannot certify production latency.
