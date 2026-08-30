# KVM lifecycle and server setup full re-audit

- Date: 2026-08-30
- Audited range: `08e4d45...50cd82e`
- Status: immediate setup, publication, portability, and CI repairs implemented; production gaps remain
- Scope: public KVM lifecycle, prepared-store hardening, Generation preparation, server bootstrap, retained evidence, and CI

## Outcome

The work is not empty, but it is not complete and several new claims exceed the evidence.
Public-to-guest Instance identity, second-Launch rejection, unknown-Instance cleanup reporting, duplicate prepared references, bounded prepared-file reads, and the portable ADR checker were materially improved.

The current path remains a one-at-a-time, in-process, uncertified, cold-boot development sandbox.
It is not the certified, jailed, prepared-restore production sandbox required by the SOMA mission.

At audited head `50cd82e`, the new empty-server setup was not executable as documented on an ordinary fresh Ubuntu host.
It had no retained end-to-end evidence, and both GitHub workflows were red.

## Remediation completed after the audit

The same repair series corrected the immediate operational and CI failures identified below.
The historical findings remain in this document so another agent can see what was found and why each repair exists.

- `setup-host.sh` now runs from an obtained repository, provisions the complete `/srv/soma` development layout, checks required host capabilities, and exits nonzero when a required check fails.
- The server runbook now uses the built binary path, states the actual doctor boundary, distinguishes the in-process development KVM backend from the future jailed VMM, and avoids mutable Node patch-version claims.
- Filesystem tooling now builds the pinned e2fsprogs source rather than symlinking mutable host executables, and forces Linux amd64 so Apple Silicon Docker hosts cannot emit the wrong server architecture.
- OCI references are normalized without corrupting qualified registries, and prepared-entry names use the SHA-256 of the exact input reference.
- Candidate publication now builds under a private sibling staging directory, refuses replacement, fsyncs its publication boundary, and uses an atomic no-replace rename on Linux.
- Shell contract tests cover reference normalization and collision resistance.
- Linux ARM64, Intel macOS, and Windows ARM64 workspace compilation now passes through explicit target gates instead of unreachable match tricks or leaked Unix-only test code.
- Portable host-daemon tests no longer confuse macOS descriptor limits or filesystem jitter with product failures.
- The exact spelling checker now passes with kernel configurations and raw retained evidence excluded as immutable data and technical vocabulary explicitly documented.
- Linux amd64 container validation passes the atomic publication example tests and a locked all-target workspace check.

These repairs do not close certified Generation admission, descriptor-pinned prepared identity, bounded vCPU reclamation, jailed VMM composition, prepared restore, or retained live KVM evidence.
Those remain production blockers.

## Standards

### P0 - Uncertified Candidate remains launchable

`crates/soma-local/src/backend/kvm/prepared.rs` refuses Candidates by default, which is a useful mitigation.
Setting `SOMA_ALLOW_UNCERTIFIED_GENERATION=1` still returns a Candidate-shaped `PreparedGeneration` to Resolve and Launch.

No certification chain, certified installed Generation type, verified `GenerationId`, compatibility proof, or artifact digest proof exists at the launch boundary.
This leaves the original P0.1 structurally open.

### P0 - Prepared-store identity remains mutable

The prepared-store implementation still checks pathnames and reopens those pathnames later.
Symlink checks, bounded reads, and deterministic duplicate handling do not prevent replacement between check and use.

`PreparedGeneration` retains a `PathBuf`, and Launch resolves artifacts again from that path.
The correction requires descriptor-relative no-follow opening from a trusted root and stable verified object identity retained through Launch.

The current `claims` function converts an oversized, unreadable, or invalid reference file into a non-claim.
A damaged second claimant can therefore disappear from ambiguity detection instead of failing the scan closed.

### P0 - Generation publication is destructive and non-atomic

`crates/soma-generation/examples/prepare_generation.rs` deletes an existing entry before compilation and builds directly inside the final destination.
It publishes `candidate.somacan` and `reference` after writing the store, without an atomic directory promotion or manifest-last transaction.

