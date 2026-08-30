# Implementation re-audit

- Date: 2026-08-29
- Repository: `Miosa-osa/SOMA`
- Previous reviewed implementation commit: `4879517`
- Re-audited implementation commit: `d790555`
- Fixed review range: `4879517...d790555`
- Review status: Action required
- Audience: Agent continuing the Linux KVM production path

## Executive judgment

The new work is substantial and materially stronger than the implementation reviewed previously.
The earlier Priority 0 and Priority 1 implementation defects were mostly corrected in code and tests rather than hidden by documentation.
The modules remain reasonably focused despite the large increase in scope.

The production sandbox path is not complete.
The new privileged network broker introduces two Priority 0 security defects.
Snapshot readiness, privileged tool supervision, protocol delivery, and the portable validation gate introduce Priority 1 correctness defects.
The retained snapshot evidence and several architecture documents still describe obsolete responder-key behavior and cannot certify the current bytes.

Do not treat networking, restore readiness, or the integrated Host path as production-capable until the blockers below are corrected and proved end to end.

## Review method

The review used two separate axes.

- The Standards axis checked the implementation against repository engineering, security, lifecycle, evidence, and modularity standards.
- The Spec axis checked the implementation against the mission, roadmap, ADRs, decision-map tickets, and stated production-sandbox behavior.

The portable repository gate was also run from the repository root with `./scripts/check.sh portable`.
It failed in the benchmark harness before reaching the remaining Rust checks.

## Confirmed fixes from the previous audit

These corrections are material and should be preserved.

- Fresh per-Instance responder authority now crosses launch-page schema 3 rather than living in reusable Generation artifacts.
- Guest stdout and stderr share one bounded allowance and process-group termination path.
- Generation build tools use isolated process groups, deadlines, and bounded capture.
- Candidate construction and certified Generation publication are separated.
- Hostile Generation compatibility fields receive materially broader validation.
- Kernel entropy credit distinguishes trusted entropy from merely mixed seed material.
- Handwritten Linux network ABI layouts are gated to x86_64.
- IPv4 network and broadcast addresses are rejected.
- Material build tools are pinned, opened, verified, and executed through the sealed-tool path.
- Structured workload commands remove the earlier implicit shell and argument ambiguity.

These closures are component-level evidence.
They do not by themselves prove an integrated production lifecycle.

## Standards findings

### P0.1 Bind network activation to authenticated guest evidence

#### Finding

`crates/soma-netd/src/activate.rs` exposes `RepairAttestation::authenticated` as a constructor whose documentation makes the caller responsible for truth.
`crates/soma-netd/src/daemon.rs` receives an `Activate` request and manufactures that attestation from the assigned Instance identifier.
No authenticated guest session, repair receipt, readiness receipt, nonce, or operation proof is consumed.

Any client able to issue the request can cause forwarding to be enabled without proving that guest repair completed.
The type name therefore claims an authority the value does not possess.

#### Required correction

- Remove the public assertion-style constructor.
- Make the guest-session owner produce a nonforgeable, single-use activation capability only after authenticated repair succeeds.
- Bind the capability to the Instance, assignment generation, operation, network intent, and freshness context.
- Consume it exactly once in the broker.
- Fail closed and release the assignment when authentication, binding, replay, or activation fails.

#### Acceptance gates

- A raw `Activate` request without an authenticated receipt cannot enable forwarding.
- A receipt from another Instance, generation, operation, or network intent is rejected.
- A replayed receipt is rejected.
- Forwarding remains disabled before the authenticated transition and after failed activation.

### P0.2 Authorize the privileged `soma-netd` control socket

#### Finding

`crates/soma-netd/src/daemon.rs` explicitly states that its Unix socket does not authenticate peers.
The accept loop admits every reachable local peer and permits Claim, Activate, Release, and Reconcile operations.
The daemon does not prove the peer identity or capability and does not explicitly establish the socket ownership and mode after binding.

A local process that can reach the socket can obtain TAP descriptors and mutate privileged network lifecycle state.

#### Required correction

- Place the listener in an explicitly owned directory with fail-closed permissions.
- Set and verify socket owner, group, and mode after binding.
- Authenticate every connection with a kernel-derived peer identity and an application capability appropriate to the operation.
- Bind transferred descriptors and replies to the authenticated request identity.
- Reject unauthorized peers before decoding or mutating lifecycle state.

