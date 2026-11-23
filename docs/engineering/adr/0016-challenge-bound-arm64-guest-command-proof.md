# ADR 0016: Admit a challenge-bound ARM64 guest-command proof

- Status: Accepted
- Date: 2026-08-28
- Extends: ADR 0014

## Context

ADR 0014 proves that an explicit ARM64 Linux fixture can cold-boot and emit diagnostic console bytes.
Console bytes cannot carry a reusable command contract because kernel diagnostics and command data would share one unauthenticated stream.
The next tracer bullet must execute one real guest process while remaining test-only and narrower than authenticated Ready, a SOMA Machine lifecycle, or a production control channel.

## Decision

`soma-kvm` retains one crate-internal `execute_arm64_fixture(fixtures, command) -> outcome` style entry under `cfg(test)`.
It hides fixture loading, framing, the guest agent, UART emulation, KVM execution, timeout containment, and cleanup.
The command names one absolute guest program and an argument vector without a shell.
Before opening KVM or creating a VM, the host rejects empty or relative programs, embedded NUL bytes, zero or excessive deadlines, zero or excessive output limits, excessive argument counts, and excessive encoded request sizes.

Each execution generates a fresh 256-bit launch challenge from the host operating system.
The host sends one versioned binary request only after the guest agent announces that its control transport is configured.
Every terminal response repeats the exact challenge and is rejected if its magic, version, kind, lengths, reserved fields, or challenge differ.
This binds a response to the current diagnostic launch and rejects stale frames.
It is not authentication because the guest kernel and fixture can observe the challenge and forge a response.
It does not satisfy ADR 0003 Ready.

The control protocol uses a second dedicated 16550 MMIO UART, separate from the diagnostic console.
Linux reaches it as a normal raw serial device.
The ARM64 kernel fixture must configure both `CONFIG_SERIAL_8250_NR_UARTS` and `CONFIG_SERIAL_8250_RUNTIME_UARTS` to at least 2 so Linux can register the dedicated control device as `/dev/ttyS1`.
PID1 opens it with `O_NOCTTY | O_CLOEXEC`, configures fully raw 8-bit mode before Hello, and removes the device node before starting the workload.
The VMM uses rust-vmm's tested `vm-superio` 16550 model and streams requests through its 64-byte receive FIFO.
On the sole vCPU and device worker, its interrupt trigger synchronously pulses the edge-rising control-UART SPI with `KVM_IRQ_LINE`.
The command-only host gate requires KVM irqchip support before VM creation, and the encoded ARM interrupt uses the full GIC SPI ID rather than the device-tree-relative index.
A no-interrupt tty would deadlock, while guest `/dev/mem` polling would create a fixture-specific transport that cannot be reused.
No virtio, vsock, block, network, or general device bus is admitted by this decision.

The version 1 frame has a fixed 64-byte big-endian header containing magic, version, header length, kind, zero flags and reserved bytes, request ID, sequence, payload length, challenge, and CRC32C.
The request payload contains the timeout, output limit, absolute program, and length-prefixed arguments.
The guest streams nonempty stdout and stderr chunks of at most 4096 bytes with strict sequence numbers and then one terminal frame carrying exact stream byte counts.
Every post-handshake frame must match the launch request ID and challenge.
All arithmetic and conversions are checked before allocation or slicing.

The guest fixture contains a small PID1 agent and a separate static probe executable.
The agent invokes the requested absolute program directly with `execve`, captures stdout and stderr independently, and never invokes a shell.
One allowance bounds stdout and stderr in aggregate.
The aggregate allowance is assigned in the order the agent drains ready descriptors, with stdout drained before stderr when both are ready.
The outcome preserves exact retained bytes per stream and does not claim a cross-stream byte ordering.
The host separately caps response frame count and total control-wire bytes for the worst legal one-byte chunking.
The agent kills and reaps the command process group on the first byte beyond the aggregate allowance or the requested child deadline and returns only the retained prefixes.
The guest reports exactly one terminal outcome: exited with a status, terminated by a signal, timed out at the requested child deadline, exceeded the output limit, failed `execve`, or suffered a typed agent failure.
An outer boot or terminal-containment watchdog expiry is an error and never fabricates a command outcome.

The existing watchdog remains the containment boundary.
One worker thread is the sole owner of the vCPU descriptor, `kvm_run` mapping, VM interrupt handle, and GIC device until the vCPU stops.
Registered guest memory stays parent-side until that worker has joined.
On outer timeout the host interrupts `KVM_RUN`, joins the worker before registered memory can be released, and aborts the dedicated test process if containment fails.

## Verification

Default tests validate command admission, exact frame encoding and decoding, challenge mismatch rejection, UART state, bounded output parsing, and outcome distinctions without KVM.
Ignored live tests run as exact selections in dedicated Apple nested-KVM test processes.
They must prove exact success output, a nonzero exit, signal termination, timeout containment, output-limit containment, and repeated descriptor and task-count cleanup where practical.
Fixture generation must be deterministic and must retain hashes with any live evidence.

## Consequences

SOMA gains one real guest-process tracer bullet behind a small internal interface.
The proof remains a cold-boot diagnostic and makes no sandbox, OCI, restore, authenticated-readiness, isolation, cleanup-certification, or latency claim.
Production work must separately define an authenticated transport, Generation identity, repair, reusable guest-agent lifecycle, and the Ubuntu 24.04 x86_64 implementation.
