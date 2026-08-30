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

## Road from the working VMM to production admission

SOMA now has a working cold-boot VMM path: the CLI can select the KVM Backend, create a VM, boot the guest, establish an authenticated control session, complete the fixed readiness probe, execute a bounded command, and clean up the in-process machine.
That is the machine floor and a real product milestone.
It is not yet the complete secure, scalable, prepared-restore sandbox described by the SOMA mission.

The stages below are cumulative.
A later stage is not complete when an earlier stage still has an open invariant.

### Stage 1 - Correct public sandbox identity and admission

The current lifecycle must first become semantically truthful.

Required work:

- Launch only installed certified Generations and make it structurally impossible to pass a Candidate into Launch.
- Bind the public Instance ID to the exact bytes in the launch page, authenticated guest transcript, ownership record, Execute request, Inspect result, Cleanup request, and terminal receipt.
- Reject a second Launch while the single-Instance Backend is occupied.
- Permanently poison and reclaim a session after any command timeout or uncertain protocol result.
- Resolve one unique immutable Generation instead of selecting the first mutable reference match.
- Report only observations proven by the implementation.

Definition of done:

- The same Instance identity is proved at every public, host, VMM, launch-page, guest-session, and receipt boundary.
- No Candidate can reach VM creation.
- No operation can replace, impersonate, or consume the response of another Instance or operation.
- Every uncertain response terminates the affected session before another operation is accepted.

### Stage 2 - Prove the cold-boot development lifecycle

The current cold-boot path should remain available as an explicitly named development and diagnostic profile.
It must be proven before it becomes a dependable fallback.

Required workload coverage:

- `node:22`
- Ubuntu
- Alpine
- BusyBox
- A representative coding agent workload
- A hostile or malformed workload fixture

Required behavioral coverage:

- Successful boot, authentication, readiness, Execute, shutdown, and cleanup
- Missing, damaged, incompatible, uncertified, and replaced Generation artifacts
- Guest boot failure and readiness failure
- Missing executable and command invocation failure
- Command timeout and late command completion
- Output-limit termination
- Guest crash and VMM crash
- Forced shutdown and incomplete cleanup
- Host process restart and state reconstruction
- Repeated Resolve, Launch, Execute, Inspect, and Cleanup operations
- Operation replay with equal and conflicting request fingerprints

Required leak checks:

- File descriptors
- Threads and child processes
- KVM VM and vCPU descriptors
- Guest memory mappings
- Writable storage heads
- Temporary names and directories
- Launch secrets and responder authority
- Network resources when networking is introduced

Definition of done:

- Every failure is typed and fail closed.
- Every test proves cleanup independently rather than inferring cleanup from thread exit.
- Sequential repetition and bounded concurrency show no accumulating descriptors, memory, storage, processes, identities, or named files.
- Retained evidence identifies the exact commit, host, kernel, toolchain, Generation, commands, failures, and cleanup results.

### Stage 3 - Complete the isolation boundary

The in-process thread is useful for development, but production isolation requires the VMM to become a separately contained process.

Required work:

- Run one native `soma-vmm` process per Machine.
- Launch it through `soma-jail` rather than directly inside the public Backend process.
- Apply an ephemeral identity, namespaces, cgroups, seccomp, capability removal, parent-death behavior, resource limits, an empty filesystem view, and an explicit descriptor allowlist.
- Transfer only verified kernel, memory, storage, TAP, control, and evidence descriptors.
- Ensure the VMM cannot open arbitrary host paths after confinement.
- Bound and supervise the VMM process through pidfd-backed ownership and deterministic cleanup.

Required hostile tests:

- Attempts to open host files and directories
- Attempts to access host devices other than transferred descriptors
- Attempts to connect to host or production sockets
- Attempts to reach cloud metadata
- Attempts to escape namespaces or acquire capabilities
- Fork, thread, memory, file-descriptor, and output exhaustion
- Malformed MMIO, virtqueue, block, network, vsock, entropy, snapshot, and launch-page inputs
- One sandbox attempting to observe or affect another sandbox
- VMM process crash during every lifecycle transition

Definition of done:

- A compromised guest-facing VMM remains inside the declared jail boundary.
- No sandbox can observe or mutate another sandbox's memory, disk, network, identity, control session, or cleanup record.
- The parent can always identify, terminate, reap, and reconcile the exact process and resources it owns.

