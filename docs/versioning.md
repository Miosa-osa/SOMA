# SOMA versioning and releases

## Version line

SOMA follows Semantic Versioning 2.0.0.
The first stable release will be `1.0.0`.
Development builds leading to that release use `1.0.0-alpha.N` and `1.0.0-rc.N` versions so an unfinished sandbox cannot be confused with the stable release.
The current source version is recorded in both `VERSION` and the Cargo workspace package metadata.

After `1.0.0`:

- A backward-compatible defect or security fix increments the patch version, such as `1.0.1`.
- A backward-compatible capability increments the minor version, such as `1.1.0`.
- An incompatible change to a stable public contract increments the major version, such as `2.0.0`.

## Stable public surface

The stable compatibility promise covers the portable library use cases, documented host operations, request and receipt semantics, fault behavior, command-line interface, and supported configuration.
Generation manifests, guest-agent messages, snapshots, and local wire frames carry their own explicit format or protocol versions because stored artifacts can outlive one process release.
A release must reject incompatible artifacts and messages rather than guessing, silently upgrading, or falling back to a weaker execution path.

An internal Rust module, private trait, unpublished test adapter, or diagnostic field is not a stable public contract unless the release documentation says otherwise.
Changing an implementation detail does not require a major release when externally observable semantics and supported artifacts remain compatible.

## `1.0.0` admission gates

The `v1.0.0` tag is forbidden until one exact revision earns all of this evidence:

- Locked formatting, linting, unit, integration, documentation, architecture, dependency, workflow, and secret checks pass.
- Ubuntu 24.04 x86_64 on a dedicated bare-metal host opens `/dev/kvm` and passes the required KVM capability checks.
- A digest-pinned OCI image is reproducibly converted into a certified Generation through the documented production pipeline.
- The public product path takes that OCI-derived Generation through a hardware-isolated KVM sandbox, authenticates the expected guest, repairs clone identity, reaches Ready, executes one bounded command, and stops with complete ownership-ledger cleanup.
- The public one-shot and managed-Machine library paths return versioned execution receipts that identify exact workload, backend, isolation, preparation, effective shape, timing boundary, outcome, and cleanup state without exposing secrets.
- Portable crates compile and pass their behavioral contract on supported Linux, macOS, and Windows client targets.
- Deterministic contract adapters, cross-builds, an empty-VM KVM probe, or a hand-built guest fixture cannot substitute for the OCI-to-isolated-sandbox product path.
- Failure injection proves that incompatibility, authentication failure, timeout, guest exit, and partial resource acquisition fail closed.
- The documented internal create and first-command latency targets pass on the certified host with retained raw samples.
- The exact 100-way ComputeSDK Burst TTI benchmark passes with median below 50 ms, p99 below 90 ms, 100 successful commands, and 100 successful cleanups.
- Release artifacts are built from the tag, carry SHA-256 checksums, and can be tied back to the source commit.
- The release notes state the exact supported host, architecture, artifact format, limitations, and retained validation evidence.

The first stable release requires the complete correctness, isolation, cleanup, and performance evidence together.
Passing macOS tests or a Linux cross-build cannot replace the real KVM gates.

## Release identity

Stable releases use annotated tags in the form `vMAJOR.MINOR.PATCH`.
Pre-releases use tags such as `v1.0.0-alpha.1` and `v1.0.0-rc.1`.
The tag version, `VERSION`, Cargo workspace version, crate versions, and generated artifact names must agree exactly.
A mismatch fails the release before an artifact is published.

The release workflow never changes the source version itself.
A reviewed source commit establishes the version first, and the immutable tag selects that exact commit for validation and packaging.
GitHub-generated release notes provide the change history so agents do not manually edit a generated changelog.

## Support and backports

Before `1.0.0`, pre-release interfaces can change without backward compatibility, but each change still requires tests and an architecture decision when it affects a documented seam.
After `1.0.0`, security fixes are prepared against every supported release line that is affected and can be corrected safely.
The security policy names supported lines explicitly rather than assuming that every historical tag receives fixes.

A patch release must not weaken isolation, compatibility checks, cleanup, authenticated readiness, or benchmark accounting.
If a safe correction requires an incompatible contract, SOMA publishes a new major version instead of disguising the break as a patch.
