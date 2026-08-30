# Public KVM Backend implementation audit

- Date: 2026-08-30
- Audited range: `951f381...08e4d45`
- Audited head: `08e4d4526d877b71b23c6651d230729adbd39847`
- Status: changes required

## Outcome

The public KVM Backend is now a real executable lifecycle rather than an unavailable placeholder.
Resolve, Launch, Execute, Inspect, and Cleanup reach Linux KVM implementation code.
The guest establishes an authenticated control session and completes `prepare_and_probe` before the Backend reports Ready.

The implementation is not ready for production admission or performance claims.
Two identity violations are release blockers, and the current ownership, timeout, prepared-store, and cleanup behavior must be repaired before this path can be called trustworthy.

## P0.1 - Launch accepts an uncertified Candidate

### Evidence

`crates/soma-local/src/backend/kvm/prepared.rs` reads `candidate.somacan` and constructs a `PreparedGeneration` containing a `CandidateId`.
`crates/soma-local/src/backend/kvm/resolve.rs` explicitly reports no `GenerationId` because certification has not produced one.
`crates/soma-local/src/backend/kvm/lifecycle.rs` nevertheless launches that object and reports `DigestBinding::LaunchEnforced`.

This bypasses the Candidate -> certification -> Generation -> Launch boundary required by ADR 0026 and the SOMA mission.

### Required correction

Resolve must return a launchable object only after independent certification and promotion produced a verified `GenerationId`.
Launch must accept a certified installed Generation type that cannot be constructed from Candidate bytes alone.
The Candidate type must remain structurally impossible to pass into Launch.

### Acceptance gates

- A Candidate entry is rejected before any overlay, VM, vCPU, guest thread, or network resource exists.
- A promoted Generation with a valid certification chain resolves and launches.
- Altering Candidate bytes, certification bytes, the Generation ID, or any bound artifact causes fail-closed rejection.
- The public receipt reports the verified `GenerationId`, not a Candidate digest and not an unavailable identity.

## P0.2 - Authenticated guest identity differs from the public Instance identity

### Evidence

`crates/soma-local/src/backend/kvm/boot.rs::boot_for` receives the public `InstanceId` but uses it only to name the writable head.
The launch page receives unrelated random bytes from `fresh16()` as its guest Instance identity.
`crates/soma-local/src/backend/kvm/lifecycle.rs` then reports the caller's `InstanceId` in `LaunchObservation`.

The authenticated guest session therefore proves one identity while the public receipt describes another.

### Required correction

Define one canonical checked conversion from the public `InstanceId` to the exact 16 bytes carried by the launch page and authenticated transcript.
The guest session, launch material, Backend ownership record, Execute requests, Inspect results, Cleanup requests, and final receipt must all bind the same identity.

### Acceptance gates

- The Instance bytes authenticated by the guest equal the exact public Instance identity.
- Changing the public Instance ID invalidates the session and its evidence.
- A receipt from Instance A cannot complete Launch, Execute, Inspect, or Cleanup for Instance B.
- The retained live test prints or otherwise proves the same identity at the public request, launch page, authenticated session, and receipt boundaries.

## P1.1 - A second Launch silently replaces the live sandbox

### Evidence

`crates/soma-local/src/backend/kvm/lifecycle.rs` assigns `self.live = Some(...)` without first rejecting occupied state.
Replacing the value drops the previous `Session`, whose `Drop` implementation shuts down and joins the previous sandbox thread.

Launching Instance B can therefore terminate Instance A without an explicit Stop or Cleanup operation.

### Required correction

For the current single-Instance Backend, Launch must reject while any live Instance is owned.
If multi-Instance ownership is introduced later, it must use an explicit keyed ownership table with independent cleanup and bounded capacity admission.

### Acceptance gates

- Launch B while A is live fails without mutating A.
- A remains executable and inspectable after the rejected Launch B.
- Only Cleanup A releases A.
- Replayed Launch A follows the public idempotency contract and does not create or replace a machine.

## P1.2 - Execute timeout leaves a desynchronized live session

### Evidence

`crates/soma-local/src/backend/kvm/session.rs` returns `SessionError::Gone` when `recv_timeout` expires but retains the worker, machine, and response receiver.
`crates/soma-local/src/backend/kvm/lifecycle.rs` also retains `self.live` after that failure.

A late response from command A can subsequently be consumed as the response for command B.

### Required correction

Any response timeout or uncertain protocol outcome must permanently poison the session, stop accepting operations, terminate and join the sandbox, and preserve cleanup evidence.
No response arriving after an uncertain boundary may be attributed to another operation.

### Acceptance gates

- A command deliberately completing after the host timeout cannot satisfy a later Execute.
- The timed-out Instance transitions out of Ready and refuses further Execute calls.
- The sandbox thread and VM are reclaimed within a bounded deadline.
- Cleanup evidence distinguishes graceful shutdown, forced reclamation, and incomplete cleanup.

## P1.3 - Prepared-store admission is mutable and nondeterministic

### Evidence

`crates/soma-local/src/backend/kvm/prepared.rs` scans directory entries and accepts the first entry whose mutable reference text matches the request.
It decodes Candidate bytes and checks that `store` is a directory, but it does not prove certification, uniqueness, directory ownership, symlink safety, stable file identity, current compatibility, or complete artifact validity before admission.

Two entries for the same reference can resolve differently according to filesystem directory order.

### Required correction