### Stage 4 - Build the prepared production fast path

The production latency target depends on moving construction out of the Launch boundary.
The existing cold-boot lifecycle must not be mislabeled as this path.

Required prepared resources:

- Certified installed Generation
- Captured immutable snapshot state
- Privately mapped copy-on-write guest memory
- Prepared sterile writable storage head
- Prepared sterile network bundle
- Reserved vCPU, memory, storage, descriptor, and network capacity
- Fresh per-Instance identity, entropy, launch authority, network identity, and time repair material

Required production transaction:

1. Admit the request against exact host capacity.
2. Resolve an immutable certified Generation.
3. Claim one prepared worker and its resource leases.
4. Bind the public Instance and Operation identities.
5. Restore private memory, vCPU, interrupt, clock, and device state.
6. Attach the prepared private storage head.
7. Transfer the sterile TAP and other required descriptors into the jailed VMM.
8. Publish fresh launch material that was never present in the snapshot.
9. Resume the guest.
10. Complete authenticated repair and the fixed readiness probe.
11. Activate networking only after the verified Ready transition.
12. Return a receipt covering the exact boundary and every effective capability.

Pool depletion behavior must be explicit.
The Backend may return overload, wait inside a declared bound, or use a separately named slower preparation class.
It must never silently perform cold construction while reporting prepared restore.

Definition of done:

- Launch performs no OCI acquisition, Generation compilation, full overlay copy, or snapshot capture.
- Replenishment happens asynchronously outside the measured Launch path.
- A failed claim returns every partially acquired resource or leaves a durable reconciliation record.
- Ready is impossible before exact-Instance authenticated repair and the fixed probe complete.

### Stage 5 - Complete production networking

Networking is part of the sandbox security boundary rather than an optional afterthought.

Required work:

- Attach a prepared TAP descriptor to the guest's virtio-net device.
- Keep forwarding disabled before authenticated readiness.
- Enforce egress denied by default.
- Apply explicit destination, port, protocol, DNS, proxy, and ingress policy.
- Block cloud metadata and host-control destinations independently of caller policy.
- Support optional proxy and ingress attachment without giving the VMM network-administration capability.
- Bind activation and release to the exact Instance and Operation capability rather than only a shared UID.
- Make network release durable, idempotent, and reconcilable after crashes.

Definition of done:

- No packet leaves before authenticated Ready and policy activation.
- Denied destinations remain denied under DNS changes, redirects, IPv4 and IPv6 variants, and guest-crafted packets.
- Network activation cannot be forged by the lifecycle claimant without guest-produced evidence.
- Cleanup proves TAP, namespace, nftables, conntrack, address, route, proxy, ingress, and reservation disposition separately.

### Stage 6 - Add multi-Instance scale and admission

The single `Option<Live>` is a development constraint, not the production ownership model.

Required work:

- Replace it with an explicit bounded Instance ownership table.
- Give every Instance independent process, session, storage, network, and cleanup ownership.
- Enforce admission across vCPUs, host threads, RAM, copy-on-write pressure, disk, IOPS, file descriptors, KVM limits, network capacity, and prepared-pool inventory.
- Apply backpressure before host exhaustion.
- Preserve operation idempotency and cleanup during concurrent Launch, Execute, Inspect, and Cleanup calls.
- Reconstruct ownership and continue cleanup after daemon or host restart.

Required scale ladder:

- 1 Instance
- 10 concurrent Instances
- 100 concurrent Instances
- 1,000 concurrent Instances where the certified host shape permits it
- Higher fleet-scale campaigns across multiple hosts after the node contract passes

The host must never promise one dedicated vCPU per sandbox merely because it admits more sandboxes than physical threads.
CPU overcommit, memory overcommit, idle-worker density, and workload service levels must be explicit policies with measured consequences.

Definition of done:

- Admission refuses or queues work before an invariant is violated.
- One slow, hostile, or failed Instance cannot block unrelated Instances.
- Burst cleanup returns the host to its measured baseline.
- Restart recovery neither leaks resources nor allocates an identity or lease twice.

### Stage 7 - Repair portability and CI

No capability is admissible while required repository gates are red.

Required gates:

