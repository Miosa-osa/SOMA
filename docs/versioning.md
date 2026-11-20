# SOMA versioning and releases

## The three versions

SOMA has three kinds of version, and each has one owner in the code.

| Version | Owner | Definition |
| --- | --- | --- |
| Release | `VERSION`, `[workspace.package]` in `Cargo.toml`, read in code as `soma_version::RELEASE` | The Semantic Versioning number of one tagged source tree, shared by every crate and every binary built from it. |
| API | `soma_api::API_VERSION` | The `MAJOR.MINOR` of the HTTP surface `soma-api` serves under `/v1`, which every response names and a client may require. |
| Protocol | one constant per protocol, in the crate that owns its encoding | The version of one wire protocol between two SOMA processes, which every binary that speaks it lists. |

The release version says which source a binary came from, the API version says what a client of the service may rely on, and the protocol versions say which processes can talk to each other.
They move independently: a release can leave the API and every protocol unchanged, and a protocol can change without changing the API.
[The API surface](protocol/api-surface.md) records every API version and its compatibility rule.

## Version line

SOMA follows Semantic Versioning 2.0.0.
The first stable release will be `1.0.0`.
Development builds leading to that release use `1.0.0-alpha.N` and `1.0.0-rc.N` versions so an unfinished sandbox cannot be confused with the stable release.
The current source version is recorded in both `VERSION` and the Cargo workspace package metadata.

Before `1.0.0`:

- `1.0.0-alpha.N` names a tagged development release while any admission gate below is still open, and each new tag increments `N`.
- `1.0.0-rc.N` names a release candidate once every gate has evidence at one revision, and only fixes land between candidates.
- `1.0.0` is a candidate that needed no further fix.

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
| `guest-api` | the host runtime and the guest agent | `soma_guest::GUEST_API_LEVEL` | the agent's answer to the level probe of ADR 0051 | Either side may be newer: a host sends a level 2 request only to a guest that answered at least 2. |
| `launch-page` | the host runtime and the guest agent | `soma_guest::LAUNCH_PAGE_SCHEMA_VERSION` | the launch page itself | Exact: the page layout is part of what a Generation is certified with. |
| `hostd` | the local runtime and `soma-hostd` | `soma_hostd::protocol::PROTOCOL_VERSION` | not carried | Lockstep: a host runs the daemon and its clients from one release, and an unknown frame is answered with the `Protocol` failure code. |
| `netd` | the local runtime or `soma-hostd`, and `soma-netd` | `soma_netd::PROTOCOL_VERSION` | not carried | Lockstep, as for `hostd`. |
| `vmm-control` | the jail supervisor and the `soma-vmm` worker | `soma_vmm::control::CONTROL_PROTOCOL_VERSION` | not carried | Lockstep: the worker is executed from the installed release for each launch. |
| `runner` | the public runner in `soma-api` and the control plane | `soma_api::runner::control_plane::RUNNER_PROTOCOL_VERSION` | the `soma-runner-protocol` header of every feed and journal request | The control plane may be newer and must keep serving every version still installed on a host; a runner skips and logs a feed event it does not know, so new event kinds are additive. |

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

A release publishes these files, all built from the tagged commit:

| File | What it is |
| --- | --- |
| `soma-vX.Y.Z-source.tar.gz` | The committed tree, from `git archive`, byte-identical for one commit. |
| `soma-host-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` | `soma-api`, `soma-hostd`, `soma-netd`, `soma-vmm`, and the static `soma-guest-agent`. |
| `soma-vX.Y.Z-TARGET.tar.gz` | The `soma` command line and `soma-mcp` for Linux x86_64, macOS arm64, and Windows x86_64. |
| `soma-vX.Y.Z-manifest.json` | The release manifest described below. |
| `SHA256SUMS` | The SHA-256 of every other file. |

Each bundle also holds `BUILD-INFO.txt` with the tag, target, commit, and toolchain, the license files, and its own `SHA256SUMS`.
The workspace crates are not published to a registry: the public crates depend on crates that are not published, and the source archive is what ties a release to its source.

## Cutting a release

A release is cut from `main` in four steps.

1. Move the version in a pull request: `scripts/set-version.sh 1.0.0-alpha.2` rewrites `VERSION`, the workspace version, every requirement between workspace crates, and `Cargo.lock`, then runs the version contract.
2. Prove the release before tagging. On Ubuntu 24.04 x86_64, `SOMA_RELEASE_TAG=v1.0.0-alpha.2 scripts/check.sh release` runs the Linux validation, builds the source part, the host bundle, and the Linux client bundle, assembles them with the manifest, and verifies the result as a deployer would. The `Release bundle` workflow run by hand on the branch does the same for every target and publishes nothing.
3. Tag the merged commit with an annotated tag and push the tag:

   ```sh
   git switch main && git pull --ff-only
   git tag -a v1.0.0-alpha.2 -m "SOMA v1.0.0-alpha.2"
   git push origin v1.0.0-alpha.2
   ```