Address installed Generations by immutable `GenerationId` or an atomic reference-to-Generation index whose target is unique and verified.
Open trusted roots and entries descriptor-relatively without following symlinks.
Verify certification, compatibility, manifest identity, and every required artifact before returning a prepared launch capability.

### Acceptance gates

- Duplicate reference mappings fail as ambiguous rather than selecting the first entry.
- Symlinked roots, entries, manifests, and stores fail closed.
- Mutation or replacement between Resolve and Launch is detected.
- Every opened artifact matches the certified descriptor before any VM is created.
- A tag update cannot silently change what an already-resolved request launches.

## P1.4 - Private-head cleanup is asserted rather than proved

### Evidence

The reflink fast path in `crates/soma-local/src/backend/kvm/boot.rs` uses the descriptor-relative exclusive primitive from `soma-storage`, which improves the original direct symlink-truncation defect.
However, successful reflink creation ignores unlink failure and still returns the head.
The copy fallback leaves a partially copied named file if `std::io::copy` fails, and successful removal is also ignored.
Cleanup later reports storage `Complete` because the sandbox thread ended, without evidence that no named head remains.

The copy fallback also converts the open directory descriptor back into `/proc/self/fd/...` path text instead of preserving descriptor-relative creation and deletion.

### Required correction

Keep creation, copy, and removal descriptor-relative through `openat2` or `openat` plus `unlinkat`.
Require successful unlink before Launch can proceed, or retain an explicit owned cleanup record that reconciliation must clear.
Every failure after destination creation must remove or durably record the destination before returning.

### Acceptance gates

- Forced copy failure leaves no named or unowned head.
- Forced unlink failure prevents Launch or produces explicit incomplete cleanup evidence.
- Parent-directory rename and replacement cannot redirect creation or cleanup.
- Cleanup reports storage `Complete` only after descriptor and namespace evidence proves no owned head remains.

## P1.5 - The public path is not the accepted production fast path

### Evidence

The current lifecycle reports `PreparationClass::OnDemand`, creates `SandboxMachine` inside an in-process thread, cold-boots the guest, creates a writable head during Launch, and installs fixed link-down networking.
It does not consume a certified prepared worker, restore a prepared snapshot, obtain host admission, transfer a storage lease, activate a network bundle, execute through a jailed native `soma-vmm` process, or compose `soma-hostd` and `soma-netd`.

This is a useful development lifecycle, but it is not the production transaction described by the KVM integration design.

### Required correction

Label this path explicitly as cold-boot development behavior until the production composition exists.
Do not present its latency as prepared restore latency.
Build the production path around certified Generations, prepared snapshots and heads, admission, owned resource leases, the jailed VMM process, authenticated repair, network activation, durable operation identity, and proven cleanup.

### Acceptance gates

- Public evidence distinguishes cold boot from prepared snapshot restore.
- The production Backend uses one native jailed `soma-vmm` process per Machine.
- Launch consumes prepared memory, storage, and network resources rather than constructing them in the timed path.
- Ready follows authenticated repair and a successful fixed probe for the same public Instance.
- The terminal receipt binds allocation, Generation, Instance, isolation, execution, and cleanup evidence.

## P1.6 - Inspect and Cleanup infer state they have not proved

### Evidence

Inspect reports Ready whenever an in-memory `self.live` entry exists.
It does not check worker health, authenticated-session health, network state, resource ownership, or durable lifecycle state.
Cleanup for an unknown Instance reports all non-network resources `Complete` despite having no ownership record proving that those resources never existed or were already released.

### Required correction

Inspect and Cleanup must reconstruct truth from explicit ownership and operation records.
Unknown, absent, already-cleaned, and incompletely-cleaned Instances must remain distinguishable.
Cleanup dispositions must be supported independently by machine, memory, storage, network, authority, and process evidence.

### Acceptance gates

- A dead worker cannot be reported Ready merely because an in-memory slot remains.
- Cleanup of an unknown Instance does not fabricate completed ownership evidence.
- Restart reconstruction reaches the same state as uninterrupted execution.
- Repeated Cleanup continues incomplete work and returns stable terminal evidence once complete.

## CI and evidence blockers

At audited head `08e4d45`, the Security workflow failed while CI and KVM smoke were still running.
The preceding public-lifecycle commit failed Ubuntu 24.04, Windows, macOS, and Security checks.
Ubuntu 24.04 failed because a non-ignored scratch-management test demanded 8 GiB while the runner had approximately 7.34 GiB free.
Windows and macOS failed warnings-as-errors checks.
The architecture checker still uses GNU `find -printf`, which fails on macOS while the script reports success.

The following earlier audit blockers also remain open and must not disappear from the repair queue:

- Snapshot readiness can still be minted from caller-constructed session evidence.
- Network activation proves claimant continuity, not authenticated guest repair.
- Network release can report completion before durable ledger release commits.
- Network assignment ownership is UID-bound rather than exact-operation-bound.
- The Generation test cache omits material compiler inputs and does not fully verify cache hits.
- The Isorun collector and report generator still disagree about success semantics.

## Required repair order

1. Enforce certified Generation admission.
2. Bind public and guest Instance identity.
3. Reject replacement Launch and poison timed-out sessions.
4. Make prepared-store resolution unique, immutable, and fully verified.
5. Make head lifecycle descriptor-relative and cleanup-provable.
6. Make Inspect, Cleanup, and operation replay evidence-backed.
7. Preserve this path as an explicitly labeled cold-boot development lifecycle.
8. Compose the production prepared-restore lifecycle.
9. Correct stale README and guide statements.
10. Fix every required CI, security, portability, and evidence gate before publishing performance claims.