- Ubuntu 24.04 x86_64 build, lint, unit, integration, and KVM tests
- Ubuntu preview compatibility without treating it as the production authority
- macOS ARM64 platform-neutral build and tests
- Windows portable client build and tests
- Security and dependency policy
- KVM smoke
- Architecture checks
- Documentation links and status vocabulary
- Benchmark and evidence regeneration

Required corrections include removing the brittle 8 GiB requirement from the ordinary Ubuntu test path, fixing warnings-as-errors on Windows and macOS, making the architecture checker portable instead of using GNU-only `find -printf`, and teaching the spell checker the repository's legitimate technical vocabulary.

Definition of done:

- Every required check is green on the same commit.
- A skipped KVM test is visibly skipped and never counted as a passing KVM proof.
- Linux-specific behavior is proved on the declared Linux production target.
- Portable clients compile without importing Linux host implementation details.

### Stage 8 - Produce admissible performance evidence

Performance optimization follows correctness, identity, isolation, and cleanup.

Required campaign rules:

- Use release builds.
- Record exact commit, host, CPU, memory, storage, filesystem, kernel, KVM, toolchain, Generation, snapshot, preparation state, and command.
- Retain raw samples, failures, timeouts, cleanup outcomes, and billing or capacity scope where applicable.
- Separate cold build, cold boot, cold-cache restore, warm-cache restore, prepared restore, and already-Ready lease measurements.
- State the precise timer start and stop boundaries.
- Report p50, p95, p99, maximum, success rate, and cleanup rate.
- Measure sequential and concurrent cohorts independently.
- Run the unmodified ComputeSDK benchmark through an external SOMA adapter.

Required scale evidence:

- Small correctness cohorts during development
- 1, 10, and 100 concurrency rungs
- At least 10,000 accepted warm samples before publishing a stable 10 ms claim
- Repeated campaigns across more than one suitable production host before claiming generality

Definition of done:

- Complete server-side prepared Launch reaches the declared p50 and p99 objectives without weakening isolation or cleanup.
- First bounded command completion reaches its separate objective.
- The exact ComputeSDK Burst TTI campaign reaches its declared objective with 100 percent creation, command, and cleanup success.
- Every public number can be regenerated from retained raw evidence without provider credentials.

### Stage 9 - Finish the public agent product surface

The sandbox must be usable by agents without exposing VMM internals.

Required work:

- Stable CLI and Rust lifecycle interfaces
- MCP tools usable by Codex, Claude Code, OSA, Hermes, and other MCP-capable agents
- Template-based workload preparation
- Environment and secret delivery only after authenticated Ready
- Upload, download, and workspace attachment
- Configurable vCPU, RAM, storage, lifetime, command timeout, output limit, networking, egress, ingress, proxy, snapshot, Stop, and destruction policy
- Accurate capability negotiation and unsupported-feature failures
- Operator deployment guides for MIOSA, AWS, GCP, and custom Linux hosts
- Control-plane reference design for placement, admission, pools, reconciliation, and fleet-scale operation

Definition of done:

- An agent can create, execute in, inspect, and destroy a sandbox through the same semantic lifecycle on every supported client platform.
- The selected remote Linux Backend supplies the hardware-isolated execution where the client platform cannot.
- Documentation clearly separates local Docker or Apple development behavior from Linux KVM production isolation.
- Every option has one canonical contract, bounded validation, and truthful effective-value evidence.

## Final definition of done

SOMA is production-admitted only when all of the following are true on one identified release commit:

- Only certified immutable Generations launch.
- Public, host, VMM, and guest identities are exactly bound.
- One jailed native VMM process owns one Machine.
- Prepared restore reaches authenticated Ready without cold construction in the timed path.
- Storage and networking are private, policy-bound, durable, and reconcilable.
- Execute, Inspect, Cleanup, timeout, crash, and replay behavior are deterministic and evidence-backed.
- Multi-Instance admission protects the host under burst load.
- Every required CI, security, portability, KVM, and architecture gate passes.
- Retained release-build evidence supports every published latency and success claim.
- CLI, library, MCP, templates, configuration, and deployment documentation let agents and operators use the system without depending on internal implementation details.

Until then, the accurate public description is: SOMA has a working Linux KVM cold-boot development sandbox and strong component foundations, while production isolation, prepared restore, network composition, scale admission, and performance admission remain in progress.