#### Acceptance gates

- An unauthorized local process cannot connect successfully or obtain a descriptor.
- A permitted process without the correct operation capability cannot claim, activate, release, or reconcile.
- Restart, stale-socket, ownership-drift, and permission-drift tests fail closed.
- The production launcher proves the exact identity and capability handoff used by `soma-hostd` and the jailed VMM.

### P1.1 Require authenticated readiness evidence after restore

#### Finding

`crates/soma-kvm/src/x86_64/snapshot/restore.rs::Restored::ready` accepts no proof.
Any caller can invoke it immediately after resume and advance the typestate to ready.
The method comment says authenticated repair and readiness completed, but the API receives no authenticated terminal evidence.

#### Required correction

- Replace the assertion method with a transition that consumes a validated guest-session readiness receipt.
- Bind the receipt to the fresh Instance, restored snapshot, launch authority, operation, and session transcript.
- Keep execution and network activation unavailable until that transition succeeds.

### P1.2 Bound and supervise privileged networking tools

#### Finding

`crates/soma-netd/src/nft.rs` executes `nft` and `conntrack` with blocking waits and no deadline.
Table listing captures stdout without a bound.
Ruleset input write failure is discarded.
The single-threaded broker can therefore hang permanently or allocate unbounded output while cleanup becomes unavailable.

#### Required correction

- Reuse or extract the repository's process-group, absolute-deadline, bounded-capture supervision primitive.
- Treat input write failure as operation failure.
- Terminate the complete process group on timeout, cancellation, capture overflow, or protocol error.
- Bound termination grace, drain, wait, output, and retained diagnostics.
- Make cleanup recoverable even when a tool invocation wedges.

### P1.3 Require complete network protocol delivery

#### Finding

`crates/soma-netd/src/daemon.rs` treats every nonnegative `send` result as success.
It does not require `sent == bytes.len()`.
An incomplete delivery can leave ownership and replay semantics ambiguous while the daemon continues processing requests.

#### Required correction

- Treat any short send as a terminal protocol failure.
- Define whether the associated lifecycle mutation commits before or after reply delivery.
- Make disconnect and uncertain-delivery recovery idempotent through operation identities and ledger reconciliation.
- Add resource-pressure and forced-disconnect tests around descriptor transfer and ordinary replies.

### P1.4 Repair the portable benchmark test gate

#### Finding

`./scripts/check.sh portable` fails while discovering five new burst-harness test modules.
The command loads files from `benchmarks/tests` as top-level modules, while these files use relative imports from `.burst_fixtures`:

- `test_burst_plan.py`
- `test_burst_report.py`
- `test_burst_results.py`
- `test_burst_run.py`
- `test_burst_slot.py`

All five fail with `ImportError: attempted relative import with no known parent package`.
The gate exits before the remaining Rust workspace checks, so the current commit does not have a passing portable validation result.

#### Required correction

- Make the test package and discovery command agree on one import model.
- Exercise the exact `./scripts/check.sh portable` command locally and in CI.
- Do not weaken discovery or omit the burst tests.

## Spec and evidence findings

### P0.3 Resolve the duplicate and incompatible ADR 0024 decisions

#### Finding

Two accepted ADR files use number 0024.
`0024-pre-launch-snapshot-capture-point.md` says a Generation-scoped responder private key remains in captured memory.
`0024-per-instance-guest-responder-authority.md` supersedes that design and requires fresh per-Instance launch-page authority.

The repository therefore has two accepted decisions with the same identity and incompatible security contracts.

#### Required correction

- Preserve historical truth, but give the decisions unique identifiers.
- Mark the obsolete responder-key provisions as superseded with a direct link to the active decision.
- Update every ambiguous `ADR 0024` reference to the exact active decision.
- Run a repository-wide consistency check for the obsolete Generation-scoped secret model.

### P1.5 Regenerate snapshot evidence for the current authority design

#### Finding

`docs/evidence/2026-08-29-x86_64-snapshot-restore.md` records and scans a Generation-scoped responder private key in `memory.raw`.
The current implementation removed that key from reusable artifacts and memory.
The retained artifact identities, scans, and timings therefore describe an older architecture.

The evidence can remain as historical evidence if labeled precisely, but it cannot certify the current implementation.

#### Required correction

