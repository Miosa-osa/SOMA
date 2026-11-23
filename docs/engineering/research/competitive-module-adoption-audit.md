# Competitive module adoption audit

- Date: 2026-08-29
- Scope: New repositories found by the second adversarial GitHub search, compared with SOMA's current implementation
- Method: Source inspection at pinned commits, interface comparison, implementation-shape comparison, and evidence review
- Status: Engineering judgment, not a security certification or reproduced benchmark

## Executive judgment

SOMA should not replace its architecture with any newly found repository.
SOMA is materially stronger in fail-closed restoration, authenticated post-restore repair, immutable artifact identity, hostile-input validation, one-process-per-Machine intent, evidence discipline, and source-file modularity.

Some competitors are still better than SOMA in narrow areas because they have working implementations where SOMA currently has a contract or an unfinished adapter.
Those advantages are real and should not be dismissed.

| Area | Project currently better | Why it is better today | SOMA action |
| --- | --- | --- | --- |
| Native cross-platform machine core | Amber | One shared ARM64 machine implementation actually runs through HVF and KVM | Preserve the lesson, but do not disrupt the Linux x86_64 fast path before it ships |
| Native Apple snapshot and warm fork | Amber | Serializable software GICv2 makes timer state restorable on Apple Silicon | Research as a distinct future macOS profile |
| Working warm-worker product loop | Amber | OCI image to template to paused worker to command is already connected end to end | Finish SOMA's real jailed `WorkerLauncher` before adding new surfaces |
| Runtime capability negotiation | Barista | A caller can require exact runtime guarantees and receive a typed failure | Add evidence-bound runtime capability reporting to SOMA's public inspect and launch surface |
| Live and incremental snapshot accounting | Tarit | CPU dirty pages and virtio DMA writes enter one snapshot accounting path | Build SOMA's unified dirty-producer ledger before any incremental snapshot feature |
| Runnable teaching progression | plyvm | The repository grows a VMM in understandable executable steps | Add a separate educational journey without contaminating production modules |
| Rootless local network ergonomics | Amber | Userspace TCP and DNS work on macOS without TAP privileges | Consider a development-only network adapter, never a production-equivalence claim |
| Live memory pressure feedback | Amber | Admission uses process RSS, worker eviction, and balloon reclaim | Add measured RSS and pressure feedback to host admission, then evaluate ballooning separately |

## The most important distinction

There are three ways a competitor can be better:

1. Its interface is deeper.
2. Its implementation is already complete.
3. Its evidence is stronger.

These are not interchangeable.

Amber is mostly better in implementation completeness and local product flow.
Barista has a useful public capability interface, although its internal module shape is much worse than SOMA's.
Tarit has useful snapshot mechanisms, but its VMM is concentrated in a 1,831-line `main.rs` and its AGPL license prevents casual code reuse.
plyvm teaches well, but it is not a hardened sandbox.

## Ten architectural insights to preserve

This section is the short learning record future SOMA implementers should read before changing the machine architecture.
Each insight names the external lesson, explains why it works, and states the SOMA adoption rule.

### 1. Serializable interrupt state can require a software device model

Amber could not reliably restore Apple timer state through its original interrupt-controller path.
It moved GICv2 behavior into userspace so the VMM owned serializable interrupt state and could recreate timer behavior after restore.

Why it works:

- State owned by the VMM can be versioned, captured, validated, and restored explicitly.
- The VMM no longer depends on an opaque host facility exposing complete snapshot semantics.
- Forced periodic vCPU exits let the software model inject timer interrupts even when the guest is compute-bound.

SOMA adoption rule:

- Keep interrupt and timer snapshot behavior behind a backend-owned interface.
- Require conformance tests for timer continuity, interrupt ordering, SMP, and repeated restore.
- Consider a software GIC only for a future native Apple adapter.
- Do not tax the Linux x86_64 KVM fast path for a capability it already obtains from KVM.

### 2. The valuable product is the complete warm-worker transaction

Amber's strongest advantage is not one VMM function.
It connects OCI preparation, guest boot, template capture, private restore, paused workers, fresh command transport, execution, and disposal.

Why it works:

- Cold image work happens before Launch.
- Snapshot-backed memory avoids copying the full guest.
- A prepared worker removes process construction and restoration from the request path.
- A fresh post-resume connection avoids preserving a live host socket in the snapshot.

SOMA adoption rule:

