# ADR 0019: Deterministic normalized root filesystem

- Status: Accepted
- Date: 2026-08-29

## Context

ADR 0018 produces a verified `ImportedOci`, but it deliberately stops before applying OCI filesystem changes.
Generation construction needs a deterministic final filesystem tree before it can choose and certify a guest disk format.
OCI layers are ordered changesets rather than directories that can be unpacked independently.
Whiteouts, opaque directories, type replacement, hard links, ownership, modes, and metadata all affect the final tree identity.
Extracting an untrusted layer into a host directory would also expose construction to host path traversal, symlink following, privilege, filesystem, and metadata differences.

The current KVM proof directly loads an explicit kernel and initramfs and has no root block device.
Calling a normalized tree bootable, a Generation, or KVM-ready would therefore exceed the evidence.
Encoding the workload as another initramfs would be a throwaway path that loses the intended immutable-root and private-copy-on-write disk topology.

## Decision

`soma-generation` exposes one additional deep entry point, `normalize_oci_rootfs`.
It accepts an `ImportedOci`, the same explicit content-store root, and explicit `RootfsLimits`.
It returns `NormalizedRootfs` and never creates or returns a `GenerationId`.

The normalizer reopens the import completion artifact from the store, verifies its exact size and SHA-256 digest, decodes the schema-owned import manifest, and requires its workload to equal the supplied `ImportedOci`.
It reopens every selected compressed layer by verified descriptor, verifies the stored bytes through the same open handle, expands the layer under the configured bounds, and requires the recorded expanded digest, size, and entry count to match.

The implementation never extracts into a host filesystem namespace.
It parses raw byte paths into a bounded logical tree and never converts guest paths to host `Path` values.
Absolute paths, NUL bytes, parent traversal, empty non-root paths, and paths longer than the configured limit are rejected.
Symlink targets are retained as bounded opaque bytes and are never followed during construction.
Missing parent directories are created with one schema-owned default metadata value, while a non-directory parent is an invalid layer conflict.
Every retained implicit or explicit path and link is charged before insertion, so prefix expansion cannot temporarily exceed the entry or metadata bounds.

Each complete layer is first converted into a bounded plan.
All valid whiteouts in that plan are applied to the lower tree before any additions from the same layer, independent of archive entry order.
An ordinary `.wh.<name>` removes that lower path and its descendants.
`.wh..wh..opq` removes all lower descendants of its directory while retaining the directory itself.
Whiteouts must be empty regular entries, and every malformed reserved `.wh.` name is rejected.
Reserved `.wh.*` names are rejected in every non-marker path component and can never enter the final tree.

A directory replacing a directory changes that directory's metadata and retains its children.
Every other type replacement removes the old path and its descendants before creating the replacement.
Same-layer additions with a non-directory ancestor conflict are rejected rather than acquiring archive-order semantics.

Regular files, directories, symbolic links, hard links, and FIFOs are the complete accepted node set for version 1.
Character devices, block devices, sockets, sparse files, contiguous files, and unknown tar entry types are rejected.
Numeric user and group identifiers, the low permission and set-id mode bits, and integral-second modification time are preserved.
User and group names do not participate in identity.
Mode bits outside the supported mask are rejected.

Hard links share one logical file inode.
The target must resolve to a regular-file inode in the lower tree or the same complete layer plan.
Missing targets, directory targets, and hard-link cycles are rejected.
Canonical encoding identifies a hard-link group by its lexicographically smallest final path, so internal allocation order never affects identity.

Version 1 accepts only local PAX `path` and `linkpath` records and accepts no filesystem xattrs.
Each accepted value must be valid UTF-8, is retained byte-for-byte as the effective path or link value, and remains subject to the normal path, link, and node-kind rules.
Malformed records, duplicate keys, global PAX, and every other local key are rejected rather than silently losing metadata.
Local PAX naming records cannot be combined with GNU long-name or long-link extensions for the same member because the formats do not establish one canonical cross-format precedence.
The rejected keys include timestamps, `SCHILY.xattr.*`, ACL, capability, integrity, security, and unknown vendor metadata, so fractional time and arbitrary PAX support remain outside version 1.
Before the normal tar parser runs, a raw streaming preflight caps each local PAX body at 64 KiB, bounds each GNU long-name and long-link body, charges all such extension bodies across selected layers against the caller's aggregate metadata budget, and rejects global PAX and GNU sparse entries from their headers.
All raw tar headers across selected layers are independently capped by `max_entries`, including ordinary entries and extension records with empty bodies, so parser work has an explicit checked bound.

