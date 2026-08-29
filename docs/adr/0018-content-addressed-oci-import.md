# ADR 0018: Content-addressed OCI import foundation

- Status: Accepted
- Date: 2026-08-28

## Context

Generation construction needs a verified immutable OCI input before it can safely build a root filesystem, guest image, kernel bundle, or snapshot.
An OCI image-layout may point directly to an image manifest or through bounded nested indexes, including the top-index to nested-index to platform-manifest shape emitted by Apple Container 1.3.
OCI descriptors make the referenced bytes content-addressed, but an importer still has to enforce size, digest, platform, layer ordering, decompression, filesystem, and publication rules.
Calling this intermediate input a Generation would incorrectly claim that later construction, compatibility certification, and boot verification already happened.

Apple Container exports also expose an identity boundary.
Two saves of the same cached image can produce different synthesized index bytes when annotation map order changes even though the selected manifest, config, and layers are identical.
Export-only traversal metadata must not make the canonical imported image identity unstable.

## Decision

The `soma-generation` crate exposes one deep entry point, `import_oci_layout`, which accepts explicit layout and store paths, one selection, and explicit limits.
The function returns `ImportedOci` and never creates or returns a `GenerationId`.
An exact selection containing a preexisting `GenerationId` is rejected.

The accepted source is an already extracted OCI image-layout version 1.0.0.
Archive extraction, registry resolution, tag resolution, image building, and network access remain outside this module.
The importer supports a direct OCI image manifest and at most eight bounded index levels.
The descriptor budget includes index entries plus the selected manifest's config and ordered layers.
Unknown descriptor media types are counted and skipped as OCI auxiliary content rather than causing an error.
The top and nested index `mediaType` field may be absent, but any declared value must be the exact OCI image-index media type.

Every supported descriptor has a nonnegative signed wire size converted with checked arithmetic.
Every traversed supported index, selected manifest, config, and compressed layer is read under a configured bound and must match both its exact descriptor size and SHA-256 digest.
Metadata documents are additionally capped at 4 MiB.
The manifest must be OCI schema version 2 with one OCI image config and an ordered layer list.
The config must declare a valid operating system, architecture, optional variant, a `layers` rootfs type, and exactly one ordered diff ID per layer.
Because this slice cannot enforce operating-system versions or features, it rejects any declared `os.version` and any nonempty `os.features` requirement instead of discarding them.

Platform selection reconciles platform claims on every selected nested-index descriptor, the manifest descriptor, and the verified config.
All claims must agree on operating system and architecture.
Two concrete variants must agree, and when exactly one verified source supplies a concrete variant that variant becomes the effective platform.
A generic platform request may select one unique concrete effective platform, while a requested concrete variant must exactly match the effective platform.
Exact-digest selection can proceed without a descriptor platform, but its requested platform must still match the merged effective platform.
An exact request with a generic platform is refined to a known concrete effective platform in `ImportedOci` instead of erasing verified variant information.

The first slice supports uncompressed OCI tar layers and gzip-compressed OCI tar layers.
It computes each expanded SHA-256 diff ID in manifest order without retaining expanded bytes and structurally validates the same expanded stream before publication.
Structural validation checks tar headers and sizes through `tar` 0.4.46, requires complete zero termination, rejects nonzero or misaligned trailing bytes, bounds logical entries and path metadata across all selected layers, rejects duplicate normalized entry paths, and prevents absolute or parent-traversing entry paths and hard-link targets.
Each logical entry and its path or link bytes are charged to that shared validation budget as the entry is observed, before the parser advances into its body or any later header.
Symlink targets remain bounded metadata because valid Linux images may contain absolute or parent-relative symlinks, and the later root filesystem applier must resolve them safely within its own destination authority.
It enforces per-blob, total selected-source, total expanded, and descriptor-count limits with checked counters.
Zstd layers, nondistributable layer media types, Docker schema media types, artifact manifests, and encrypted layers are rejected as unsupported.

The importer writes immutable objects below `v1/blobs/sha256` in a caller-created content-store root.
It opens each final layout and store root relative to its ambiently opened parent with no-follow semantics and rejects a final symlink or Windows reparse point.
It opens descendant directories and files without following symlinks, stages each object in a create-exclusive temporary file, verifies the bytes while copying, syncs the staged bytes, and publishes with a create-exclusive hard link.
Non-Windows systems mark the stage read-only before linking, while Windows removes the writable temporary link before marking the published link read-only because Windows does not remove read-only files portably.
Non-Windows publication syncs the containing directory after directory creation and hard-link publication.
Native Windows syncs every staged file but has no portable directory-fsync equivalent, so this slice claims atomic create-exclusive visibility there but not directory-entry crash durability.
An existing digest path is accepted only after its size and digest are verified, its read-only attribute is restored, and its bytes are verified again.
Publication never overwrites an existing object.
All selected layers are staged and expanded-verified before any selected layer is published.
The schema-owned import manifest is attempted last and appears through one atomic hard link, so earlier verification failures can leave harmless index, manifest, or config CAS objects but no selected layer or partial completion artifact.
A late directory-sync failure can leave a complete but unreturned import-manifest object, and an idempotent retry verifies and reuses it.

