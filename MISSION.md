# SOMA mission

SOMA is the Secure Optimized Machine Architecture.
It is an open-source virtual machine monitor and launch runtime for fast, hardware-isolated Linux execution.

## North star

SOMA aims to become the state-of-the-art hardware-isolated sandbox engine across clouds, bare-metal operators, workload images, machine shapes, and storage sizes.
Portability is earned through explicit capability contracts and conformance evidence rather than a claim that every host behaves alike.
The initial Ubuntu 24.04 x86_64 KVM target is the first rigorously supported substrate, not a permanent product boundary.
The client library and command-line interface target Linux, macOS, and Windows, while local isolation engines and remote hosts earn support independently.

## Mission

Turn a certified immutable machine state into a fresh, isolated, command-ready virtual machine with the smallest trustworthy latency and the smallest caller-facing interface we can sustain.

## Product outcome

An infrastructure operator should be able to prepare an OCI-derived workload once, restore it many times concurrently, and receive a trustworthy readiness result without learning KVM, snapshot, device, jail, or guest-identity internals.
The operator should select validated CPU, memory, and storage shapes without depending on one cloud's instance vocabulary or one vendor's control plane.

## Success criteria

- One native VMM process owns one virtual machine.
- The initial production target is Ubuntu 24.04 x86_64 with KVM.
- The portable client and command-line contract build on Linux, macOS, and Windows without importing one host's virtualization mechanism.
- Unsupported local hosts fail closed and can use an explicitly configured remote SOMA engine without changing use-case semantics.
- Public contracts contain no cloud, provider, billing tier, or fixed machine-size assumption.
- Generation compatibility states the supported CPU, memory, device, and storage shape constraints explicitly.
- Restored memory uses immutable shared backing with private copy-on-write mappings instead of eager full-memory copies.
- Every restore has a fresh globally unique Instance identity, entropy state, network identity, and time repair.
- Readiness means an authenticated guest agent completed a real command after repair.
- Snapshot compatibility is explicit, versioned, content-addressed, and fail closed.
- A 100-way burst preserves isolation, readiness, cleanup, and identity invariants.
- Public performance claims include raw samples and the complete measurement boundary.
- Every terminal use case returns a versioned execution receipt that identifies the immutable workload, effective isolation class, preparation class, result, timing boundary, and cleanup state.

## Initial performance targets

These are engineering targets, not current claims.

- Prepared worker acquisition and dispatch p50 below 0.10 ms and p99 below 0.50 ms.
- Private mapping, KVM restoration, and guest-control wake p50 below 1.15 ms and p99 below 3.30 ms on a certified warm host.
- Combined authenticated guest repair and Ready p50 below 1.50 ms and p99 below 3.50 ms after guest-control wake.
- The additive server-side create budget totals 3.25 ms p50 and 8.90 ms p99.
- Complete server-side create p50 below 5 ms and p99 below 10 ms.
- First bounded command completion p50 below 10 ms and p99 below 20 ms from accepted Launch.
- Exact ComputeSDK Burst TTI median below 50 ms and p99 below 90 ms for 100 concurrent `node:22` sandboxes.
- 100 percent successful creation, first command, and cleanup in the measured cohort.

## Non-goals for the first production generation

- General-purpose PC emulation.
- BIOS or UEFI boot.
- PCI topology, USB, graphics, or device hotplug.
- Arbitrary guest kernels supplied at launch time.
- Nested containers inside the guest as the default execution model.
- macOS production virtualization.
- Arbitrary non-Linux guest operating systems in the first stable release.
- Hiding cold OCI acquisition and image construction inside a warm-start measurement.

## Current status

SOMA is pre-alpha research and implementation work.
It is not yet safe for untrusted production workloads.