- Finish the real jailed `WorkerLauncher` before adding new architectural surfaces.
- Keep construction, restoration, repair, execution, evidence, and cleanup inside one tested transaction.
- Expose one small Launch interface instead of making callers coordinate the individual modules.

### 3. Dirty memory has more than one producer

Tarit and Panorama show that KVM dirty logging is not a complete incremental-snapshot contract.
Guest CPUs dirty memory through KVM, while emulated devices and host repair code can write guest memory outside that path.

Why it works:

- One tracked guest-memory write interface accounts for device-originated writes.
- Quiescing every producer closes the dirty set before capture.
- Merging KVM and userspace dirty sets produces one authoritative restoration plan.

SOMA adoption rule:

- Inventory every CPU, loader, launch-page, block, network, vsock, entropy, console, repair, and future DMA write.
- Route host writes through one deep tracking module.
- Fail snapshot publication if any producer cannot be quiesced or accounted for.
- Do not implement incremental snapshots until this proof exists.

### 4. Capability portability must be negotiated, not implied

Barista lets callers require hardware isolation, memory snapshots, copy-on-write fork, lazy restore, egress control, and guest-agent support independently.
A missing guarantee fails explicitly instead of selecting a weaker mechanism.

Why it works:

- Callers express the property they need rather than naming infrastructure.
- Backends remain free to implement the property differently.
- A typed refusal prevents silent security or performance degradation.

SOMA adoption rule:

- Publish a versioned runtime-capability result.
- Bind every positive capability to a backend, host profile, implementation version, and retained evidence identity.
- Let Launch require guarantees without selecting a provider or hypervisor name.
- Keep private-mapped restore, eager-copy restore, prepared reflink storage, and copied storage as distinct capabilities.

### 5. Static admission and live pressure feedback solve different problems

Amber observes worker RSS and evicts warm workers, while SOMA reserves conservative CPU, memory, dirty-memory, storage, and launch capacity.
Neither mechanism should replace the other.

Why it works:

- Conservative reservations protect the host against the workload's allowed worst case.
- Live observations reveal pressure, leaks, and unexpectedly expensive templates.
- Evicting unused prepared capacity recovers memory without violating active Instance guarantees.

SOMA adoption rule:

- Keep reservations authoritative.
- Add per-worker resident, shared-clean, private-dirty, page-fault, and pressure observations.
- Reconcile live observations with the durable reservation ledger.
- Evaluate ballooning only after measuring latency, reclaim effectiveness, guest behavior, and snapshot interaction.

### 6. Private file-backed memory is the common fast-cloning foundation

Amber, Clone, Firecracker-derived systems, and SOMA converge on immutable snapshot memory mapped privately into each Instance.

```text
immutable snapshot memory
            |
      shared page cache
       /      |      \
Instance A Instance B Instance C
 private     private    private
 writes      writes     writes
```

Why it works:

- Clean pages remain physically shared.
- Only pages written by an Instance become private.
- Mapping cost does not scale like copying the complete configured RAM size.

SOMA adoption rule:

- Preserve immutable snapshot backing and `MAP_PRIVATE` restoration.
- Admission must still account for private-dirty growth because sharing can disappear under real workloads.
- Never describe virtual RAM capacity as physical density evidence.
- Keep memory identity and disk-head identity independent so one Instance cannot mutate another.

### 7. Restore time is not sandbox readiness time

Several projects time KVM creation, memory mapping, register restoration, or worker handoff and describe the result as sandbox creation.
Those measurements omit identity repair, authentication, command execution, failure handling, and sometimes the control-plane round trip.

The honest boundary is:

```text
Launch accepted
  -> worker claimed
  -> Machine resumed
  -> entropy, identity, time, transport, and network repaired
  -> guest authenticated
  -> readiness command completed
  -> Ready returned
```

SOMA adoption rule:

- Retain separate timestamps for acquisition, restore, wake, repair, authentication, command, Ready, and cleanup.
- Publish raw distributions and every failure.
- Never use an internal restore measurement as a public creation claim.
- Keep the exact ComputeSDK boundary separate from the node-local internal budget.

### 8. One VMM process per untrusted Machine is worth preserving

Some VMMs place several tenants in one process to reduce management overhead.
That turns one unsafe memory defect, panic, or process crash into a multi-tenant failure.

Why one process per Machine works:

- The operating system supplies a mature process address-space boundary around the unsafe VMM implementation.
- Seccomp, namespaces, cgroups, Landlock, file descriptors, and pidfds can be scoped to one Machine.
- Cleanup and failure evidence can name one kernel process identity.