A failed build can destroy the previous usable entry or expose a partial entry to concurrent Resolve.
Staging cleanup failure is ignored, and no rollback, crash, replacement, or concurrent-reader test protects this new seam.

### P1 - Timeout cleanup is not bounded

Execute timeout now poisons the session, which prevents a late response from satisfying a later command.
The timeout path then calls blocking `JoinHandle::join()` without a bounded vCPU interruption and reclamation deadline.

A sandbox thread stuck in guest execution can therefore make an already-timed-out Execute block indefinitely.
P1.2 remains partial until forced interruption, bounded join, terminal state, and cleanup disposition are proven.

### P1 - Writable-head ownership is still pathname-based

The private-head fallback converts an open directory descriptor into `/proc/self/fd/...` pathname text and performs path-based creation and deletion.
Failed cleanup after a failed copy can still be ignored without a durable reconciliation record.

P1.4 remains open until creation and removal stay descriptor-relative and cleanup evidence proves the named head is absent.

### P1 - Supply-chain inputs are not fully pinned

`scripts/setup-host.sh` executes the current rustup installer through `curl | sh`.
`scripts/build-fs-tools.sh` installs mutable package-index contents inside the builder and accepts mutable host e2fsprogs tools by version text before symlinking them into the tool directory.

The builder image and erofs commit are pinned, which is good, but the complete executing-tool identity and provenance chain are not.

### P1 - Required CI is red

At audited head `50cd82e`, CI run `33309425973` failed and security run `33309425977` failed.
The earlier compatibility run reproduced Linux cross-target exhaustiveness and unreachable-pattern failures in `crates/soma-local/src/backend/mod.rs`.
The security job is blocked by spell-check handling of legitimate technical names such as `mke2fs`.

Local `cargo fmt --all -- --check`, shell syntax, ShellCheck, and the architecture checker pass on macOS.
Local `cargo test -p soma-local --all-features` passes 23 tests, but Linux-only KVM prepared-store tests do not execute on macOS.
Local `cargo test -p soma-generation --all-features` fails 9 of 75 tests on macOS in descriptor-pinned process control.

## Spec

### Fixed

- The public Instance identity is converted to the exact guest launch-page bytes.
- A second Launch is rejected instead of silently replacing the live sandbox.
- Unknown Cleanup reports resources as `NotOwned` instead of asserting completed cleanup.
- Candidate digest binding is reported as `ObservedOnly` instead of `LaunchEnforced`.
- Duplicate reference mappings fail as ambiguous.
- Prepared reference and Candidate reads are bounded.
- Directory-iteration errors are no longer discarded with `flatten()`.
- Direct linked roots and entries receive explicit rejection tests.
- The architecture checker no longer depends on GNU `find -printf`.

### Partial

- Default Candidate refusal occurs before decode, but a process-wide environment escape hatch still permits Launch.
- Session poisoning prevents response reuse, but forced reclamation is unbounded.
- More symlink cases are detected, but stable descriptor-pinned identity is absent.
- Private-head unlink errors are handled more often, but creation and cleanup remain pathname-based.
- Public and guest identity code agrees, but no retained live run at the current commit proves the request, launch page, transcript, receipt, and cleanup chain.

### Open

- Certified Generation admission and immutable `GenerationId` receipts.
- Artifact compatibility and digest verification before VM creation.
- One jailed native `soma-vmm` process per Machine.
- Prepared snapshot restore on the public Backend.
- Host admission, prepared memory, storage, and network leases.
- Real guest networking through `soma-netd`.
- Durable operation identity and lifecycle reconstruction across restart.
- Multi-Instance ownership and bounded host capacity.
- Concurrent burst evidence and admitted performance distributions.
- Independent cleanup proof for process, memory, storage, network, and authority.

## Empty-server setup failures

### The documented order is impossible

The runbook tells the operator to execute `./scripts/setup-host.sh` before it tells them to clone or transfer the repository containing that script.
The repository must exist before any repository script can run.

### `/srv/soma` is never provisioned

The runbook writes filesystem tools, prepared entries, and writable heads below `/srv/soma`.
No script creates that root with ownership for the invoking user.

