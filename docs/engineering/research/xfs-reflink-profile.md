# SOMA XFS reflink storage profile v1

## Decision

The immutable EROFS root is shared read-only.
Writable state uses one private ext4 overlay head created with `FICLONE` from a sterile size-class template on XFS with `reflink=1`.
Launch never formats, grows, scans, or copies a filesystem.

## Size classes and ownership

Operators publish versioned overlay classes with logical bytes, ext4 UUID policy, block size, features, inode policy, mount options, template digest, and minimum free-space evidence.
Admission resolves requested storage to one exact class or rejects it.
The allocator may precreate sterile reflink heads outside Launch, but an assigned head is single-use and destroyed after the Instance.

Creation opens the verified template read-only and a create-exclusive destination under a capability-owned directory, performs `FICLONE`, syncs directory publication, verifies apparent size and extent sharing, then transfers the open destination descriptor.
The VMM receives no storage-directory path.

## Correctness and performance

Conformance writes different patterns through two clones, forces allocation, flushes, remounts, and proves the template and peer remain unchanged.
It tests ENOSPC, quota exhaustion, reflink-disabled filesystems, unsupported mount options, fragmentation, crash during clone and deletion, and concurrent create and cleanup.

The benchmark matrix crosses image sizes, allocated extent counts, size classes, cache states, concurrency 1, 10, and 100, free-space pressure, and cleanup pressure.
It retains every raw duration plus kernel, XFS, mount, device, CPU, and template identity.
On-demand cloning is admitted only if p99 fits the disk budget under the worst certified matrix; otherwise prepared heads are mandatory.

Modules are `storage/profile`, `storage/template`, `storage/clone`, `storage/verify`, `storage/lease`, `storage/release`, and `storage/reconcile`.

## Measured result

The matrix ran on 2026-08-29 through `crates/soma-storage` and `scripts/xfs-reflink-bench.sh` on a loop-backed XFS `reflink=1` filesystem inside a privileged pinned Ubuntu 24.04 container.
No 100-way cell came near the 1.00 ms p99 disk share of fresh resource activation, so on-demand cloning is not admitted and prepared sterile heads are mandatory.
The retained tables, identities, and decision are in [the XFS reflink evidence](../evidence/2026-08-29-xfs-reflink-profile.md).