SOMA adoption rule:

- Keep one jailed VMM process per untrusted Machine.
- Recover performance through prepared processes, warm restoration, descriptor transfer, and a small control protocol.
- Prove that killing, crashing, exhausting, or corrupting one VMM leaves sibling Machines alive.

### 9. Executable teaching stages make machine architecture understandable

plyvm teaches VMM construction by adding one working primitive at a time.
That approach explains causality better than presenting only the finished architecture.

```text
host hypervisor
  -> guest memory
  -> vCPU
  -> kernel entry
  -> interrupt controller and timer
  -> console
  -> block
  -> network
  -> guest agent
  -> disposable sandbox
```

SOMA adoption rule:

- Add a separate educational journey that follows this sequence.
- Every stage should name the new interface, owned host resource, guest-visible effect, security obligation, and test.
- Keep tutorial code outside production crates.
- Use the journey to explain why each primitive exists before explaining its optimization.

### 10. Import mechanisms, not weak repository topology

Amber, Tarit, and Barista contain valuable mechanisms but also concentrate production knowledge in very large files.
SOMA's focused files and ownership-specific crates provide better locality and a smaller interface per module.

Why SOMA's current shape is stronger:

- Snapshot compatibility, device state, guest repair, storage, networking, jail, allocation, and public lifecycle semantics have separate owners.
- Host-specific resources do not leak into the portable use-case interface.
- Tests can cross the same narrow seams used by production callers.

SOMA adoption rule:

- Reimplement learned behavior behind existing deep modules.
- Add a new seam only when two real adapters or materially different policies require it.
- Reject pass-through wrappers and god files.
- Preserve provenance and perform clean-room implementation where licenses prevent source reuse.

## The combined architecture lesson

The best synthesis is:

```text
Amber's connected warm-worker transaction
        +
Tarit's complete dirty-producer accounting
        +
Barista's explicit capability negotiation
        +
plyvm's executable teaching progression
        +
SOMA's fail-closed restore, authenticated repair,
evidence discipline, containment, and deep modules
        =
the architecture SOMA should implement and prove
```

The synthesis does not authorize copying an external repository wholesale.
It records the mechanisms and interface lessons SOMA should implement under its own architecture, tests, licensing, and evidence requirements.

## Amber compared with SOMA

