# ADR 0005: macOS VM development backend

- Status: Accepted
- Date: 2026-08-28
- Amended by: ADR 0013

## Context

SOMA's production substrate is Ubuntu 24.04 x86_64 KVM.
The primary development computer is Apple Silicon macOS, where `/dev/kvm` is unavailable and a Linux cross-build cannot prove a sandbox lifecycle.
Developers still need to exercise OCI image selection, VM isolation, resource shaping, execution, inspection, shutdown, cleanup, CLI rendering, and error behavior before a production host is available.

Docker Desktop does not provide one independently hardware-isolated virtual machine for every requested sandbox.
A deterministic adapter proves domain ordering but does not prove that an OCI workload ran inside a real VM.
Running the production x86_64 KVM backend directly on macOS is impossible.

Apple's `container` runtime uses Virtualization.framework and creates one lightweight Linux virtual machine per OCI container.
Its command interface provides image acquisition, CPU and memory limits, create, start, exec, stop, delete, inspect, and one-shot run operations.

## Decision

SOMA includes a development-only `soma-macos` adapter around the verified Apple `container` 1.3 command family.
The adapter invokes an explicitly selected executable directly through `std::process::Command` and never through a shell.
It owns bounded process execution, bounded output capture, timeouts, typed failures, deterministic SOMA-owned names and labels, and cleanup proof.

The repository provides an unprivileged bootstrap script that downloads the exact signed package, verifies its published SHA-256 digest and Apple package signature, extracts it into a versioned user-owned directory, starts it with explicit application and log roots, and verifies `running` status.
The bootstrap does not install a privileged system package or add an executable to a global path.

The product CLI may select this backend on Apple Silicon for local development.
The same CLI must label its backend and development status in machine-readable results.
The adapter must fail closed on unsupported hosts, unverified command versions, a stopped service, malformed output, process failure, timeout, output overflow, or cleanup uncertainty.

## Evidence boundary

A passing macOS lifecycle proves that the selected OCI userland executed inside a Virtualization.framework Linux VM with the requested local resource shape.
It can validate CLI semantics, OCI compatibility cases, guest command behavior, and local cleanup.

It does not prove Linux KVM ioctls, x86_64 CPU policy, snapshot restoration, private memory mappings, reflink behavior, TAP networking, namespaces, cgroups, seccomp, production jail behavior, production host density, or production latency.
It cannot satisfy a stable-release KVM or benchmark gate.

## Alternatives considered

### Docker Desktop containers

This option was rejected as lifecycle evidence because multiple requested containers can share one Docker Desktop Linux VM and guest kernel.
It remains useful for ordinary builds that do not claim per-sandbox VM isolation.

### A native Virtualization.framework VMM in this repository

This option would provide deeper control and may be added behind the same contract later.
It was deferred because the current need is a truthful local conformance backend, while SOMA's latency-critical implementation remains the Linux KVM backend.

### Nested KVM on Apple Silicon

The default Apple guest kernel did not expose `/dev/kvm` when virtualization was enabled.
ADR 0013 accepts an explicitly built KVM-enabled ARM64 kernel and Apple Container nested virtualization as a development-only KVM probe substrate.
That nested path does not certify the production x86_64 host contract or direct-host latency.

## Consequences

Developers can run real OCI workloads in per-sandbox VMs locally without confusing that result with production proof.
The CLI and adapter must keep backend identity explicit in structured evidence.
The Linux KVM implementation remains independently testable and cannot depend on Apple runtime types or behavior.
