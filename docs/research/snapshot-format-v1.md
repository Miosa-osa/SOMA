# SOMA snapshot format v1

## Decision

One Generation snapshot is an immutable memory object plus a canonical state manifest and separately managed disk artifacts.
Launch maps memory with `MAP_PRIVATE | MAP_NORESERVE`, registers it directly with KVM, and never copies the full image.
Version 1 has no delta chain, live migration, post-copy, or userfaultfd dependency.

## Objects and identity

`memory.raw` is page-aligned, exactly the certified guest-memory size, sparse only when the artifact store preserves holes, and SHA-256 addressed.
`overlay.raw` is the quiesced private overlay head of the machine that was captured, and it is the sterile template every restore of that snapshot clones, so the guest's page cache and the disk it describes stay consistent.
`state.somasnap` starts with `SOMASNP\0`, schema version, architecture, page size, GenerationId, machine and device contract digests, CPU-template digest, host-profile requirements, memory descriptor, and bounded typed sections.
Sections contain VM state, one vCPU state, irqchip and routing state, KVM clock state, five device states, queue states, and the disconnected repair-point marker.
Every section has a role, version, length, digest, and critical flag.
Unknown critical roles, duplicates, trailing bytes, unsupported versions, or absent required state reject restore.

Host paths, file-descriptor numbers, TAP names, namespaces, sockets, random bytes, launch secrets, Noise keys, live connections, pending packets, and backend buffers never enter a snapshot.
Disk descriptors bind the immutable EROFS digest and sterile-overlay contract, while each Launch supplies a fresh private overlay head.

## Capture

The builder boots the Generation's pinned guest agent, reaches the disconnected repair point before any launch page exists, disables ingress, drains device work, flushes the overlay, pauses the vCPU, and proves every guest-driven queue quiescent.
Receive and event queues hold buffers the driver posted in advance; those are ordinary capacity and are restored with the queue.
Capturing before any launch material exists is what makes the image free of Instance authority by construction rather than by scrubbing, and the capture point is recorded in the manifest (ADR 0024).
It reads KVM and device state in a fixed order while the vCPU remains joined outside `KVM_RUN`.
It writes memory and state to private staging objects, independently decodes them, hashes through retained handles, and publishes the Generation manifest last.
Capture failure destroys the disposable builder and publishes nothing complete.

## Restore

Restore validates constant-size compatibility metadata before mapping large artifacts.
It maps memory privately, registers slots, recreates irqchip and routes, constructs devices with fresh backends, creates the vCPU, restores CPUID and MSRs, then register, LAPIC, event, clock, queue, and interrupt state.
The fresh launch page is a separate anonymous memory slot excluded from the snapshot and written before resume.
Eventfds and irqfds exist before pending interrupt state is armed.
The vCPU resumes only after all state constructors succeed, then authenticated repair and the fixed command gate Ready.

## Compatibility and integrity

Restore requires exact architecture, page size, memory layout, vCPU count, CPU template, KVM API, required capabilities, machine contract, device contract, queue limits, feature negotiation, guest protocol, and snapshot schema.
The installed Generation is authenticated before Launch, while request-time checks use immutable inode and constant-size manifest identity rather than hashing guest RAM.
Any mutable or untrusted artifact store requires signature or MAC verification at installation and an immutable local publication boundary.

## Modules and gates

`snapshot/manifest`, `snapshot/memory`, `snapshot/kvm_state`, `snapshot/device_state`, `snapshot/capture`, `snapshot/restore`, and `snapshot/compatibility` remain separate modules, and the live half that turns them into KVM calls lives separately again under `x86_64/snapshot`.
Tests must cover golden bytes, every truncation and corruption boundary, unknown fields, arithmetic overflow, private-memory divergence across clones, restore ordering, pending interrupts, fresh authority, timeout cleanup, wrong CPU and host rejection, and raw p50 and p99 mapping and restore samples.
Firecracker's documented `MAP_PRIVATE` restore validates the mechanism, but SOMA adds cryptographic artifact identity and excludes host resource paths from persisted state.

The codec and the live capture and restore both exist on Linux `x86_64`, and the retained result is [the x86_64 snapshot restore evidence](../evidence/2026-08-29-x86_64-snapshot-restore.md).
Re-hashing the memory object is the installation and audit boundary rather than a request-time check, exactly as the compatibility section requires, so a warm restore validates constant-size identity only and a tampered memory object is caught when the Generation is installed.