An ordinary user on fresh Ubuntu cannot create `/srv/soma/fs-tools`, so the documented Step 3 fails before a sandbox exists.

### Host setup reports failure but exits successfully

`scripts/setup-host.sh` prints failed checks for KVM, CPU virtualization, Docker, Cargo, musl, and XFS but never exits nonzero from `report`.
An agent can therefore interpret a failed host as successfully prepared and continue into confusing downstream failures.

Reflink absence should be a warning, while required runtime failures must produce a nonzero exit.

### Strict doctor does not check the listed prerequisites

The runbook says `soma doctor --strict` checks cgroup v2, namespaces, seccomp, networking, writable-root cleanup, stable timing, and capacity.
The current Linux KVM doctor performs the KVM API probe and treats `ProbePassed` as success even though `production_ready` is false.

It does not check most prerequisites listed immediately above the command.

### The command path is wrong

`scripts/build-soma.sh` builds `target/release/soma` but does not install it into `PATH`.
The runbook invokes `soma doctor --strict`, which will fail on a clean host unless the operator manually changes `PATH` or uses `./target/release/soma`.

### The runbook does not set up `soma-vmm`

The opening architecture section says the document sets up `soma-vmm`.
The actual build creates the CLI, guest agent, and guest kernel, while the VM runs inside the CLI process.

The runbook later admits that a separately confined `soma-vmm` process does not exist on this path.
The opening claim must describe the development KVM backend instead.

### OCI reference handling is incomplete

`scripts/prepare-generation.sh` always prefixes input with `docker://docker.io/library/`.
That works for simple Docker Hub official images such as `node:22`, but breaks qualified references for other registries and Docker Hub namespaces.

Replacing `/` and `:` with `-` also permits distinct image references to collide on the same prepared-entry and temporary-layout name.
Publication must use parsed OCI identity and a collision-resistant immutable key.

### The exact Node version promise is mutable

The runbook promises `v22.23.2` while preparation pulls the mutable `node:22` tag.
That output was true for one retained historical run, but it is not a stable result of the current command.

The example should verify a Node 22 major version or pin an immutable OCI digest.

### No retained empty-host evidence exists

Commit text says the flow was verified end to end on a fresh host, but no evidence document records the host, exact commands, outputs, artifact digests, commit, failure behavior, or cleanup result.
The existing public KVM CLI evidence predates the identity, timeout, admission, storage, and setup changes.

The setup capability is therefore not Live-proved.

## Evidence and documentation corrections

`docs/evidence/2026-08-30-kvm-backend-cli-run.md` calls old `08e4d45` evidence current and records `launch_enforced`, while current code reports `ObservedOnly`.
It must be labeled historical or replaced by a fresh retained run at the current commit.

The claim ledger simultaneously says the public KVM path was Live-proved at `08e4d45` and that no public Backend KVM run exists.
The ledger must be reconciled and downgraded where current behavior lacks current evidence.

The README and guides still contain stale statements that the KVM Backend always returns unavailable.
Public documentation must distinguish the live cold-boot development path from the unimplemented production path.

## Required repair order

1. Make the setup flow executable on an ordinary fresh Ubuntu 24.04 x86_64 host and retain the complete run.
2. Make host preflight return truthful exit status and check only what it claims.
3. Make Generation preparation transactional, collision-resistant, recoverable, and safe for concurrent readers.
4. Replace pathname validation with descriptor-pinned immutable prepared admission.
5. Implement certified Generation promotion and remove Candidate acceptance from the production Launch type.
6. Bound timeout interruption, join, reclamation, and cleanup evidence.
7. Finish descriptor-relative writable-head ownership and reconciliation.
8. Reconcile the claim ledger, evidence documents, README, and operational guide with current code.
9. Restore green Linux, macOS, Windows, KVM-smoke, and security workflows.
10. Only then continue into the jailed prepared-restore production composition and performance admission.

## Production stop condition

Do not publish production readiness, 10 ms readiness, competitor superiority, or empty-server automation claims from this path yet.
The honest description is that SOMA has a live Linux KVM cold-boot development sandbox and substantial VMM components, with production admission and reliable server bootstrap still incomplete.