The schema-owned import manifest binds the exact selected workload platform, optional caller-supplied registry index digest, selected manifest descriptor, config descriptor, ordered layer descriptors, ordered expanded diff IDs, expanded sizes, and structurally validated logical entry counts.
Its fixed field order and format version make its SHA-256 digest deterministic.
The caller-supplied registry index digest is preserved only for exact selection and is treated as upstream resolution provenance.
This local importer does not claim to re-resolve or authenticate that registry identity.

The local `index.json` bytes and every nested index on the selected path are verified and stored as traversal provenance.
Their digests are returned separately in `traversed_indexes` and are deliberately excluded from the canonical import manifest.
Changing only synthesized index annotation order therefore changes provenance but not the canonical imported identity.
Platform selection does not synthesize a registry index identity from local export bytes.

## Safety boundary

The caller supplies existing layout and store roots and is responsible for their ownership and lifecycle.
Capability-relative no-follow operations prevent descendant symlink substitution, and exact descriptor-backed reads detect referenced blobs that grow or shrink while being read.
The unreferenced layout marker and top index have no external exact-size contract, so their reads enforce only the configured byte cap before their content is parsed and identified.
Final roots must end in a normal path component and cannot themselves be symlinks or Windows reparse points.
Ambient resolution of each root's ancestors still depends on a trusted, stable parent authority, and this slice does not attempt a component-by-component operating-system sandbox above that parent.
Digest verification fails closed if source bytes change between selection and publication.
Portable Rust cannot publish a hard link from an already verified open file handle, so this store requires exclusive trusted writer authority across `v1/tmp` and `v1/blobs`.
A competing writer can cause denial of service, and a crash after hard-link creation but before destination revalidation can leave an unreturned object that a retry will verify or reject.
This slice does not claim protection from an actor that can replace already opened directories, retain a writable handle to stored content, mutate the store with greater operating-system authority, or corrupt storage after successful return.

The tar parser bounds the complete expanded stream and the effective path and link metadata it returns.
A raw streaming preflight caps each GNU long-name and long-link record at 4,097 bytes and each local PAX record at 64 KiB before `tar` 0.4.46 can materialize an extension body.
Local PAX and GNU naming bodies across all selected layers share a 64 MiB extension-byte budget, while every global PAX header is rejected before its body is read.
All raw tar headers across all selected layers share a separate checked one-million-header work ceiling, including ordinary entries and extension records with empty bodies.
GNU sparse entries are rejected from their raw header before their body is read.
The verified staged handle is rewound before the complete structural parser and digest pass, so this allocation bound does not replace tar integrity verification.

`ImportedOci` proves only that the selected OCI inputs and deterministic import manifest were verified and stored.
It alone does not prove applied whiteouts, filesystem ownership or permissions, symlink policy, device-node policy, a Generation, disk image, kernel bundle, snapshot, signature verification result, authenticated guest identity, sandbox, or readiness result.
ADR 0019 defines logical filesystem application without host extraction, while disk materialization, x86_64 guest artifacts, Generation certification, and KVM boot verification remain later boundaries.

## Consequences

Later Generation construction receives one compact content-addressed input instead of reimplementing OCI traversal and integrity checks.
Identical selected image content produces the same canonical import digest even when local export indexes differ only in noncanonical provenance.
Failed and concurrent imports are idempotent at the object level and never overwrite a conflicting digest path.
The initial dependency surface remains synchronous and portable: `soma`, Serde JSON, SHA-256, pure-Rust gzip, `tar` 0.4.46, capability-based filesystem access, and a test-only temporary-directory helper.

## Verification

Default tests cover direct and nested indexes, absent and exact index media types, absent descriptor platforms, nested and leaf ARM64 variant reconciliation for generic and exact requests, exact caller index provenance, plain and gzip diff IDs, annotation-order stability, deterministic repeated and concurrent imports, and atomic-last completion behavior.
Hostile tests cover wrong index media types, unsupported operating-system requirements, negative sizes, size and digest corruption, concrete platform disagreement, malformed gzip and plain tar input, tar checksum and termination corruption, incremental aggregate raw-header, extension-byte, logical-entry, and path-metadata bounds, sparse rejection before body reads, unsafe or duplicate paths, escaping hard links, layer and diff-ID count mismatch, marker, index, descriptor, compressed, total, and expanded bounds, ambiguous selection, conflicting duplicate descriptors, unknown auxiliary media, source and final-root symlinks, substituted staged paths, corrupted existing CAS objects, redacted paths, structural nested-index cycles, and rejection of preexisting Generation identity.
A Windows-only publication test exercises the public import path without requiring unsupported directory fsync semantics.
An ignored live test imports a real extracted Apple Container OCI archive with the top-index to nested-index to Linux ARM64 manifest shape.
That Apple Silicon test proves only local OCI traversal, verification, deterministic manifest construction, and CAS publication.
It does not prove archive extraction safety, registry authenticity, x86_64 artifact construction, a bootable guest, KVM behavior, snapshot restore, sandbox isolation, or latency.
