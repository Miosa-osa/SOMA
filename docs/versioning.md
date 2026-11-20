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

## Protocol versions

Every wire protocol between two SOMA processes has one named constant in the crate that owns its encoding, and every binary lists the ones it speaks in `--version`.
A frame or message a build does not understand is refused, never guessed at.

| Name | Between | Owner | Where the number travels | Skew rule |
| --- | --- | --- | --- | --- |
| `guest-application` | the host runtime and the guest agent | `soma_guest::APPLICATION_PROTOCOL_VERSION` | the header of every frame | Exact. The agent is built into each Generation, so a new number needs Generations rebuilt with the new agent, and a host that must keep running older Generations has to accept both numbers before any Generation carries the new one. |
| `guest-api` | the host runtime and the guest agent | `soma_guest::GUEST_API_LEVEL` | the agent's answer to the level probe of ADR 0051 | Either side may be newer: a host sends a request of a level only to a guest that answered at least that level. Level 2 is stat, rename and ranged files; level 3 is an argument of up to 32 KiB in one `Execute` record (4 KiB before). |
| `launch-page` | the host runtime and the guest agent | `soma_guest::LAUNCH_PAGE_SCHEMA_VERSION` | the launch page itself | Exact: the page layout is part of what a Generation is certified with. |
| `hostd` | the local runtime and `soma-hostd` | `soma_hostd::protocol::PROTOCOL_VERSION` | not carried | Lockstep: a host runs the daemon and its clients from one release, and an unknown frame is answered with the `Protocol` failure code. |
| `netd` | the local runtime or `soma-hostd`, and `soma-netd` | `soma_netd::PROTOCOL_VERSION` | not carried | Lockstep, as for `hostd`. |
| `vmm-control` | the jail supervisor and the `soma-vmm` worker | `soma_vmm::control::CONTROL_PROTOCOL_VERSION` | not carried | Lockstep: the worker is executed from the installed release for each launch. |

The exchange between a client and its resident machine host has no number on purpose.
A resident machine host outlives an upgrade of the binary that started it, so a newer client talking to an older host is the normal case.
The exchange therefore grows only by new tagged variants, which an older host refuses, and an existing variant never changes shape.
A change that cannot follow that rule needs an architecture decision and a versioned exchange.

Stored artifacts such as Generation manifests, snapshots, the machine state store, and execution receipts carry their own format versions beside their encoders, and a release that does not know a version rejects the artifact.

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

## Build identity

Every shipped binary answers `--version`, given as its only argument, with the same report.

```text
soma-api 1.0.0-alpha.2 (2ec3789c4f1a 2026-11-20)
release: 1.0.0-alpha.2
commit: 2ec3789c4f1a8e9d0b1c2d3e4f5a6b7c8d9e0f1a
commit-short: 2ec3789c4f1a
commit-date: 2026-11-20T18:04:11Z
source: clean
profile: release
target: x86_64-unknown-linux-gnu
protocol.guest-application: 1
protocol.guest-api: 3
protocol.launch-page: 3
protocol.hostd: 1
protocol.netd: 1
```

The binaries are `soma`, `soma-mcp`, `soma-api`, `soma-hostd`, `soma-netd`, `soma-vmm`, and `soma-guest-agent`.
The first line is for people and the `key: value` lines after it are a stable contract, which `soma_version::ParsedReport` reads back.
`source` is `clean` when every file Git does not ignore matched the commit, `dirty` when one did not, and `unknown` when the build was not told.
A dirty build also marks its first line, as in `(2ec3789c4f1a-dirty 2026-11-20)`.
`soma version --format json` carries the same facts in the `soma.cli.v1` envelope.

The commit reaches a binary through `scripts/build-identity.sh`, which runs a build command with `SOMA_BUILD_COMMIT`, `SOMA_BUILD_COMMIT_DATE`, and `SOMA_BUILD_SOURCE` set from the local repository.

```sh
scripts/build-identity.sh cargo build --release --locked -p soma-api
```

The script reads no network and no clock: the date is the commit's own committer date, so one clean commit always produces the same report.
`soma_version::build!()` reads the three variables in the binary's own crate, and Cargo recompiles only those crates when they change.
A build script would have followed the commit too, but every crate that depends on it would then recompile on every commit, the host daemons and the local runtime included.
A binary built without the script reports its commit as `unknown`, which is true, rather than a commit it cannot prove.
The release workflow and `scripts/build-soma.sh` always build through the script.

The guest agent is the one exception and never records a commit.
It is built into every Generation and a Generation is addressed by its content, so a commit inside the agent would give every new commit a new Generation identity even when the agent had not changed.
The release manifest ties the agent's digest to the commit of its release instead.

## Support and backports

Before `1.0.0`, pre-release interfaces can change without backward compatibility, but each change still requires tests and an architecture decision when it affects a documented seam.
After `1.0.0`, security fixes are prepared against every supported release line that is affected and can be corrected safely.
The security policy names supported lines explicitly rather than assuming that every historical tag receives fixes.

A patch release must not weaken isolation, compatibility checks, cleanup, authenticated readiness, or benchmark accounting.
If a safe correction requires an incompatible contract, SOMA publishes a new major version instead of disguising the break as a patch.
