# ADR 0014: Admit an explicit-fixture ARM64 KVM cold-boot proof

- Status: Accepted
- Date: 2026-08-28
- Extends: ADR 0013

## Context

ADR 0013 admitted nested ARM64 KVM only as a host capability and empty-VM development probe.
The next useful boundary is a real Linux direct boot that exercises guest memory registration, vCPU state, GICv3, the architectural timer, a device tree, an initramfs, and serial output together.
That proof must remain narrower than a SOMA Machine or sandbox lifecycle.

## Decision

`soma-kvm` retains a crate-internal Linux ARM64 cold-boot function that accepts explicit kernel Image and initramfs paths only for its ignored live tests.
The function creates exactly one vCPU and 128 MiB of anonymous guest RAM, direct-boots Linux, and returns after observing the exact serial sentinel exported by the crate.
The caller is responsible for trusting and identifying both fixture files.
The function does not discover, download, rebuild, or attest either fixture.
Execution has a fixed deadline.
The vCPU thread is the sole owner of its `VcpuFd` and `kvm_run` mapping.
It blocks the reserved kick signal in its ordinary pthread mask, installs an eight-byte `KVM_SET_SIGNAL_MASK` override with that signal removed, and reports readiness only after both masks are installed.
This makes a kick sent just before `KVM_RUN` remain pending until KVM atomically applies its temporary mask, while a kick sent during `KVM_RUN` interrupts it directly.
At the deadline the host targets that thread with the reserved signal, performs a bounded join, and only then releases the VM and registered memory.
If signal-mask setup, targeted cancellation, or joining cannot be contained within its bounded grace period, the process aborts instead of returning while a vCPU may still access registered memory.
The function is deliberately not exported by the library because its process-wide signal reservation and abort containment are unsafe integration semantics for an embedding application.
Each ignored live test must run alone as an exact test selection in its own test process.
It exclusively reserves `SIGRTMIN + 7` while running, temporarily installs its kick handler, and restores the previous process-wide signal action after the vCPU joins.
The worker restores its original pthread signal mask before it exits.

Before creating the VM, the proof requires KVM API version 12, a nonzero vCPU mapping size, and exactly the capabilities exercised by this path: user memory, one-register access, device control for GICv3, and ARM PSCI 0.2.
IRQFD, IOEVENTFD, and `immediate_exit` are not part of this cold-boot path.

The proof implements only the platform pieces required by that boundary.
It uses KVM GICv3 and the architectural timer, a checked fixed memory layout, a generated device tree, and transmit-only 16550 MMIO emulation.
The emulated line-status register always reports an empty transmitter, so Linux can emit its polled console bytes without an interrupt-injection subsystem.

Successful return proves only that the expected bytes crossed the emulated console boundary before the deadline.
The internal proof does not authenticate which guest component emitted those bytes, and an untrusted kernel can spoof the sentinel.
PID1 attribution is valid only for retained evidence whose kernel and initramfs hashes match the reviewed fixture inputs.
It does not prove an authenticated control channel, command readiness, OCI execution, network isolation, snapshot restore, production cleanup, or any latency objective.

## Verification

The default suite validates fixture sizing and memory layout, register IDs, UART behavior, signal-mask encoding, and cross-compilation without requiring KVM.
An ignored live test must be explicitly given existing absolute fixture paths.
That test must open the real `/dev/kvm`, boot the generated PID1 fixture, observe the exact sentinel through the crate-internal function, and verify the process file-descriptor count is unchanged after return.
An ignored timeout regression must search for an unreachable sentinel, interrupt the running vCPU at a short test deadline, return a timeout error, and verify descriptor cleanup.
Nested ARM64 elapsed time remains diagnostic evidence only under ADR 0013.

## Consequences

SOMA now has an executable ARM64 kernel-boot tracer bullet owned by `soma-kvm`.
The test-only fixture boundary makes its trust assumptions visible and keeps image construction outside the VMM.
Later work must separately admit shutdown, UART interrupts, authenticated readiness, devices, snapshots, and the production x86_64 path instead of silently expanding this proof.