4. The tag runs the `Release bundle` workflow, which checks that the tag names the commit it built, builds every target, assembles and verifies the release, and creates a draft GitHub release with generated notes, marked as a pre-release when the version has a pre-release part. Publish the draft after reading it: `gh release edit v1.0.0-alpha.2 --draft=false`.

Anyone can verify a downloaded release with the same verifier the workflow uses, from a checkout of the tag:

```sh
scripts/verify-release-artifacts.sh release DOWNLOAD_DIR v1.0.0-alpha.2
```

## Release manifest

`soma-vX.Y.Z-manifest.json` is the one machine-readable record of a release, schema `soma.release-manifest.v1`.
It is written by `scripts/assemble-release.sh` from the binaries' own `--version` reports, so it cannot claim a commit or a protocol a binary does not report.

```json
{
  "schema": "soma.release-manifest.v1",
  "release": "1.0.0-alpha.2",
  "tag": "v1.0.0-alpha.2",
  "prerelease": true,
  "commit": "2ec3789c4f1a8e9d0b1c2d3e4f5a6b7c8d9e0f1a",
  "commit_date": "2026-11-20T18:04:11Z",
  "rust": "1.98.0",
  "api_version": "1.0",
  "artifacts": [
    { "file": "soma-host-v1.0.0-alpha.2-x86_64-unknown-linux-gnu.tar.gz", "sha256": "...", "size": 12345678 }
  ],
  "binaries": [
    {
      "name": "soma-api",
      "archive": "soma-host-v1.0.0-alpha.2-x86_64-unknown-linux-gnu.tar.gz",
      "path": "soma-host-v1.0.0-alpha.2-x86_64-unknown-linux-gnu/soma-api",
      "sha256": "...",
      "size": 14658384,
      "release": "1.0.0-alpha.2",
      "commit": "2ec3789c4f1a8e9d0b1c2d3e4f5a6b7c8d9e0f1a",
      "reports_commit": true,
      "source": "clean",
      "profile": "release",
      "target": "x86_64-unknown-linux-gnu",
      "api_version": "1.0",
      "protocols": { "guest-application": 1, "guest-api": 2, "launch-page": 3, "hostd": 1, "netd": 1, "runner": 1 }
    }
  ]
}
```

`artifacts` lists every published file except the manifest and `SHA256SUMS`, and `binaries` lists every binary inside every bundle.
`reports_commit` is `false` only for the guest agent, which records no commit; its `commit` is still the release commit it was built from.

A deployer pins from the manifest instead of a bare checksum:

1. Choose the binary by `name` and `target`, and keep its `sha256`, `size`, `release`, `commit`, `api_version`, and `protocols` beside the pin.
2. Download `SHA256SUMS`, the manifest, and the named `archive`, and check the archive against both.
3. Extract `path` from the archive and refuse it unless its SHA-256 and byte size equal the pinned ones.
4. After installing, read `soma-api --version` or `GET /v1/version` on the host and refuse the host unless it reports the pinned `release` and `commit`.

To rebuild a released binary and compare it with its record, check out the commit and run the packager itself, for example `SOMA_RELEASE_TAG=v1.0.0-alpha.2 scripts/package-host-release.sh`, then compare the new `.binaries.json` with the manifest.
The packager is the reference build on purpose: Cargo unifies the features of every package in one invocation, so which packages are built together is part of the input, and the packager remaps the checkout and the Cargo home out of every path a binary embeds.
The guest agent is built by `scripts/build-guest-agent.sh`, which does not remap paths yet, so its bytes still depend on the directory it was built in.

## Reading the version of a running host

The binary on disk answers `--version`, and a running `soma-api` answers `GET /v1/version` with the same facts plus the API version.

```sh
soma-api --version
curl -s -H 'x-soma-tenant: operator' http://127.0.0.1:8787/v1/version
```

Every response of the service also names its API version in `soma-api-version`, so a client learns it from any call.

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
api: 1.0
protocol.guest-application: 1
protocol.guest-api: 2
protocol.launch-page: 3
protocol.hostd: 1
protocol.netd: 1
protocol.runner: 1
```

The binaries are `soma`, `soma-mcp`, `soma-api`, `soma-hostd`, `soma-netd`, `soma-vmm`, and `soma-guest-agent`.
The first line is for people and the `key: value` lines after it are a stable contract, which `soma_version::ParsedReport` reads back.
`api` appears only for `soma-api`, and each binary lists only the protocols it speaks.
A running `soma-api` reports the same facts at `GET /v1/version`.
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
