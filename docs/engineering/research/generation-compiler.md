# SOMA Generation compiler v1

## Decision

The SOMA Generation compiler is an offline, fail-closed pipeline that converts one verified normalized OCI tree into immutable machine artifacts.
It does not run in `soma-vmm`, on the Launch path, or with tenant runtime authority.

Generation version 1 contains:

- One deterministic uncompressed x86_64 Linux ELF kernel with the required PVH entry note.
- One deterministic initramfs containing the pinned early init and guest agent.
- One deterministic read-only EROFS root filesystem compiled from the normalized OCI tree.
- One sterile ext4 overlay template used only to create an Instance-private writable head.
- One certified memory snapshot captured at the disconnected repair point.
- Canonical machine, device, CPU, guest-protocol, snapshot, and compiler contracts.
- Content digests, byte sizes, provenance, compatibility requirements, and retained validation evidence.

The guest mounts EROFS as the read-only lower filesystem and one private ext4 block device as the OverlayFS upper and work filesystem.
This corrects the earlier one-disk assumption in device ticket #5.
It preserves byte-reproducible immutable input while allowing each sandbox to have an independently sized writable filesystem.

This document resolves the architecture and reproducibility question in decision-map ticket #6.
It does not claim that the production compiler, certification, or an integrated OCI sandbox exists.
Status words here are the five terms defined in [the engineering standard](../standards/sota-engineering-standard.md#status-vocabulary): designed, component-tested, live-proved, integrated, production-admitted, and [the claim ledger](../claim-ledger.md) carries them in one table.

## Why EROFS

The compiler compared three practical choices.

| Format | Strength | Blocking problem for v1 |
| --- | --- | --- |
| Writable ext4 base | One guest block device and familiar tooling | `mke2fs -d` copied host inode change times in the tested Ubuntu 24.04 toolchain, so two builds in different seconds produced different bytes despite fixed UUID, hash seed, and `SOURCE_DATE_EPOCH`. |
| SquashFS lower plus overlay | Mature read-only image and established microVM pattern | Its large option space and compressor behavior need separate pinning, while EROFS is designed for container image and page-cache-oriented use cases. |
| EROFS lower plus ext4 overlay | Deterministic read-only artifact, direct tar and OCI-oriented tooling, fixed timestamps and UUID, independent writable capacity | Requires a second virtio block device and a small pinned early-mount sequence. |

EROFS is selected because immutable root bytes are easier to reproduce, verify, share, and cache than a pre-writable filesystem.
The current EROFS project explicitly documents reproducible builds with fixed timestamps and UUIDs.
Containerd also documents EROFS as a snapshotter and records the fixed-time options required for reproducible images.

The additional block device is deliberate rather than accidental complexity.
It separates trusted immutable Generation bytes from disposable tenant writes and makes the writable capacity an Instance property instead of a Generation property.

## Primary sources

- The [EROFS filesystem documentation](https://erofs.docs.kernel.org/en/latest/mkfs.html) defines image construction, fixed timestamps, fixed UUIDs, and tar input.
- The [erofs-utils repository](https://github.com/erofs/erofs-utils) is the primary formatter and checker implementation.
- The [erofs-utils 1.9.4 release commit](https://github.com/erofs/erofs-utils/commit/f36cadb5c563995ab3aa8572a60ed6b721b9557d) is the exact formatter revision tested by the prototype.
- The [containerd EROFS snapshotter documentation](https://github.com/containerd/containerd/blob/main/docs/snapshotters/erofs.md) documents production container integration and reproducible-build flags.
- The [OverlayFS documentation](https://docs.kernel.org/filesystems/overlayfs.html) defines lower, upper, work, copy-up, and whiteout behavior.
- The [ext4 documentation](https://docs.kernel.org/filesystems/ext4/index.html) defines the writable upper filesystem used by the first guest profile.
- The [e2fsprogs `mke2fs` manual](https://github.com/tytso/e2fsprogs/blob/master/misc/mke2fs.8.in) documents fixed hash seeds and lazy-initialization behavior.
- The [Firecracker root filesystem guide](https://github.com/firecracker-microvm/firecracker/blob/main/docs/rootfs-and-kernel-setup.md) demonstrates direct Linux boot with an external root filesystem.
- The [Firecracker containerd image builder](https://github.com/firecracker-microvm/firecracker-containerd/blob/main/tools/image-builder/README.md) documents the read-only-root plus writable-overlay pattern.
- The [OCI Image Specification v1.1.1](https://github.com/opencontainers/image-spec/tree/v1.1.1) defines the source image semantics already verified by SOMA.

## Pipeline boundaries

The compiler has six explicit phases.
Each phase consumes immutable verified inputs and publishes its output only after complete validation.

```text
verified OCI layout
    |
    v
ImportedOci
    |
    v
NormalizedRootfs logical tree and content objects
    |
    +--> canonical tree stream --> pinned EROFS builder --> immutable root disk
    |
    +--> pinned kernel configuration and source --> uncompressed PVH kernel
    |
    +--> pinned early init and guest agent --> deterministic initramfs
    |
    +--> sterile overlay recipe --> ext4 overlay template
    |
    v
cold boot, authenticated repair, quiesce, capture, validation
    |
    v
canonical Generation manifest --> GenerationId
```

### Phase 1: resolve inputs

The compiler accepts an existing `NormalizedRootfs`, an explicit content store, and one versioned compiler profile.
It reopens and verifies the normalized-tree manifest and every referenced content object through capability-relative handles.
It rejects a platform other than `linux/amd64`, a preexisting `GenerationId`, an unsupported node type, an unresolved hard link, missing content, size mismatch, digest mismatch, or exceeded bound.

The compiler profile contains no registry credential, cloud identifier, host path, current time, random seed, or caller-supplied shell fragment.

### Phase 2: emit the canonical filesystem stream

The compiler decodes the canonical tree manifest through a bounded schema-owned decoder.
Entries are emitted in raw path-byte order.
The root entry is handled explicitly and does not become a second nested directory.

Each directory, file, hard link, symbolic link, and FIFO carries the normalized numeric ownership, supported mode bits, and integral modification time.
Regular-file bodies stream from already verified content objects.
No entire file or root filesystem is retained in memory.
Hard-link aliases refer only to the canonical preceding anchor.

Production construction uses the pinned EROFS tar input mode so guest paths never need to become paths in the host namespace.
If the selected formatter build cannot consume the canonical stream with the required semantics, the profile is unsupported.
The compiler must not fall back to extracting tenant paths into an ordinary host directory.

The stream has explicit bounds for entries, path bytes, link bytes, metadata bytes, individual file bytes, aggregate bytes, and output bytes.
It contains no device node, socket, sparse extent, xattr, ACL, capability, or security label because normalized-rootfs version 1 rejects them.

### Phase 3: build and verify immutable artifacts

The version 1 EROFS profile pins:

- `erofs-utils` commit `f36cadb5c563995ab3aa8572a60ed6b721b9557d`, released as 1.9.4.
- Filesystem format and formatter options as one immutable compiler-profile digest.
- A 4 KiB filesystem block size.
- No compression for the first correctness profile.
- No fragments, deduplication, tail-packing extension, chunk mode, incremental mode, rebuild mode, remote source, or host xattr import.
- One fixed filesystem UUID derived by a documented domain-separated transformation of the normalized-tree digest.
- One fixed volume label, `SOMA_ROOT`.
- `--all-time` with the normalized policy epoch.
- One thread for any future compressed profile unless cross-thread reproducibility is separately proved.

The builder runs inside a content-addressed Linux builder image with no network after all inputs are installed.
The profile binds the builder image digest, formatter executable digest, formatter revision, target architecture, and exact arguments.
The formatter's ambient configuration files and environment are empty or explicitly pinned.

After formatting, `fsck.erofs` must pass.
The verifier then independently walks the EROFS image and compares every path, type, mode, numeric owner, modification time, link target, hard-link group, file size, and content digest with the normalized tree.
A successful formatter exit alone is insufficient.

The kernel, initramfs, guest agent, and sterile overlay template follow the same build and independent-verification rule.
The kernel verifier checks the ELF program headers, PVH entry note, configuration digest, linked address, and required built-in drivers.
The initramfs verifier decodes the archive and checks its exact allowlisted paths and artifact digests.
The overlay verifier runs read-only `e2fsck`, checks its UUID, capacity, features, block size, empty upper and work directories, and absence of tenant data.

### Phase 4: boot and capture

Generation construction boots the immutable EROFS root with one private clone of the sterile overlay template.
The pinned early init mounts both filesystems, creates the OverlayFS root, pivots into it, and starts the pinned guest agent.

The builder completes a fresh authenticated guest session, validates a known file from the OCI tree, validates a private write and flush, and proves the EROFS artifact remained unchanged.
It then drives the guest to the disconnected repair point defined by the guest-agent ticket.

At quiesce:

- No user workload has run.
- No vsock session or Noise key remains live.
- No network packet or live network identity remains.
- Both block queues and the entropy queue are empty.
- The overlay is clean and contains only invariant early-boot state approved by the profile.
- The guest agent is blocked waiting for fresh launch material.
- The filesystem durability boundary completed.

Only then may the builder capture memory, vCPU, interrupt, clock, and device state.

### Phase 5: certify compatibility

The certification runner restores the candidate snapshot on every host profile it intends to admit.
It performs fresh identity, entropy, time, vsock, and network repair and executes the fixed readiness command.
It repeats restore, command, and cleanup under the conformance counts required by ticket #14.

The certificate records exact host-kernel, KVM API, KVM capability, CPU template, machine-contract, device-contract, guest-kernel, and builder identities.
Certification evidence is not folded into the content identity because adding an independent certified host must not change immutable machine bytes.
The Generation manifest points to the certificate policy and retained evidence set separately.

### Phase 6: publish atomically

Every artifact is written to a private staging object, hashed through the same open handle, verified, marked immutable, and published under its digest.
The canonical Generation manifest is published last with create-exclusive semantics.
Failure leaves no discoverable complete Generation.

Concurrent builds of the same inputs either converge on identical verified objects or fail closed on a conflicting byte sequence.
Orphaned staging objects are reclaimed outside the compiler transaction.

## Generation identity

`GenerationId` is `sha256:` plus the SHA-256 digest of the canonical `SOMAGEN` version 2 manifest bytes.
It is not the OCI manifest digest, normalized-tree digest, disk digest, or snapshot digest alone.

The manifest uses fixed-order binary fields with big-endian integers, explicit presence bytes, and length-prefixed bounded byte strings.
It contains no map with implementation-dependent ordering.

The identity binds:

1. Manifest schema and compiler-policy versions.
2. Source OCI manifest digest and effective OCI platform.
3. Normalized-tree manifest digest and byte size.
4. EROFS image digest, byte size, UUID, format profile, formatter executable digest, formatter revision, and builder-environment digest.
5. Sterile ext4 overlay-template digest, byte size, filesystem UUID derivation, feature profile, and minimum and maximum supported writable capacities.
6. Kernel digest, byte size, ELF and PVH contract version, kernel configuration digest, and CPU architecture.
7. Initramfs digest, byte size, layout version, and early-init digest.
8. Guest-agent executable digest, byte size, build provenance, and application and handshake protocol versions.
9. Complete kernel command line bytes.
10. Machine-contract version and digest.
11. Device-contract version and digest, including MMIO map, GSIs, queues, and feature allowlists.
12. CPU-template version and digest.
13. Memory size, vCPU count, memory-slot layout, and immutable launch-page layout.
14. Snapshot-format version, memory digest and size, captured-overlay digest and size, machine-state digest and size, and capture-point version.
15. Required repair-policy version and readiness-command digest.

Changing any bound field produces a different `GenerationId`.
Changing only a registry tag, host path, build location, build time, certificate set, or retained benchmark evidence does not.

The Generation manifest contains descriptors for every artifact rather than embedding large data.
Every descriptor contains media type, digest, and exact byte size.
Unknown field, version, media type, duplicate descriptor role, trailing byte, unsupported optional state, or digest mismatch rejects the Generation before KVM creation.

## Kernel contract

The first kernel target is Ubuntu 24.04-compatible x86_64 Linux built as an uncompressed ELF image with the PVH entry note required by machine contract v1.
The exact upstream commit, configuration, toolchain image, compiler versions, and output digest are pinned.

The required built-in facilities include KVM paravirtualization, PVH entry, virtio core, modern virtio-mmio, virtio block, virtio network, virtio vsock, virtio entropy, EROFS, ext4, OverlayFS, devtmpfs, procfs, sysfs, tmpfs, Unix sockets, and the cryptographic primitives used by the guest agent.
Modules are disabled for the first profile.
PCI, ACPI, graphics, USB, audio, SCSI, ballooning, device hotplug, and arbitrary filesystems are excluded.

The kernel command line is generated from fixed ordered fields.
It declares all five MMIO devices, disables unsupported platform facilities, identifies the EROFS lower and ext4 upper devices, selects the pinned early init, and contains no caller text.

## Initramfs and early init

The initramfs is a deterministic `newc` archive with lexicographically ordered entries, fixed ownership, fixed modes, fixed timestamps, zeroed padding, and no host-specific metadata.
Its minimum contents are the statically linked early init, the statically linked SOMA guest agent or its verified target path, required device nodes created by devtmpfs, and only the libraries proven necessary by the selected linking profile.

Early init performs one bounded sequence:

1. Mount devtmpfs, procfs, and sysfs with fixed options.
2. Wait for exactly the expected virtio block devices within the boot deadline.
3. Mount the EROFS lower read-only with fixed options.
4. Mount the private ext4 upper with fixed options and reject a dirty or unexpected identity.
5. Create or verify the upper and work directories on the private filesystem.
6. Mount OverlayFS with the EROFS lower and private upper and work directories.
7. Pivot into the composed root and detach the temporary root.
8. Start the guest agent as PID 1 or transfer PID 1 responsibility through one pinned supervisor contract.
9. Enter the disconnected repair wait state.

Any unexpected device, filesystem UUID, mount option failure, dirty upper, missing agent, extra authority, or timeout causes a panic or controlled shutdown before Ready.

## Writable capacity

The immutable EROFS image size belongs to the Generation.
Writable disk capacity belongs to the Instance shape.

The initial host profile supports a bounded set of preformatted sterile ext4 overlay sizes.
The allocator selects an exact compatible size and creates a private reflink head before assignment.
Launch never runs `mkfs`, grows a filesystem, scans an image, or copies the immutable root.

Arbitrary byte-level disk sizing is rejected in the fast profile because formatting or online resizing on Launch would destroy the latency budget and complicate snapshot compatibility.
Operators may publish additional certified size classes without changing the EROFS Generation identity.
The selected overlay-template digest and capacity appear in the Instance execution receipt.

Ticket #11 must measure reflink creation and determine whether sterile private heads must be created before Launch.

## Security boundaries

The compiler handles hostile OCI-derived metadata and content but does not run that content.
Its filesystem formatter is still a complex parser and runs in a disposable builder sandbox with:

- No KVM descriptor or production VMM process.
- No network after dependencies and inputs are present.
- A read-only toolchain and source input.
- One writable bounded staging volume.
- User, mount, PID, IPC, UTS, cgroup, and network namespaces.
- No host path outside capability-transferred inputs and staging output.
- No ambient capabilities, device access, secret, credential, control-plane socket, or cloud metadata route.
- Fixed CPU, memory, process, file-size, descriptor, and wall-clock limits.
- Complete process destruction after one build.

The compiler never mounts the produced guest filesystem on the production host.
Independent verification uses bounded userspace readers inside the same disposable trust boundary.
Kernel mounting happens only inside a disposable certification guest.

## Prototype result

Run the retained throwaway proof with:

```text
./scripts/prototype-generation-compiler.sh
```

The proof builds `erofs-utils` revision `f36cadb5c563995ab3aa8572a60ed6b721b9557d` inside the content-addressed Ubuntu 24.04 image `sha256:561618e2c15bf2397621dd04f96926663a3b5616c189cf7e38db7e82f5c538ea`.
It creates two logically identical trees in opposite insertion orders containing directories, ordinary files, a hard link, a symbolic link, a FIFO, executable mode, and sticky mode.
It fixes the filesystem timestamp and UUID, compiles both uncompressed EROFS images, requires byte-for-byte equality, and runs `fsck.erofs`.

The proof passed on the Apple Silicon development machine through Docker's ARM64 Linux VM.
It proves the small fixture is reproducible under that builder execution.
It does not prove canonical-tree decoding, tar streaming, arbitrary OCI semantics, cross-architecture equality, x86_64 guest mounting, OverlayFS boot, KVM execution, snapshot capture, or production security.

The ext4 experiment was also valuable negative evidence.
Two builds within one second matched, but builds in different seconds changed because directory population retained host change-time state not controlled by `SOURCE_DATE_EPOCH` in the tested path.
SOMA therefore does not use populated ext4 as its immutable content-addressed root artifact.

## Production implementation modules

The implementation belongs in small modules under `soma-generation`:

```text
generation/
  request.rs          explicit compiler input and bounds
  tree_decoder.rs     hostile canonical manifest decoder
  tar_stream.rs       ordered bounded EROFS source stream
  artifacts.rs        typed role, media type, digest, and size
  erofs.rs            pinned formatter invocation and evidence
  overlay.rs          sterile ext4 template contract
  kernel.rs           ELF, PVH note, config, and provenance checks
  initramfs.rs        deterministic newc construction and verification
  manifest.rs         canonical SOMAGEN v2 encoder and decoder
  identity.rs         GenerationId derivation only
  publish.rs          atomic-last content-store publication
  verify.rs           independent cross-artifact verification
```

No module may accept an arbitrary command line, shell string, host path embedded in an artifact, unbounded reader, or untyped artifact role.
The formatter process adapter is replaceable, but the canonical SOMA input and output contracts are not delegated to the formatter.
Under ADR 0025 every external tool leads its own process group, one supervising thread owns its deadline, termination, and reaping, and a deadline, feed failure, capture failure, or cancellation terminates the complete group inside a declared grace with the invoking phase in the error.

## x86_64 production-module status

The modules above exist under `crates/soma-generation/src/generation/` and are component-tested for phases 1 through 3 and 6 on Linux x86_64 with the pinned host toolchain; phase 4 is partial and phase 5, certification, is designed only.
Under ADR 0026 the compiler therefore publishes a Generation Candidate, not a Generation: the Candidate has its own magic, media type, and `CandidateId`, no Launch or resolution interface accepts it, and only `certify_candidate` followed by `promote_candidate` can publish a ready `SOMAGEN` manifest.
Under ADR 0027 every decoded manifest field is validated as hostile input with checked arithmetic, explicit version support, cross-field relations, and one typed redacted `Incompatibility` per failed invariant.
The compiler input is one `TemplateRevision` (selected image reference plus resolved digest and platform, Machine shape with vCPU count, memory, and writable-storage size class, network policy intent inside the shape capabilities, startup and readiness behavior, lifetime limits, and preparation profile version) together with its `NormalizedRootfs`; every field except the image reference is bound by the manifest, and the manifest therefore carries a sixteenth group for the Template fields not already covered by the fifteen listed above.
The formatter consumes the canonical tar stream through its standard input rather than a host tar file or host directory, and the independent verifier is a crate-private bounded EROFS reader that parses the superblock, directories, inodes, and data without mounting or extracting; `fsck.erofs --extract` is used only as a test oracle.
Measured deviations: `mke2fs -d` copied host change times even under `E2FSPROGS_FAKE_TIME`, so the empty `upper` and `work` directories are created by `debugfs -w -R mkdir` under the same fake time, which was byte-reproducible across seconds; the pinned `--all-time` option flattens every per-file modification time to the profile epoch, which the verifier checks instead of the per-entry tree time.
Under ADR 0029 every external tool is executed through a descriptor that was opened and hashed before the spawn, and the six tools that materially shape or judge an artifact are sealed into one required builder-environment digest; that digest is not an OCI builder-image digest, because the host build still runs the pinned tools directly rather than inside a content-addressed builder image.
The CPU-template digest covers a declaration statement rather than defined CPUID masks, and the fixture proof covers a small synthetic tree because the normalized `node:22` store was not present on the Linux host.
Gates 1, 4, 5, and the atomic-last and timeout portions of 8 have test evidence; gate 2 still lacks a same-input second `node:22` build, because the `node:22` revision cached on this host normalized to a different tree than the development Mac's recorded run and was compiled once; gate 3 holds for the machine artifacts while the `GenerationId` still differs through the bound source OCI manifest digest; gates 9 and the disk-exhaustion and crash cases of 8 remain designed.
Gate 6 is live-proved at `71161ea`, historically: the pinned guest mounted the EROFS root and the private ext4 overlay, composed the writable root, executed a file from the OCI tree, and left the EROFS artifact byte-identical while the private head changed, as recorded in [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md).
That run used initramfs layout v2, so its artifact digests are not reproducible on current code.
Gate 7 is live-proved at `5d71524` for restored Instances, in [the capture and restore on the per-Instance authority design](../evidence/2026-08-30-x86_64-snapshot-restore-current-authority.md).
Initramfs layout v2 added the `/dev/console` and `/dev/null` nodes PID 1 needs before devtmpfs is mounted, plus a Generation-scoped responder private key at `/etc/soma/responder.key` supplied as a fifth machine input.
Initramfs layout v3 removes that key and its `etc/soma` directory under [ADR 0024, per-Instance guest responder authority](../adr/0024-per-instance-guest-responder-authority.md), so the compiler takes four machine inputs and no secret input at all; the responder static secret is fresh per Instance and reaches the guest only through the non-snapshot launch page.
Phase 4 is therefore partial: boot, the authenticated session, quiesce, and memory capture are live-proved at `5d71524`, while the compiler itself still performs none of them and publishes `SnapshotBinding::Absent`, so no launchable snapshot binding exists.

## Acceptance gates

Ticket #6 counts as component-tested, rather than merely designed, only when all of these gates pass:

1. The production decoder consumes real `NormalizedRootfs` artifacts and rejects every malformed field and bound.
2. Two isolated builds of the same Node 22 normalized tree produce byte-identical EROFS, initramfs, manifest, and `GenerationId` artifacts.
3. Reordering OCI layers without changing final filesystem semantics produces the same immutable machine artifacts except explicitly excluded provenance.
4. Changing file content, metadata, kernel, command line, guest agent, machine contract, device contract, CPU template, or snapshot state changes `GenerationId`.
5. Independent EROFS traversal equals the normalized logical tree exactly.
6. The pinned x86_64 guest mounts EROFS plus the private ext4 overlay and reads and writes the expected files.
7. Two Instances from one Generation cannot observe each other's writes and cannot change the EROFS base or sterile overlay template.
8. Builder timeout, crash, malformed formatter output, digest conflict, disk exhaustion, and concurrent identical build all preserve atomic-last publication.
9. The builder sandbox cannot access the network, KVM, host root, production sockets, credentials, or artifacts outside its transferred capabilities.
10. Raw evidence retains builder image, tool revisions and digests, arguments, inputs, outputs, verifier results, timing, and the SOMA revision.

The next dependency is ticket #7, which defines the canonical snapshot format and private memory restore contract consumed by this Generation manifest.
