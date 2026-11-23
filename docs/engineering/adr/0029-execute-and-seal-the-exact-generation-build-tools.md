# ADR 0029: Execute and seal the exact tools that build a Generation

- Status: Accepted
- Date: 2026-08-30
- Extends: ADR 0025 and ADR 0027

## Context

The Generation compiler recorded the formatter's provenance by hashing a program at a host path and then spawning that same path.
Nothing bound the two operations to the same bytes: a rename, a symlink repoint, or a package upgrade between the hash and the spawn produced a Generation whose evidence named one executable while a different one shaped the artifacts.

The binding was also partial.
Only the two filesystem formatters were hashed at all.
The EROFS checker was version-probed but never hashed, and the ext4 populator, checker, and inspector were neither hashed nor probed, although the populator creates the two directories every overlay template ships with and the other two decide whether a template is accepted.

The manifest carried `builder_image_digest` as an optional field that the host build always left `None`, so a Generation could name no builder identity at all and verification had nothing to require.

The implementation audit of 2026-08-29 records this as Priority 1 finding P1.4.

## Decision

### Execution is bound to the measured descriptor

Every external tool is a `PinnedTool`: opened once, hashed through that one open file description, and executed through that same description.
Execution names `/proc/self/fd/N` rather than the original path, so the kernel re-opens the object the descriptor already holds and a replaced path cannot change what runs.
The descriptor path is proved to reach the same device and inode when the tool is opened and again immediately before every spawn, so a system that publishes no working descriptor paths fails as unsupported instead of quietly executing a name again.
The parent keeps its copy close-on-exec and only the child that is about to execute those exact bytes inherits the descriptor, which a `#!` tool needs because its interpreter opens the path after the first `execve`.

The compiler still owns exactly one `unsafe` module, `process/control.rs`, which now holds the group signal and the child-side `fcntl` that clears close-on-exec.

### Every material tool is bound

The six tools that materially shape or judge an artifact are bound: the EROFS formatter and checker for the root, and the ext4 formatter, populator, checker, and inspector for the overlay templates.
Each is bound by its bare name, the digest of the executable that ran, and the revision that executable reported.

### The builder environment is sealed and required

`BuilderEnvironment` holds those bindings in canonical name order and hashes them into one digest over a domain-separated, count-prefixed, length-prefixed serialization.
Binding one name to different bytes or a different revision is an integrity failure, and an environment that bound no tool has no digest.

`RootBinding::builder_image_digest` becomes `RootBinding::builder_environment_digest`, a required `Sha256Digest` rather than an optional one.
The compiler always sets it and profile verification always requires it to be nonzero, so no Generation can be published or accepted without naming its complete toolchain identity.

This renames the field rather than keeping the old name, because the value is not an OCI builder-image digest: the host build still executes the pinned tools directly.
Running them inside a content-addressed builder image remains open work, and when it lands the image digest becomes one more bound member of this environment rather than a replacement for it.

The manifest encoding drops the optional-presence byte in favor of the digest, which changes the canonical `SOMAGEN` bytes and therefore the golden vector and `GenerationId` of the pinned fixture.

## Verification

A pinned tool executes the bytes it measured after its path has been removed and replaced with a different executable, and reports the original program name in its evidence.
A pinned tool's digest equals the SHA-256 of the file on disk, its program path is not its original path, and its binding still holds immediately before a spawn.
A directory and an empty file are not pinnable tools, and a missing tool fails with the phase that asked for it.

The builder-environment digest is independent of binding order, changes when any name, digest, or revision changes, changes when a tool is added or removed, and cannot be confused across the name and revision length prefixes.
Duplicate binding is idempotent for an exact restatement and an integrity failure otherwise, the bound-tool count is limited, and names must be bare printable file names.

A real profile v1 build over the pinned `erofs-utils` and `e2fsprogs` binds exactly the six material tools, each to the SHA-256 of the executable on disk, each with a nonempty revision.
The published manifest's builder-environment digest equals the seal over those six, differs from the root formatter digest alone, and changes when any one of the six is dropped from the seal.

## Consequences

Provenance now names the process image the kernel loaded, and it names every tool that shaped or judged an artifact rather than one formatter per phase.

Adding a child-side `pre_exec` step means tool spawning no longer takes the `posix_spawn` fast path, which is irrelevant at the scale of one build tool per phase.

A tool that lives on a filesystem whose descriptors have no path, or a system without `/proc`, cannot be pinned, and the compiler refuses rather than degrading.

This decision does not add a signature, an attestation, a reproducible builder image, or a hermetic build sandbox.
It binds which bytes ran; it does not prove that those bytes are trustworthy.
