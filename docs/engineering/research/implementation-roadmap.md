# SOMA implementation roadmap

## Purpose

This is the coding-agent execution order for the architecture resolved in decision-map tickets #1 through #15.
An agent must not claim a later gate because an earlier module compiles.
Each slice ends with runnable Linux evidence and keeps the public lifecycle stable.

## Dependency order

1. Implement hostile-input-safe modern virtio transport and split queues with unit, property, fuzz, and Kani tests.
2. Implement the immutable and private block devices and boot the pinned x86_64 PVH kernel from deterministic EROFS plus ext4 OverlayFS.
3. Integrate the static guest agent, fresh launch page, entropy and identity repair, vsock Noise control, and one authenticated command.
4. Implement canonical snapshot capture and `MAP_PRIVATE` restore with exact compatibility rejection and no live authority.
5. Implement the VMM jail from the measured syscall and ioctl inventory and prove crash containment.
6. Implement `soma-netd`, sterile network bundles, protected routing, repair-gated activation, and reconciliation.
7. Implement XFS reflink overlay classes and select on-demand or prepared heads from p99 evidence.
8. Implement `soma-hostd` prepared workers, single-winner transfer, bounded pools, backpressure, and recovery.
9. Wire the complete KVM lifecycle into the portable backend with truthful receipts and idempotent operations.
10. Run production admission gates, publish no speed claim until raw retained evidence passes, then scale through multi-host cells.

## Agent handoff contract

Each implementation agent receives the owning research document, linked ADRs, relevant module-map section, exact predecessor commit, and one vertical acceptance test.
It must update NOTES.md with non-obvious decisions, preserve unrelated work, avoid weakening portable contracts, and leave unsupported behavior fail closed.
It must distinguish tests runnable on macOS from required Ubuntu 24.04 x86_64 KVM evidence.

Every handoff reports changed repositories and files, commands run, passing and skipped tests, retained evidence paths, unproven claims, performance samples, and the next dependency.
No skipped Linux test, synthetic backend, ARM64 development proof, Docker result, or compiler-only check satisfies a KVM acceptance gate.

## Definition of complete

SOMA is a working v1 sandbox only when an OCI-derived Node 22 Generation restores on a certified Ubuntu 24.04 x86_64 KVM host, creates fresh isolated identity, authenticates the expected guest agent, executes a bounded Node command, enforces network and storage policy, stops, and proves complete cleanup.
It is fast only when the exact measured boundary passes the published p50 and p99 goals with raw successful 100-way burst evidence.
It is production-admitted only when the signed HostProfile report passes every gate in the production evidence architecture.