Pinned source: [Amber commit `54cebed`](https://github.com/lupodevelop/amber/tree/54cebedae733633ceb9f633b8f99c349d81e941e).

### What Amber does better

#### One real machine core over two native hypervisors

Amber's `Hypervisor` and `Vcpu` interfaces hide HVF and KVM behind one backend-neutral ARM64 run loop.
The core owns Linux boot, device-tree construction, MMIO dispatch, devices, snapshots, and vCPU coordination once.
The two adapters own only host-hypervisor behavior.

This is a deep module because callers learn one small machine interface while two materially different host implementations remain local to their adapters.
SOMA currently has a strong Linux KVM implementation and a separate Apple Container development backend, but it does not have one native machine core proven across KVM and HVF.

Judgment: Amber is better at native cross-platform VMM reuse today.

Application to SOMA:

- Do not insert a generic hypervisor trait into the x86_64 KVM fast path merely for symmetry.
- Record the machine semantics that must remain portable: memory regions, vCPU exits, interrupt delivery, pause, capture, restore, and fatal fault behavior.
- Introduce a native host-hypervisor seam only when SOMA has a second real native adapter, because two adapters make the seam real.
- Keep backend-owned state opaque so KVM file descriptors and HVF handles never leak into the portable lifecycle interface.

#### Software interrupt controller for Apple snapshots

Amber found that its Apple in-kernel interrupt-controller path could not restore the timer correctly.
It implemented a userspace GICv2 and periodic vCPU exits so the complete interrupt state became serializable.

This is better than pretending HVF and KVM snapshot semantics are equivalent.
It is also a warning that portability sometimes requires moving a mechanism out of the host kernel and into the VMM.

Judgment: This is the strongest new technical insight from the second search.

Application to SOMA:

- Treat interrupt-controller and timer snapshot support as a backend capability with conformance evidence.
- If SOMA builds a native Apple backend, prototype a software GIC behind an Apple-only adapter.
- Do not put software GIC work on the Linux x86_64 version 1 critical path.
- Require timer continuity, interrupt ordering, SMP, and repeated restore tests before advertising Apple snapshots.

#### Connected warm-worker flow

Amber already connects image import, template capture, private memory mapping, paused workers, command delivery, output framing, exit status, and disposal.
SOMA has stronger individual modules and retained KVM evidence, but its host pool still lacks the real jailed launcher that makes the complete product path live.

Judgment: Amber's system is less rigorous but more operationally complete at this seam.

Application to SOMA:

- The next integration priority is the real `WorkerLauncher`, not another architectural layer.
- Connect `soma-hostd`, `soma-jail`, prepared storage heads, network bundles, the restored KVM Machine, guest repair, and receipt completion through one end-to-end test.
- Keep the existing `WorkerLauncher` interface if the real adapter can satisfy it without exposing jail, KVM, TAP, or snapshot internals.
- If the adapter needs many new public methods, deepen the launcher rather than leaking construction steps into the pool.

#### Rootless userspace networking

Amber's userspace network provides outbound TCP, DNS, and inbound forwarding on macOS without a privileged TAP setup.
That is better local ergonomics than SOMA's current Docker-backed macOS path.

Judgment: Useful for development, but inferior to SOMA's Linux production policy boundary.

Application to SOMA:

- Consider a development-only userspace network adapter for a future native macOS machine.
- Preserve the same deny, allow, metadata, DNS, ingress, and receipt vocabulary across adapters.
- Report that adapter's actual protocol and enforcement limits.
- Never label userspace TCP proxying equivalent to a Linux TAP, namespace, nftables, and conntrack profile without conformance proof.

#### Live RSS and balloon feedback

Amber distinguishes configured guest RAM from actual resident memory, observes worker RSS, evicts warm workers under a fleet ceiling, and supports balloon reclaim.
SOMA has a more exact static capacity model, including resident and dirty-memory gates, but does not yet close the loop with live worker measurements.

Judgment: Amber is ahead in feedback, while SOMA is ahead in admission correctness.

Application to SOMA:

- Add per-worker RSS, shared clean, private dirty, page-fault, and pressure observations to host telemetry.
- Reconcile observations with the reservation ledger instead of replacing conservative admission with optimistic RSS.
- Evict unused prepared workers before active Instances.
- Evaluate ballooning only after measuring guest latency, reclaim effectiveness, and snapshot interactions.

### What SOMA does better than Amber

- SOMA restore fails closed and models compatibility explicitly, while Amber's KVM GIC restore logs and skips some failed attributes.
- SOMA captures before fresh identity material exists and performs authenticated repair, while Amber uses an unauthenticated fresh vsock command connection.
- SOMA's Generation and OCI pipeline applies substantially stronger hostile-input, content-addressing, and publication checks.
- SOMA's source files are capped around 300 lines, while Amber has six production files above 780 lines and three above 1,000 lines.
- SOMA separates network ownership, jail ownership, storage ownership, allocation, guest protocol, and VMM mechanisms into deep modules.
- SOMA's benchmark contract requires distributions, raw samples, failures, exact boundaries, and environment identity, while Amber's headline warm-exec result contains only five trivial-command samples.

### Amber decision

Adopt the lessons.
Do not copy the topology wholesale.
Do not cite its 30 ms result as SOMA evidence.

## Tarit compared with SOMA

Pinned source: [Tarit commit `81757b5`](https://github.com/instavm/tarit/tree/81757b54fee03fc75c59c73af06da392c8aa164e).

### What Tarit does better

Tarit explicitly merges KVM dirty logging for vCPU writes with a software tracker for virtio DMA writes during live snapshot.
It quiesces device workers for the final stop and bounds pre-copy rounds so a write-heavy guest cannot loop forever.
It reports the actual blackout when convergence fails.

This is better than adding incremental snapshots on top of KVM dirty logging alone.
SOMA documents the obligation but does not yet implement a unified dirty-producer ledger.

Application to SOMA:

- Make every guest-memory writer use one tracked write interface.
- Inventory loader, launch-page, block, network, vsock, entropy, console, repair, and future DMA writes.
- Merge KVM and userspace dirty sets only after all producers are quiescent.
- Prove that injected tracker failures prevent publication.
- Defer live pre-copy until ordinary capture and restore are production-complete.

Tarit also separates its VMM-to-orchestrator wire contract into a dependency-light crate.
SOMA already follows this principle through `soma-guest`, `soma-vmm`, and focused host protocol modules, so no new shared mega-protocol crate is justified.

### What SOMA does better than Tarit

- Tarit's VMM implementation is concentrated in a 1,831-line `main.rs`, while SOMA localizes machine, device, snapshot, and lifecycle knowledge.
- SOMA has a stronger immutable Generation and authenticated repair model.
- SOMA's storage profile measures and requires prepared reflink heads instead of allowing a slow copy fallback into the fast path.
- Tarit's source comment mentions sub-15 ms cold boot, but its README correctly refuses to treat an unversioned headline as release evidence.
- Tarit is AGPL-3.0-or-later, so SOMA must use clean-room architectural learning rather than source copying.

### Tarit decision

Adopt the unified dirty-accounting requirements and bounded-convergence semantics.
Reject its VMM file topology.
Keep live migration outside the version 1 launch path.

## Barista compared with SOMA

Source: [Barista](https://github.com/mpuig/barista.sh).

### What Barista does better

Barista advertises runtime capabilities such as memory snapshots, disk snapshots, hardware isolation, lazy restore, copy-on-write fork, live checkpoint, guest agent, and egress control.
A caller can require a guarantee and receive `CAPABILITY_MISSING` rather than an undisclosed fallback.
It also separates copy-on-write fork from full-copy fork so a slow fallback cannot impersonate the requested mechanism.

SOMA has the stronger architectural rule in documentation, but its currently exposed Machine capabilities are narrower and primarily network-oriented.
The full runtime guarantee set is not yet one machine-readable, evidence-bound public result.

Application to SOMA:

- Add a versioned `RuntimeCapabilities` result to diagnosis and inspection.
- Bind each positive capability to a backend, host profile, implementation version, and retained conformance-evidence identity.
- Let Launch require capabilities independently of choosing a backend name.
- Keep mechanism distinctions such as private-mapped restore, eager-copy restore, prepared reflink head, and copied disk explicit.
- Fail before allocation when a required guarantee is absent.

Barista also models CPU class and runtime-bundle identity as restore compatibility keys.
SOMA's content-addressed Generation, CPU template, device schema, snapshot schema, and host-profile compatibility are already more precise.
SOMA should expose that stronger model clearly rather than replace it with a single opaque CPU-class string.

### What SOMA does better than Barista

- Barista contains production files of 3,516, 3,058, 2,753, 2,235, and 2,191 lines, so its internal locality is much worse.
- Its broad session platform mixes many lifecycle concerns that SOMA assigns to narrower modules.
- Arbitrary pre-snapshot and post-restore hooks are flexible but expand nondeterminism and authority.
- SOMA's fixed authenticated repair state machine is safer for a disposable sandbox fast path.

### Barista decision

Adopt evidence-bound capability negotiation.
Reject its god-file topology and arbitrary-hook model for the default Machine.

## plyvm compared with SOMA

Pinned source: [plyvm commit `9f41d84`](https://github.com/iluxav/plyvm/tree/9f41d84e33df89058c6307841c3130be3cbdbfc9).

plyvm's value is educational.
It presents Apple HVF Linux construction as executable stages: create a VM, add memory and a vCPU, synthesize the device tree, add console, add block, add networking, and run an image.
That progression makes the origin of a sandbox easier to understand than a large finished architecture map.

SOMA's documentation is more complete, but completeness can obscure causality for beginners.

Application to SOMA:

- Add a separate `docs/journey/` sequence that starts with an empty Machine and adds one primitive per chapter.
- Each chapter should show the new interface, the new host resource, the new guest-visible effect, and the new security obligation.
- Keep tutorial code out of production crates.
- Do not inherit plyvm's pervasive `unwrap` use or lack of containment evidence.

## AgentENV, FCVM, Yobo, and other runtime finds

These repositories mostly confirm existing SOMA decisions rather than reveal a better VMM core.

- AgentENV reinforces content-addressed environment storage, distributed placement, and userfaultfd restoration, but remains a Firecracker platform.
- FCVM provides a broad Firecracker clone and durability test quarry, but it is not a new VMM architecture.
- Yobo demonstrates a compact libkrun-based OCI integration and observation layer, but libkrun owns the actual machine.
- Kuasar confirms the value of a first-class sandbox abstraction and removing per-container shim overhead, but its multi-sandbox container-runtime scope is much broader than SOMA's engine.
- Nimbus's libkrun fork shows that tenant-scoped bind-address behavior may require changes below a high-level network interface, but SOMA's network namespace and descriptor-transfer design gives it a clearer Linux ownership seam.
- SigmaOS and Aleph were code-signature matches, but their relevant virtualization code does not displace the stronger machine references already selected.

## Priority adoption plan

## How the 2026-08-30 Isorun observation changes the plan

The retained Isorun experiment observed a vendor-reported `create_ms` p50 of 22 ms in a ten-request sequential Node cohort and 73 ms in each of two 100-request Node cohorts.
The vendor timer endpoints are undocumented, the raw records are not yet retained, and the quantity is not equivalent to SOMA authenticated Ready.
The values therefore inform architecture and test design but do not establish a directly comparable performance result.

The important signal is that sequential performance did not predict burst performance in the observed service cohorts.
SOMA must treat concurrency as a first-class workload dimension across all preparation classes.

Required implementation consequences:

1. `soma-hostd` must claim prepared workers without constructing them on the request path.
2. Pool admission must reserve CPU, memory, private-dirty memory, storage, network, file-descriptor, process, and concurrent-repair capacity before a claim wins.
3. Replenishment must run behind bounded background work and must not compete without limits against active Launch repair.
4. A depleted pool must return explicit overload or a separately named slower preparation class rather than silently moving work into the timer.
5. Per-stage telemetry must identify whether burst degradation came from claim contention, page faults, KVM resume, guest repair, authentication, command execution, or cleanup pressure.
6. Benchmark cohorts must cover concurrency 1, 10, 25, 50, 100, and capacity-edge rungs instead of reporting only a sequential best case and one final burst.
7. Each rung must retain raw success, failure, cleanup, preparation, and scheduling evidence.

The new observation strengthens Priority 0 rather than adding a new architectural module.
The real jailed `WorkerLauncher`, prepared pool, authenticated repair, and complete receipt remain the shortest path to a defensible advantage.

The evidence-specific corrections are tracked in [the Isorun evidence review](../reviews/2026-08-30-isorun-evidence-review.md).

### Priority 0: finish the actual product path

Implement and prove the real jailed `WorkerLauncher` that composes the modules SOMA already owns.
This closes the most important gap that Amber currently exposes.

Acceptance:

- One accepted Launch claims one prepared worker.
- The worker receives only owned descriptors and typed authority.
- The restored Machine completes authenticated repair and the first command.
- The receipt proves timing, effective capabilities, result, and cleanup.
- A 100-way run proves isolation and no leaked processes, namespaces, TAPs, storage heads, or leases.

### Priority 1: public evidence-bound runtime capabilities

Implement Barista's best interface idea with SOMA's stronger evidence model.

Acceptance:

- Diagnosis and inspection return a versioned capability object.
- Every positive claim names its evidence and compatible host profile.
- Launch can require capabilities without selecting a provider.
- No backend silently weakens a requested mechanism.

### Priority 1: unified dirty-producer ledger

Implement Tarit's and Panorama's strongest correctness lesson before incremental snapshots.

Acceptance:

- Every host write into guest memory is registered through one seam.
- CPU and device dirty sets merge deterministically.
- Quiesce closes all producers before the final set is read.
- Mutation tests prove that omitted and failed producers prevent snapshot publication.

### Priority 2: live resource feedback

Combine SOMA's conservative admission with Amber's measured pressure feedback.

Acceptance:

- Per-worker resident, shared, and private-dirty measurements feed host observations.
- Reservations remain authoritative even when observed use is low.
- Warm-worker eviction is explicit and measured.
- Ballooning remains disabled until a separate benchmark proves benefit and safety.

### Priority 3: native macOS research profile

Investigate Amber's software GIC and userspace networking only after Linux version 1 is complete.

Acceptance:

- The profile is named separately from Linux KVM.
- Snapshot timer and interrupt state survive repeated restore.
- Network enforcement differences are explicit.
- No macOS number is used to support a Linux production claim.

## Final recommendation

The newly found repositories do not reveal a better complete architecture than SOMA.
They reveal four places where SOMA can improve without losing its strengths:

1. Finish the real warm-worker integration instead of adding more planned modules.
2. Expose runtime capabilities as evidence-bound, caller-requirable guarantees.
3. Centralize all dirty-memory producers before implementing incremental state.
4. Add live memory-pressure feedback while keeping conservative admission authoritative.

Amber supplies the strongest implementation lesson.
Tarit supplies the strongest snapshot-correctness lesson.
Barista supplies the strongest capability-interface lesson.
plyvm supplies the strongest teaching lesson.

SOMA should adopt those lessons through its existing deep modules, not import their weaker file structures, error policies, or benchmark claims.