- Rerun capture and repeated restore with the current commit and fresh per-Instance authority.
- Retain exact source, toolchain, kernel, Generation, snapshot, host, configuration, and artifact identities.
- Prove absence of every reusable private authority from the Generation and snapshot.
- Prove unique authority, identity, context identifier, writable head, and authenticated transcript across restores.
- Replace current performance statements only with measurements from the new coherent run.

### P1.6 Reconcile contradictory decision-map and guide status

#### Finding

Ticket 7 in `docs/research/vmm-decision-map.md` says a real Node 22 Generation was captured and restored repeatedly.
Ticket 8 says the restored repair path remains unproven because no snapshot has been captured.
Several guides and module descriptions repeat the old no-snapshot and Generation-secret status.

Readers cannot determine what is implemented, what was proved only on an obsolete revision, and what remains unproved.

#### Required correction

- Use one status vocabulary: designed, component-tested, live-proved, integrated, and production-admitted.
- Record the exact commit and evidence artifact for every live-proved statement.
- Mark obsolete runs as historical rather than silently rewriting their observations.
- Update the decision map, module map, how-it-works guide, Generation research, and Template guide together.

## Incomplete production gates

The following are incomplete capabilities, not necessarily implementation defects.
They must remain labeled prototypes until their gates pass.

### Ticket 5 and Phase 2: real guest networking

The machine path still uses a link-down loopback backend rather than a production virtio-net attachment.
Restore-hostile behavior, forced cleanup, and latency evidence remain open.

### Ticket 6 and Phase 5: Generation certification

`certify_candidate` still fails closed as unimplemented.
There is no complete boot, capture, certification, signed-manifest, SBOM, revocation, and registry-publication pipeline.

### Ticket 9 and Phase 4: real VMM jail

The retained jail proof constrains `jail-probe`, not the production `soma-vmm` process with its real descriptors, threads, startup syscalls, and cleanup behavior.

### Ticket 10: production connectivity

Proxy attachment, ingress forwarding, daemon authorization, jailed TAP transfer, and VMM virtio-net integration remain open.

### Ticket 11: production writable storage

The XFS and reflink work is useful component evidence.
The production launch path does not yet consume prepared private overlay heads end to end.

### Ticket 12: Host composition

`soma-hostd` uses an explicitly development-only in-process launcher.
No real VMM, jail, storage, network, authenticated guest, cleanup, and allocator composition has passed the required concurrent proof.

### Ticket 13: public KVM Backend

The public Backend lifecycle remains unavailable over the KVM implementation.

### Ticket 14 and Phase 6: accepted performance evidence

The burst harness dry run is Docker harness evidence only.
There is no admitted KVM cohort, signed report, 100 engineering bursts, 10,000 samples, or accepted 10 ms result.

## Required repair order

1. Fix the broken portable benchmark test gate.
2. Remove forgeable network activation and authorize the privileged socket before doing more networking work.
3. Add deadline-bounded tool supervision and unambiguous delivery semantics to `soma-netd`.
4. Bind restore readiness to authenticated guest evidence.
5. Resolve duplicate ADR numbering and obsolete responder-key documentation.
6. Rerun snapshot evidence on the current authority design.
7. Integrate one real KVM Instance through `soma-hostd`, real `soma-vmm`, jail, prepared storage, TAP networking, guest authentication, execution, shutdown, and complete cleanup.
8. Expose that exact lifecycle through the public Backend.
9. Run concurrency, hostile-failure, recovery, and resource-bound gates.
10. Only then run and publish the admitted KVM performance campaign.

Do not add another major subsystem before steps 1 through 6 are complete.

## Completion definition for the next audit

The next audit should receive:

- One passing `./scripts/check.sh portable` transcript.
- Linux KVM CI and retained live-host evidence for the affected path.
- Negative authorization and replay evidence for `soma-netd`.
- Deadline, output-bound, process-tree, and recovery evidence for every privileged subprocess.
- A current snapshot artifact whose authority model matches current code and ADRs.
- One end-to-end Instance receipt covering allocation through proven cleanup.
- A claim ledger that distinguishes prototype, component-tested, live-proved, integrated, and production-admitted capabilities.

Until those artifacts exist, the correct public statement is that SOMA has strong component foundations and live KVM proofs, but not yet one production-integrated sandbox lifecycle or admitted 10 ms performance result.