Regular-file bodies stream directly from the verified layer entry into immutable content-addressed objects.
The construction never retains a whole file or expanded layer in memory.
Content objects can be orphaned by a later failure without corrupting identity or atomic-last completion, but repeated failures can consume store capacity until garbage collection.
The private pre-alpha builder therefore requires an operator-enforced job or store quota and out-of-band garbage collection before it accepts tenant-triggered normalization.
Publication inherits ADR 0018's private-store writer authority and platform-specific durability contract.

The completion artifact is a schema-owned canonical binary tree manifest.
It starts with a fixed magic, format version, policy version, and entry count.
Entries are sorted lexicographically by normalized raw path bytes.
Fixed-width big-endian fields encode node kind, numeric ownership, supported mode bits, integral modification time, and a zero xattr count.
Regular-file anchors encode exact size and SHA-256 content digest.
Additional hard-link paths encode their canonical anchor path.
Symlinks encode their raw target bytes, and directories and FIFOs have no payload.

The rootfs manifest digest is the normalized tree identity.
It excludes OCI layer partitioning, compression, tar entry order, selected manifest identity, traversal indexes, and import provenance.
Two valid OCI histories with the same final filesystem semantics therefore produce the same rootfs identity.
`NormalizedRootfs` separately retains the source import-manifest digest as provenance.

All path, entry, metadata, file, aggregate content, and manifest lengths use checked arithmetic and explicit request bounds.
Errors expose only a stable phase and classification.
Request and result debug formatting never exposes host paths, guest paths, file contents, symlink targets, or xattr values.

## Safety boundary

The content store retains ADR 0018's trusted stable ancestor and exclusive writer requirements.
The normalizer protects its host process from guest path traversal and symlink following because it never materializes guest names in the host namespace.
It does not make stored bytes safe against a separate actor with stronger operating-system authority or a retained writable handle.
This slice does not implement reachability garbage collection or an internal store quota.
Tenant admission requires an external enforced capacity boundary until transactional staging or reachability collection lands and is tested.

This artifact is not a mounted filesystem, disk image, kernel bundle, guest-agent bundle, snapshot, compatibility certificate, signature, Generation, sandbox, or readiness result.
The current VMM cannot consume it because it has no root block device.

## Consequences

Root filesystem semantics and identity now live behind one small interface instead of leaking into disk builders and VMM adapters.
The portable normalizer can run deterministically on Linux, macOS, and Windows because it performs no privileged host metadata operations.
Later disk compilers consume one normalized tree artifact and must preserve its node semantics or fail closed.

A later ADR must select and pin the raw guest filesystem format, compiler version, feature set, UUID and hash seeds, capacity policy, and verification procedure.
The first KVM consumption proof must then attach that disk through a minimal block device and demonstrate an exact guest mount and file read.
Snapshot, networking, authenticated readiness, and request-time performance remain outside this slice.

## Verification

Public-interface tests cover import through normalization, deterministic repetition, concurrent idempotence, atomic-last publication, and distinct source provenance with identical final tree identity.
Semantic tests cover ordinary and opaque whiteouts, directory metadata replacement, type replacement, implicit directories, symlink non-resolution, stable hard-link groups, ownership, modes, modification time, FIFO nodes, and non-UTF-8 paths.
Hostile tests cover malformed whiteouts, unsafe paths, non-directory parents, unresolved and cyclic hard links, sparse rejection before body reads, malformed, duplicate, global, oversized, repeated zero-length, and unsupported PAX metadata, aggregate raw-header and extension-byte work bounds, rejected xattrs, tampered stored import artifacts and layers, every configured bound, checked counter overflow seams, redacted diagnostics, and absence of a completion artifact on failure.
Identity tests require local PAX `path` and `linkpath` values to produce the same canonical tree as equivalent ordinary headers.
The canonical binary manifest has pinned golden bytes and a pinned SHA-256 identity, and plain and gzip transports are required to normalize to the same tree identity.
An ignored Apple Container `node:22` test requires explicit retained import digest, tree digest, tree size, and entry count evidence rather than accepting repeatability alone.
It cannot prove a guest mount, x86_64 KVM boot, sandbox execution, snapshot restore, or launch latency.
