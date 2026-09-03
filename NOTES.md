# Notes

Entries are dated and append-only.
Each one records what was true when it was written, and none of them is updated afterwards.
For the current status of any capability, read [the claim ledger](docs/claim-ledger.md) rather than an entry below.

## 2026-08-31 - The contract, the dimensions, and the gap map

SOMA is measured against an interface that already exists rather than against a competitor feature list. ComputeSDK defines one provider surface and MIOSA is already on its provider list beside E2B, Modal, Vercel, Daytona, Runloop, Superserve, Isorun, and Declaw, so the interface is a specification. The three records are [the provider contract](docs/research/provider-contract-gap-analysis.md), [the capability survey](docs/research/sandbox-provider-capability-survey.md), and [the gap map](docs/research/gap-map.md).

The gap that costs the most is not a missing subsystem but an unwired one. `soma-netd` is about 7,800 lines of namespaces, netlink, nftables, address management, DNS, ingress, and reconciliation, and `soma-kvm` implements a TAP backend, and the KVM Backend uses neither: `link_down_network` hands the guest a placeholder with the link down. Installing a package, cloning a repository, pulling an image, and calling a model API are the same missing thing wearing four names.

The gap that is deepest is the guest protocol. It carries eight frame kinds and `GuestCommand` carries a program, arguments, a timeout, and an output bound. There is no standard input, environment, working directory, user, signal, or streaming, and no frame reads a file or attaches to a terminal. Six filesystem operations and an interactive terminal are in the contract and cannot be written above that protocol however the host API is designed, so framing precedes operations.

Shape turned out to be the finding worth the most care, because the obvious statement about it is wrong. SOMA does accept a `MachineShape` and the command line accepts all three fields. The vCPU count is fixed at one by the machine contract and the receipt honestly reports the contract's value rather than echoing the request. The memory size must exactly equal the size the Generation's snapshot was captured with, and `compatibility::check_header` rejects a mismatch as a memory-layout incompatibility rather than quietly ignoring it. The overlay comes from the Generation. So shape is a Generation build parameter and each Generation has exactly one launchable shape, which is a harder constraint than a fixed default because a range means a Generation and a captured snapshot per size.

Two things the survey found that SOMA has never designed, as opposed to not yet built: a per-sandbox snapshot, which every provider has and which holds tenant state that SOMA's build-time snapshot deliberately does not; and credential delivery, where the Template schema carries secrets and nothing delivers one to a running Instance.

On the optimisation side the map records that cohort variance at concurrency 100 is about 40 percent of the median, so an improvement smaller than roughly 60 ms cannot be told from noise by one cohort. Measurement method is on the critical path for optimisation work, not beside it.

## 2026-08-31 - Five hundred sandboxes, and the figures that had no files

eval-1 was never off. `ssh eval-1` resolved to 192.168.1.1 through a wildcard `.lan` answer, which is a router that refuses SSH, and a subnet scan that covered only half its range was then read as proof the host was gone. It is at 10.0.110.245 with 80 threads and its whole prepared store intact. The lesson is the cheap one: a name that resolves is not a name that resolves correctly, and a partial scan proves nothing.

Checking the retained files before quoting them found that two of the three figures in use had nothing behind them. There was no 62.6 ms sequential result anywhere; the only retained sequential sample was a single run at 79.9 ms, which is not a distribution. The stage medians that the whole prepared-worker argument rested on had never been written down at all, because the script that produced them only printed. The one figure that was real was 164.9 ms at concurrency 100.

So all of it was measured again on `b65f41f` and retained, in [the eval-1 burst and sequential record](docs/evidence/2026-08-31-eval1-burst-and-sequential.md). Sequential is 65.5 ms over 25 samples, close to the figure that had been quoted and now with a file behind it. Concurrency 100 is a cohort p50 between 166.0 and 233.5 ms with a cohort median of 181.4 ms, over five cohorts, and **five hundred of five hundred sandboxes succeeded**.

The spread is the finding that matters for method. Sixty-seven milliseconds between the fastest and slowest cohort is about 40 percent of the median, so a single hundred-way cohort is not a point estimate, and the previously retained 164.9 ms sits at the optimistic edge of the same distribution rather than showing a regression. Comparing one cohort against one cohort, which is what had been done, cannot distinguish a change from noise. Cohort order was not a factor and the host held no leaked state between runs.

Stage medians are now retained beside the totals they describe, from the same sandboxes: machine construction 48.0 ms, Ready a further 57.7 ms, the command a further 79.0 ms. Machine construction is the segment a prepared worker removes and it measured 2.71 ms uncontended, so the prepared-worker case now rests on artifacts. Removing all of it would leave about 133 ms, which is arithmetic rather than a measurement, and 133 ms against Isorun's 73 ms is why [host class](docs/research/host-class-and-burst-projection.md) remains part of the result.

Both harnesses now write their samples and their stage medians to disk from one cohort. The reason a stage median could be quoted that no file contained is that the two came from different scripts on different runs and one of them only printed.

## 2026-08-30 - Restore was broken on main, and the host is part of the result

Two findings, one a defect and one a constraint.

`f614458` broke every restored sandbox and the break reached `main`. It derived a per-Instance vsock context identifier for the machine and left the launch page carrying the constant that identifier replaced, and the guest agent refuses a session when the two disagree, so a restored machine reached its repair point and died there with `poisoned by Transport`. `c0fd993` repairs it and adds the regression test that compares the two values, which is the whole of the defect. The record is [the launch page context identifier defect](docs/evidence/2026-08-30-launch-page-context-identifier-defect.md). Measurements taken before `f614458` are unaffected, because one constant then served both the machine and the page and they agreed.

Finding the defect needed the guest console, which the failure path discarded: a sandbox that fails never reaches cleanup, and only cleanup received the machine's evidence. `SOMA_KVM_TIMELINE` now writes each sandbox's milestone offsets and per-phase durations, and a failed one's console beside them. It is a diagnostic with no signature, no identity binding, and no stable schema, and it produced the first per-stage breakdown of a restored sandbox, retained as [the restore stage timeline](docs/evidence/2026-08-30-x86_64-restore-stage-timeline.md). That breakdown shows machine construction at 2.71 ms uncontended against the 44.4 ms the eval-1 hundred-way cohort spends on the same segment, which is the prepared-worker case stated in measured numbers.

The constraint is that eval-1 cannot produce a leading burst figure. Removing all 44.4 ms of machine construction leaves about 128 ms against Isorun's observed 73 ms, so the gap is not software that remains to be written; it is a 2017 processor. Reporting an eval-1 burst figure would understate the engine while appearing to measure it. Host class is therefore part of the result and must be named in the artifact, and a burst campaign needs a current high-core-count server part. The reasoning, and the clear line between what is measured and what is projected, is in [host class and the burst result](docs/research/host-class-and-burst-projection.md).

`98cb00c` then landed the machine half of the prepared worker. `restore_sterile` produces a machine that has paid for memory mapping, VM creation, vCPU and device restoration, interrupt routing, and the event loop, and holds neither of the two authorities that belong to an Instance: the private disk head and the context identifier. `Sterile::assign` installs both. Every restore now composes those two halves, so a prepared worker and a direct restore cannot reach different device state, and installing the identifier moved into assignment because that is the only place it can happen for a worker built before its Instance exists.

Two defects are open and deliberately not folded into either commit. A restored guest observes a large time jump despite `KVM_SET_CLOCK` and `IA32_TSC` both being restored, which trips the netdev watchdog and then breaks the shutdown acknowledgement, so cleanup cannot be proven on a host slow enough to keep a sandbox alive for five seconds. A filesystem without reflink copies the whole overlay template per launch, costing seconds and about ten gibibytes, which is why a development laptop cannot produce a latency result.

## 2026-08-30 - Isorun telemetry strengthens the prepared-worker priority

The new Isorun experiment is a valuable burst signal but currently records provider-reported `create_ms` without a retained harness or raw samples, so it must be classified as independently collected vendor telemetry rather than independently measured server timing.
Its observed Node cohorts returned 22 ms p50 sequentially and 73 ms p50 in two concurrency-100 runs, which does not prove the cause but demonstrates that sequential behavior did not predict burst behavior in those cohorts.
SOMA should respond by finishing the real jailed prepared-worker transaction, reserving every bottleneck before claim, bounding replenishment outside Launch, returning explicit overload or a named slower preparation class on depletion, and measuring concurrency rungs with per-stage evidence.
The required evidence corrections are in `docs/reviews/2026-08-30-isorun-evidence-review.md`, and the architectural consequences are in `docs/research/competitive-module-adoption-audit.md`.

## 2026-08-29 - Adversarial GitHub pass found Amber and code-signature-only VMMs

A second GitHub pass searched raw virtualization implementation fingerprints rather than repository names, including KVM creation and memory ioctls, dirty-log constants, vhost and virtio constants, Apple `hv_vm_create`, Windows `WHvCreatePartition`, userfaultfd snapshot combinations, and fully qualified rust-vmm types.
It found Amber, plyvm, Tarit, AgentENV, SigmaOS, Aleph, Barista, Yobo, FCVM, and tenant-networking libkrun variants that the original repository-description search did not surface.
Amber is the strongest new reference: a one-star ARM64 VMM with a shared machine core over Apple HVF and Linux KVM, a userspace GICv2 that makes Apple snapshot timers restorable, OCI-to-squashfs images, private-mapped CoW restores, warm workers, framed vsock exec, userspace networking, memory admission, and one-process-per-VM isolation.
Its retained M1 Pro evidence reports five warm exec samples between 30 and 32 ms and about 16 MiB idle RSS per 512 MiB-cap fork, but it provides no real-hardware KVM performance numbers, no large latency distribution, and no 10 ms result.
Source inspection also found warning-only KVM GIC restore failures, ignored agent I/O results, and several files above 1,000 lines, so SOMA should import the mechanisms and cross-platform lessons while keeping fail-closed restoration, authenticated repair, smaller modules, and stronger benchmark evidence.
The comparative adoption decision is to prioritize the real jailed worker integration, public evidence-bound runtime capabilities, a unified dirty-producer ledger, and live resource feedback.
Amber's software GIC and userspace network belong to a later native macOS research profile, not the Linux version 1 critical path.
The permanent learning record is the ten-insight section in `docs/research/competitive-module-adoption-audit.md`.
It preserves the mechanism, reason, and SOMA adoption rule for software interrupt state, warm workers, dirty producers, capability negotiation, live pressure, private mappings, honest timing, process containment, executable teaching, and clean-room modular adoption.

## 2026-08-30 - soma-hostd review: durable replay, admitted claims, and exact capacity

A line-by-line review of `crates/soma-hostd` produced twenty-six confirmed findings against the crate and one against `soma-netd`; every one of the crate findings is now fixed with a regression test, and the review outcome is recorded here rather than in a separate report.

Replay idempotency is durable. The registry was the only record of an `OperationId` binding, so a restart or an eviction let the same operation claim a second worker and let a changed intent win; `Pool::open` now seeds the registry from `Ledger::claims`, every completed claim records its operation, and a registry miss on a recorded operation is answered from `Ledger::claim_of`, so the ledger and not the process is the binding of record.
Capacity admission is wired into the claim path. A pool is opened against a `PoolAdmission` that binds the host `Admission` to the exact Machine shape its workers are prepared for: `Pool::claim` reserves every visual-atlas dimension atomically before it wins a slot and answers a refusal with the typed `CapacityRejection` naming the gate, the transfer frees the Launch slot at commit, every teardown returns the reservation, and reconciliation rebuilds the committed usage of each retained Instance, so no worker is granted for an Instance the host cannot admit and the daemon binary builds its certified profile from explicit operator inventory arguments.
The CPU gate is exact. Rounding each Instance up to whole milli-units lost one admission per about forty-two Instances at ratios that do not divide 1000, so the atlas ladder row of 42 at 3:1 reproduced as 41; `Usage` now keeps raw vCPUs per workload class and applies the ratio once to the census, per node as well as per host, and `estimate` derives the overcommitted bound by cross multiplication and includes the section 14 burst limits it had ignored.
Validation is unavoidable and every division is checked. `HostProfile::validate` and `MachineShape::validate` return the `CertifiedProfile` and `ValidShape` newtypes that `estimate`, `Demand::of`, and `Admission` take, so an unvalidated profile or shape can no longer reach a plain division and abort the process, and an overflow is `Gate::Arithmetic` instead.
Each reservation owns its cleanup slot, `Gate::HostMemory` names the combined memory check that a guaranteed gate used to mislabel, and a placement refusal reports requested, committed, and limit in one comparable dimension.
The ledger proves more on its own: the fold rejects any record that skips a phase of the transition table, moves the lease generation that only a claim bumps, or claims a worker that was already assigned, and an `Assigning` record names the fresh per-Instance references before the first authority frame so a crash between the broker's assignment and the commit releases the leased head and the network bundle at the next reconciliation instead of leaking them.
The lifecycle races are closed: a starting worker stays in the owned table with its handle taken out so the blocking launcher round trip runs outside the pool-wide lock and a concurrent release is refused by phase rather than told the pool owns nothing, reconciliation passes are serialized and the slot table refuses a worker identity it already holds, the claim deadline is measured from the won slot and re-checked before the assignment commits, a transfer grant issued by another pool is refused and destroyed through the pool that issued it, and every replenishment pass reaps the construction threads that finished.
The daemon answers a replay from the disposition of the worker behind it: the launch page it delivered when the transfer succeeded, and a typed terminal failure when the worker is destroying, dead, or gone.
`InstanceShape` is now `MachineShape` as `CONTEXT.md` and the visual atlas name it, and the overlay key field and argument are `template_digest` and `--overlay-template-digest` with a note that this is the storage crate's sterile ext4 template and not the glossary Template.
`Limits::min` reports replenishment urgency in `ReplenishReport::urgent` instead of being validated and never read, and the tests now prove Generation eviction, the pool key digest changing with every one of its eleven components, and the rollback of every gate where it is exercised.

Two items are outside this crate and remain open. `crates/soma-netd/src/ingress.rs` has two imports that are unused off Linux, so the cross-target CI jobs fail before `soma-hostd` is reached; `soma-hostd` itself passes `cargo check` and `cargo clippy --all-targets` on `x86_64-apple-darwin` and `aarch64-pc-windows-msvc` once that dependency is capped, and the fix belongs on `main`.
`RUSTDOCFLAGS=-D warnings cargo doc --no-deps` passes for `soma-hostd` but not for the workspace, because `soma-storage` and `soma-kvm` still have unresolved intra-doc links, so the documentation build is not yet a gate in `scripts/check.sh`.

## 2026-08-29 - State of the art is an all-dimensions admission standard

SOMA now defines state of the art as simultaneous admission across architecture, modularity, correctness, isolation, compatibility, evidence, performance, density, portability, operability, supply chain, and usability.
No average score exists, and one red applicable dimension blocks a support or performance claim.
The permanent standard is `docs/standards/sota-engineering-standard.md`, while the dated current assessment is `docs/reviews/2026-08-29-overall-engineering-assessment.md`.
The standard preserves one production sandbox architecture, deep modules with small interfaces, capability-triggered safety primitives, explicit evidence classes, and dependency-ordered closure before breadth.

## 2026-08-29 - Template compiler slice 1 emits canonical Template Locks

`crates/soma-template` implements the first slice of tickets T1 through T5 of the Template implementation map: it parses one `soma.template/v1alpha1` TOML document, composes a flat ordered module list with transitive requirements from a bounded in-memory registry of four data-defined modules, validates the ten rejection classes from the template system design, pins the OCI platform digest through an `OciResolver` seam, and emits the canonical `SOMALOCK` version 1 lock whose SHA-256 is the `LockId`.
The parser reads a generic TOML table through a claim-tracking reader, so an unknown key anywhere is reported with its full dotted path during parsing, before any validation rule runs, although within one table a missing or mistyped required field is reported before an unknown key; an unsupported `schema` value wins over every later error, and the document is bounded to 256 KiB before the parser sees a byte.
The specification's minimum example placed `modules` after the `[workload]` header, which TOML reads as `workload.modules`; the design document now lists `modules` before the first table and explains why.
The lock has fifteen fixed-order big-endian fields with explicit presence bytes and length-prefixed bounded strings: template schema, policy version, the digest of the composed selection, resolved digest, size, and platform, ordered module identities with schema versions and digests, the effective command with `/` and `root` defaults applied, resources, the normalized network envelope with canonical CIDR text, lifecycle, the environment contract including module-sealed values sorted by name, secret references with delivery and scope sorted by name, the policy ceiling, and the Backend capabilities.
`name`, `description`, the mutable image text, and TOML layout never enter the bytes, and no resolved secret value ever enters the crate; conservative secret-literal detection runs over environment values, secret sources and scopes, command fields, the description, and module sealed values, matching name markers on whole `_` components and known credential shapes inside separator-delimited segments, so a credential the heuristics do not recognise is not detected.
The decoder is bounded, rejects trailing bytes, accepts only canonical forms such as sorted destination lists, sorted environment and secret names, and canonical CIDRs, and re-applies the validator's shape rules to every decoded record, so the revision view is built from a decoded lock without re-validation.
The content digest is computed from the composed selection rather than the authored document: `deny` with destinations and `allowlist` with the same destinations, explicit and omitted defaults, an explicit command equal to the module default, a transitive requirement listed or omitted, a restated sealed value, reordered or duplicated destination lists, textual CIDR variants, and reordered environment or secret tables all produce one `LockId`, while composed module order, every resource, command, lifecycle, environment, secret, and platform field, the resolved digest, the policy ceiling, and the Backend capabilities each move it.
Secret delivery targets are exclusive: two secrets to one environment name, guest file, or destination, a secret to a name the Template or a module seal already fills, and a file secret inside a module-owned path are rejected with the owner named.
The `TemplateRevision` view maps field by field onto `soma_generation::generation::template::TemplateRevision`: image digest and platform, the three Machine shape dimensions, `ttl_seconds` from the maximum lifetime, and provenance through the content digest; the envelope maps exactly onto the portable `NetworkPolicy` only for fully denied and unrestricted-egress cases and otherwise fails closed, because `soma` has no destination-filtered egress class until the policy compiler exists.
The lifetime bound is the compiler's thirty-day `MAX_TTL_SECONDS`, but `shape()` is not pre-validated against the compiler profile: profile v1 accepts exactly one vCPU, so the specification example's two vCPUs build a shape that `soma_generation::TemplateRevision::new` rejects as unsupported, which the crate-boundary test in `crates/soma-generation/tests/template_boundary.rs` records by building the compiler's revision from the view.
Ninety-four tests cover golden bytes and identity for the specification example, repeated-resolution equality, reordering and renaming identity, one test per rejection class naming the module and field, every lock prefix and every single-bit flip with the rule that any accepted flip re-encodes identically, same-length byte substitutions that the decoder must reject, garbage, prefix, mutation, and deep-nesting TOML sweeps, cycles, unpinned transitive inputs, secret literals in every bound field, exclusive delivery targets, canonical CIDRs, policy widening, and the revision mapping.
The bit-flip and ceiling tests found one implementation bug before commit: IPv4 CIDR containment masked at 32 bits while both families were left-aligned in 128 bits, so a narrower IPv4 request never fit inside a wider ceiling.
`toml` 1.1.4 is the only new dependency, pinned with the `parse`, `serde`, and `std` features and recorded in the dependency policy.
The registry, resolver, and filesystem oracle are seams with deterministic test implementations only; nothing here contacts a registry, inspects a normalized rootfs, plans a build, constructs a Generation from a lock, publishes, or resolves remotely, and those remain tickets T6 through T18.
Within T1 through T5 the multi-workload golden corpus, module resolution from a content-addressed store, the user, port, process-name, and mount-destination conflict fields, the field-origin `explain` result, the proof that a Launch override narrows but cannot widen the locked ceiling, and the workspace binding are still open.
## 2026-08-29 - soma-hostd claims sterile workers once and transfers authority exactly once

Decision-map ticket #12 now has a node-local implementation: `crates/soma-hostd` keeps one bounded pool per exact `PoolKey` of host profile digest, Generation, CPU and memory class, overlay class, and network profile digest, and every worker moves through `Constructing`, `Sterile`, `Claiming`, `Assigned`, `Running`, `Destroying`, and `Dead` with a typestated handle whose transitions are one compare-and-swap over a packed phase and 56-bit lease generation word.
Only `Sterile` is claimable, the claim is the one transition that bumps the lease generation, and the transition table has no arrow back to `Sterile`, so a worker that was ever assigned can be neither reclaimed nor scrubbed; the ledger projection additionally fails closed on a sterile record after an assignment.
The claim registry binds each `OperationId` to a request fingerprint: a replay with the same fingerprint returns the identical `ClaimOutcome` for as long as the ledger holds the claim, across a registry eviction and a restart, a changed fingerprint is a typed `OperationConflict`, a concurrent replay waits at most the claim deadline for the in-flight attempt, and an exhausted pool answers with a typed `Exhausted` carrying the occupancy instead of queueing; an operator may instead select inline construction, which is labelled `OnDemand` so it never mixes with prepared-worker results.
Transfer delivers identity, deadline, entropy, launch page, disk head, TAP, control, and commit as eight acknowledged frames through the `WorkerHandle` seam; a rejection, timeout, partial acknowledgement, closed channel, claim deadline, resource fault, ledger fault, or intent mismatch at any step records the fault, destroys the worker, releases the leased head and bundle by reference, and closes the ledger, and dropping a claim grant without transferring does the same.
Disk heads are leased through the real `soma-storage` `HeadLedger` with the Instance-derived token and the network bundle is assigned as `soma-netd` would assign it, producing the exact `LaunchNetwork`; the live `clone_head` and `Broker::assign` mechanisms sit behind the `ResourceBroker` seam and the jail from ticket #9 behind the `WorkerLauncher` seam, and the in-process implementations in `soma_hostd::testing` drive every policy test without a kernel.
Replenishment reserves one of `replenish_concurrency` slots before it spawns, honours the construction deadline, stops at `max`, and refuses to run until a restarted pool has reconciled; reconciliation marks every nonterminal ledger entry without a live slot suspect, probes the launcher by recorded identity and the brokers by recorded references, terminates or releases, and retains a running Instance that is still alive.
The ledger is one create-exclusive, SHA-256-checksummed, 232-byte record per event with file and directory `fsync`, and a second writer on the same directory is a typed `Contended` failure rather than a silent overwrite.
Admission implements the visual-atlas capacity equation: `HostProfile` carries the host reserve, a labelled `MeasuredOverhead`, per-dimension limits, and per-class overcommit ratios; `Admission::reserve` computes every demand with checked arithmetic, applies it to a copy of the committed usage gate by gate, places it through the `NumaPlacement` hook, and only then commits, so a `CapacityRejection` names the gate with requested, committed, and limit while usage is untouched; guaranteed and elastic memory are an explicit enum with separate gates, and `estimate` reproduces the ladder, for example 49 Instances by memory for the 512 MiB plus 64 MiB shape on the 16-thread, 32 GiB host with a 2-thread, 4 GiB reserve.
The 64 MiB atlas overhead remains a labelled placeholder and the `x86_64` PVH boot proof's 3.6 MiB single-sample debug-build resident total is offered as the labelled `PVH_BOOT_SINGLE_SAMPLE` input; neither is a certified per-VM figure.
Tests prove one winner among 100 barrier-released claimers on one sterile worker fifty times, identical concurrent and sequential replay, changed-intent conflict, no reuse after release, every step and every fault class destroying the worker with a ledger disposition, immediate exhaustion, a 32-thread replenishment storm never exceeding three concurrent constructions, 100 operations completing over ten workers with bounded retries, restart reconciliation over a surviving process table, every admission gate by name with rollback, and the atlas ladder.
Claim latency over 1,000 claims with one durable record each measured p50 52.6 us and p99 69.8 us with the ledger on tmpfs and p50 1.14 ms and p99 8.75 ms with the ledger on the development ext4 NVMe root, where transfer with its nine records cost p50 9.8 ms and p99 67 ms; these are in-process debug-build numbers on a busy host that measure the ledger `fsync` protocol, not the fast-path budget, and the durable record placement is the first thing to revisit when the real launcher arrives.
The daemon is a single-threaded `SOCK_SEQPACKET` skeleton over one pool that does not authenticate its peer, carries no descriptor transport yet, and starts only with `--launcher in-process`; the jail adapter is pending ticket #9 and the live 100-way proof with a real VMM, XFS heads, and TAP bundles is pending ticket #13.

## 2026-08-29 - A captured Generation restores into independent authenticated Instances

Decision-map ticket #7 now has its live half.
`crates/soma-kvm/src/x86_64/snapshot/` pauses a running machine at the guest agent's disconnected repair point, proves the quiesce preconditions the device surface requires, reads KVM and device state in the certified order while vCPU 0 sits outside `KVM_RUN`, and publishes `memory.raw`, `overlay.raw`, and `state.somasnap` through the existing codec, each written to a private staging name, flushed, hashed through the handle that wrote it, and published with a link that fails when the name already exists.
Restore validates constant-size compatibility before it maps anything, maps the memory object `MAP_PRIVATE | MAP_NORESERVE` without copying it, registers the certified slot, rebuilds the interrupt controller, timer, and five devices with entirely fresh backends, creates the vCPU, installs CPUID, the allowlisted MSRs, and every register group, registers eventfds and interrupt routes before arming the captured interrupt state, writes a fresh launch page into the separate slot the snapshot never contained, and only then resumes.
Capture happens before any launch page exists, so the image cannot contain an Instance identity, session key, context identifier, or network identity; ADR 0030, the pre-launch snapshot capture point, records that decision and replaces the `GuestAuthenticated` quiesce precondition with the pre-launch marker the agent prints; it was written as ADR 0024 and renumbered on 2026-08-30 because ADR 0024 was already the per-Instance guest responder authority.
The guest agent now flushes filesystems and announces the repair point on the console before it blocks, and waits for its assigned vsock context identifier because a restored driver adopts the fresh assignment asynchronously through the transport-reset event.
Three implementation facts came out of the live runs and are now contract: `KVM_GET_SUPPORTED_CPUID` answers from whichever host core services the call, so the CPU template pins the topology and cache leaves as well as the initial APIC identifier; `IA32_XSS` must be carried or the restored guest faults in `XRSTORS` the first time a task returns to user mode; and `IA32_SPEC_CTRL` must be carried or a restored machine runs with weaker mitigations than the one that was captured.
A real `node:22` Generation was captured once and restored thirteen times on one host: every Instance reached `Ready`, ran `/usr/local/bin/node --version`, and shut down orderly, two Instances differed in Instance identity, hostname, machine identity, context identifier, and private overlay head, a file one Instance wrote was invisible to the other, and the memory object's digest was unchanged afterwards.
A flipped byte in the state manifest, a flipped byte in the memory object under installation-time verification, and a foreign CPU-template digest are all rejected before any VM exists, and the published objects carry no decodable launch page.
The retained result is `docs/evidence/2026-08-29-x86_64-snapshot-restore.md`; it proves no cold page cache, no hundred-way burst, no jail, no prepared workers, no network egress, and no latency objective.

## 2026-08-29 - XFS reflink heads must be prepared outside Launch

Decision-map ticket #11 now has its mechanism and its measurement: `crates/soma-storage` owns published overlay classes with exact-size admission, a Linux profile probe that proves XFS plus one working `FICLONE` before any head exists, sterile ext4 templates from a pinned `mke2fs` and `e2fsck -fn` invocation with a private `mke2fs.conf`, derived UUID and hash seed, fixed creation time, and lazy initialization off, descriptor-only `FICLONE` head creation with file and directory `fsync` plus size and shared-extent verification through `FIEMAP`, the two-clone isolation and `ENOSPC` proofs, a single-use head ledger, durable release, and report-only reconciliation.
Every live proof runs on a loop-backed XFS `reflink=1` filesystem inside a digest-pinned privileged Ubuntu 24.04 container through `scripts/xfs-reflink-bench.sh`, because the development host root filesystem is ext4 without reflink; the tests are ignored by default and fail on a missing prerequisite instead of passing silently.
The matrix in `soma-storage-bench` crossed 100 MiB, 1 GiB, and 4 GiB templates, sterile, preallocated, and fragmented extents, warm and cold cache, 1, 10, and 100 simultaneous clones, ten percent free space, and 100 concurrent unlinks, and compared in-process `FICLONE` with the `cp --reflink=always` subprocess: 69 cells, 200 raw samples each, zero failures.
The best 100-way cell has a complete-clone p99 of 9.9 ms and the worst 1,868 ms against the 1.00 ms disk share of fresh resource activation, and even the best single clone is 1.25 ms at p99 because the durable file `fsync` alone costs 0.6 ms, so on-demand cloning is not admitted and prepared sterile heads are mandatory.
Clones of one template serialize on the template inode, so a 100-way burst costs about 100 times one `ioctl` and parallel replenishment gains nothing per class.
The `ioctl` cost is proportional to the source extent count at about 0.6 us per extent, so template size hardly matters but a fragmented template must never be certified.
`FICLONE` maps unwritten source extents as holes rather than shared extents, so a head never inherits its template's `fallocate` reservation and the free-space evidence plus the `ENOSPC` proof remain the only capacity guard.
The in-process `ioctl` beats the `cp` subprocess by five times per head, and cleanup must also stay off the request path because 100 concurrent unlinks raised the 100-way p99 from 21.5 ms to 57.1 ms.
The retained result is in `docs/evidence/2026-08-29-xfs-reflink-profile.md`; it is a loop-backed decision input, not a production-host latency claim, and the next dependency is the prepared-head pool in the host allocator.

## 2026-08-29 - soma-netd prepares sterile bundles and activates only after repair

Decision-map ticket #10 now has a first implementation: `soma-netd` owns namespaces, TAP and veth devices, `/30` IPAM, MAC derivation, nftables text, conntrack zones, resolver policy, exclusive port reservations, the durable ledger, single-use claimant-bound activation, ordered release, and reconciliation, while the VMM side receives one TAP descriptor through `SOCK_SEQPACKET` plus `SCM_RIGHTS` with a fixed typed header.
Namespaces, TAP devices, link flags, addresses, routes, and veth pairs use direct syscalls and a minimal `RTM_NEWLINK` encoder over `libc` only; no netlink or nftables crate was added.
The version 1 firewall mechanism is generated ruleset text fed to the pinned `/usr/sbin/nft -f -` on standard input, and zone flushing shells to the pinned `/usr/sbin/conntrack -D -w <zone>`; both are documented interim seams because a libnftnl binding would add an unreviewed dependency graph before the ruleset shape has stabilised, and the generators are unit tested independently of the binary.
The protected destination list is the ADR 0012 and threat-model set plus the cloud metadata endpoints named in `RESOURCES.md`: AWS `169.254.169.254`, `169.254.169.253`, `169.254.169.123`, `169.254.170.2`, and `fd00:ec2::254`; Google `169.254.169.254`; Azure `169.254.169.254` and `168.63.129.16`; together with `0.0.0.0/8`, RFC 1918, `100.64.0.0/10`, loopback, `169.254.0.0/16`, `192.0.0.0/24`, `198.18.0.0/15`, multicast, `240.0.0.0/4`, broadcast, `::/128`, `::1/128`, `::ffff:0:0/96`, `fc00::/7`, `fe80::/10`, `ff00::/8`, every lease and transit plan, every host address, and every control-plane prefix an operator adds.
Google's IPv6 metadata address is covered by the ULA block rather than listed as a separate literal because the exact literal was not verified from a primary source during this slice.
A denied DNS policy delivers the gateway as the launch-page resolver so the guest never learns an operator resolver, and the ruleset drops every port 53 packet regardless.
The live container run proved a down guest link and no forwarding before activation, gateway ARP and ICMP plus public TCP egress and declared DNS after activation, metadata, undeclared DNS, host, peer guest, and peer gateway drops in `PublicInternet` mode, complete release, and a clean 100-way prepare, assign, activate, release burst; the retained result is `docs/evidence/2026-08-29-linux-network-profile-live.md`.
The conntrack zone is bound with `ct original zone set` on the host veth so masqueraded replies still match, and the sandbox namespace's own conntrack table is the primary per-bundle isolation.
The daemon skeleton does not authenticate its peer, ingress ports are reserved but never forwarded, proxy attachment is typed `Unimplemented`, and the broker's socket mechanisms have not been exercised from a jailed VMM; those are the next dependencies together with the virtio-net attach of the transferred TAP.

## 2026-08-29 - Recent implementation audit requires corrective work

The reviewed x86_64 KVM boot, launch-page identity, static guest-agent, Generation compiler, and Template-binding series has an agent-ready handoff in `docs/reviews/2026-08-29-implementation-audit.md`.
The PVH kernel boot, modularity, OCI normalization, and guest protocol are strong foundations.
Priority 0 corrections cover published responder authority, unbounded guest-output queues, incomplete Generation tool deadlines, premature Generation publication, and incomplete hostile manifest validation.
Priority 1 corrections cover entropy crediting, target-specific ioctl layouts, unusable subnet addresses, executable provenance, and structured workload commands.
Priority 2 work completes Host launch-page retirement, real virtio integration, real guest-agent readiness, Generation certification, narrow Template claims, and eventual performance evidence.
The full portable validation profile passed on macOS, but Linux-only and ignored live tests remain outside that evidence.

## 2026-08-29 - Canonical sandbox stack separates containment from dependency

The beginner documentation now distinguishes physical containment, dependency direction, runtime primitives, code modules, Template modules, optional capabilities, and optimizations.
The canonical stack reads from physical hardware through Host Linux, KVM, the VMM, virtual hardware, guest Linux, `soma-guest`, and the user workload.
Separate filesystem, command, network, and Generation traces show what connects each layer and why.
A capability activation rule makes optional networking, egress, ingress, storage, PTY, and checkpointing acquire mandatory isolation, policy, cleanup, and evidence requirements when enabled.
Tickets remain implementation and verification slices converging on one production sandbox architecture rather than separate sandbox products.

## 2026-08-29 - Template implementation uses four separate planes

The complete Template flow separates authoring, placement, Host Launch, and maintenance.
Authoring resolves modules, authorizes capability ceilings, locks exact inputs, builds, certifies, and publishes an immutable Generation.
Placement resolves a Template revision to one ready Generation before selecting a compatible Host.
Host Launch receives exact immutable identity and effective policy, then delivers fresh environment, secret, upload, workspace, and network inputs only after a unique authenticated Instance exists.
Maintenance owns revocation, retention, leases, distribution, and garbage collection outside Launch.
Five deep interfaces own Template compilation, Generation building, launch-input delivery, policy narrowing, and agent sessions without introducing agent-brand behavior into the VMM.
The dependency-ordered implementation map contains eighteen vertical slices and explicitly updates VMM decision-map tickets #6, #8, #10, #13, and #15 rather than duplicating them.

## 2026-08-29 - Templates compose into immutable Generations

A SOMA Template is a user-facing preparation recipe rather than a running sandbox or a VMM input.
The Template composes a base workload with focused agent, tool, workspace, environment, secret, network, lifecycle, and resource modules.
Resolution produces a canonical Template Lock with exact input identities, then offline construction and certification produce an immutable Generation.
Launch accepts the Generation plus fresh Instance inputs and never installs runtimes, resolves mutable image tags, or runs package managers.
Flat ordered composition is preferred over nested inheritance so the resolved result is inspectable and module conflicts fail explicitly.
Template network permissions are a maximum envelope that Launch may narrow but may not silently widen.
Secret references never place reusable values in a Template, lock, Generation, snapshot, log, or receipt.
Agent modules for Claude Code, Codex, OSA, Hermes, and future agents use one common contract and are convenience modules rather than privileged VMM behavior.

## 2026-08-29 - Static Linux guest agent for the repair state machine

Decision-map ticket #8 now has a guest side: `crates/soma-guest-agent` is a statically linked x86_64 musl executable that runs as PID 1 from the Generation initramfs.
It performs the compiler's early-init sequence itself, waits at the disconnected repair point, and drives the exact `Captured`, `MaterialAccepted`, `EntropyRepaired`, `TransportFresh`, `IdentityRepaired`, `NetworkRepaired`, `Authenticated`, `Probed`, `Ready`, `Running`, `Stopping`, and `Poisoned` order through a typestated controller with a runtime ledger.
Every protocol byte comes from `soma-guest`; the agent adds only Linux mechanisms behind narrow `libc` calls with `SAFETY` comments.
Three machine-contract decisions are now constants in `soma-guest`: the launch page lives in a dedicated slot at guest-physical `0xd0100000` above RAM and the MMIO window, the control endpoint is vsock port `0x534f4d41` on host CID 2 with the guest connecting, and launch-page schema 2 carries the non-secret vsock CID, network generation, MAC, IPv4 identity, resolver, time sample, and a BLAKE2s digest as accepted by ADR 0023.
The guest reads the page by mapping `/dev/mem` because the x86 `read` path rejects addresses above `high_memory` while `mmap` does not; the kernel therefore needs `CONFIG_DEVMEM=y`, may keep `CONFIG_STRICT_DEVMEM` because the page is not System RAM, and must not enable `CONFIG_IO_STRICT_DEVMEM` for that range, plus `CONFIG_VSOCKETS`, `CONFIG_VIRTIO_VSOCKETS`, `CONFIG_HW_RANDOM_VIRTIO`, EROFS, ext4, OverlayFS, devtmpfs, procfs, sysfs, and tmpfs.
`pivot_root` cannot leave the initial ramfs, so the agent moves the composed OverlayFS over `/` and enters it with `chroot` exactly as `switch_root` does after moving `/dev`, `/proc`, and `/sys`.
The Generation responder private key is read from `/etc/soma/responder.key` in the initramfs, then overwritten and unlinked before the root switch.
Version 1 commands run as root with a fixed environment allowlist, `/` as working directory, and closed standard input because the wire contract carries no environment fields; a caller-controlled environment needs a new wire version and an ADR.
`scripts/build-guest-agent.sh` produced a 815,288-byte binary with SHA-256 `e6367c5774aab8926f13ef9c9f952bcdb5a1e7d347d0e4cdfae851e94e1e3eb1` that `file` reports as statically linked; the digest is a local measurement and not yet a reproducible-build claim.
Host tests prove the state machine, page consumption and erasure, output accounting, invocation bounds, transport deadlines, kernel structure layouts, and the executor against host binaries, but the agent has not booted inside a SOMA virtual machine.
The next dependency is the VMM side: the launch-page memory slot, the vsock device on the fixed port, and the initramfs writer that places `/init` and the responder key.

## 2026-08-29 - Five virtio device models and the fixed MMIO bus are host-tested seams, not a working machine

`soma-kvm/src/virtio/devices/` implements the block, network, vsock, and entropy models from the minimal device surface behind the existing `VirtioDevice` seam, and `virtio/bus.rs` dispatches the five fixed transports at `0xd0000000` plus `slot * 0x1000` with GSIs 5 through 9, all as pure, `unsafe`-free Rust with 53 new host-side tests against in-memory guest RAM.
Every model is one parser over an already validated descriptor chain, one backend seam that accepts only validated operations, and one fixed little-endian identity record with a version byte, so no guest address, length, sector, port, CID, or feature bit reaches a host call before device-specific validation succeeds.
The shared `service_queue` loop pops at most a host budget of chains per notification, reports used lengths no larger than the validated writable capacity, skips a hostile chain with one counter tick, and sets `DEVICE_NEEDS_RESET` only when validated guest memory turns inconsistent or a backend becomes unusable.
The block parser accepts `IN`, `OUT`, `FLUSH`, and `GET_ID`, requires exactly sixteen readable header bytes and one trailing writable status byte, rejects direction mismatch, sector multiplication overflow, offset plus length overflow, non-sector-multiple or empty data, capacity overrun, and short host I/O, answers a write to the read-only root with `IOERR` before touching the backend as the specification requires, and answers a flush on a device that did not offer `VIRTIO_BLK_F_FLUSH` with `UNSUPP`.
The aggregate request limit is 1 MiB plus header and status, and the file backend uses positional `read_exact_at` and `write_all_at` on an owned `File` with the capacity rounded down to whole sectors at open time.
The network model offers only `VIRTIO_NET_F_MAC`, requires every byte of the 12-byte header to be zero on transmit, bounds frames to 14 through 1514 bytes, and drops everything while the host-controlled link is down; receive checks that the driver posted a buffer before reading from the backend and reads into a buffer one byte larger than the maximum frame so a TAP read that silently cuts an oversized frame is observed rather than delivered.
The vsock model accepts one stream connection at a time on the fixed control port `0x534f_4d41`, the same value the merged guest agent publishes as `soma_guest::CONTROL_VSOCK_PORT`; `soma-kvm` restates the literal as `SOMA_CONTROL_PORT` with a test rather than adding a crate dependency, so the two constants are one machine-contract field that must change together.
It validates type, operation, both CIDs, declared length against the readable payload, a 64 KiB payload bound, payload-free control operations, and flags only on `SHUTDOWN`, answers a `REQUEST` to any other port or a non-`RST` packet for an unknown or stale connection with `RST`, and resets the connection on impossible credit accounting or a window overrun.
Credit counters advance modulo 2^32 as the specification defines, while every derived quantity is a checked subtraction guarded by an impossibility check, so a hostile `fwd_cnt` or `buf_alloc` cannot authorize bytes beyond what either side buffers.
The host side of the connection is `HostEndpoint`, a bounded byte stream with 64 KiB in each direction that the future control owner reads, writes, and shuts down; the vsock identity record carries only the CID placeholder, a test proves the bytes are identical with and without a live connection, and restore clears every connection and queues one `TRANSPORT_RESET` event.
The entropy model fills each writable chain up to 64 KiB from `/dev/urandom` through an owned `File`, never retains or logs bytes, and turns a host entropy failure into a typed `DeviceFault::Backend` that stops the device.
The bus derives every address, interrupt, identifier, queue count, and the kernel command-line fragment from one slot table, and a test asserts the fragment equals the device-surface string exactly.
Each model's `snapshot_state` is a small versioned identity record, and the separately merged `snapshot/device_state` container carries its own `DeviceSpecific` fields for the same five devices; mapping the live bus onto that container is the next integration step and has not been written.
`IrqSink` and `NotifySource` are the seams the `x86_64` machine will implement with irqfd and ioeventfd; nothing here registers an eventfd, decodes a KVM exit, runs an event loop, owns the snapshot container, or has run against a real guest, and the receive paths have been exercised only with in-memory backends plus a pipe for the TAP wrapper.

## 2026-08-29 - x86_64 halt guest proves the KVM machine floor

The first x86_64 code in `soma-kvm` creates one VM, one 128 MiB private memory slot, and one protected-mode vCPU, then captures port-I/O exits and `KVM_EXIT_HLT` on a real Ubuntu 24.04 host.
It deliberately does not create the in-kernel interrupt controller for the halt proof, because with `KVM_CREATE_IRQCHIP` the kernel emulates `hlt` by parking the vCPU and never reports `KVM_EXIT_HLT` to userspace.
The same proof run with the in-kernel controller therefore ends only through the watchdog, and that path is retained as a second ignored test that proves deadline enforcement and descriptor cleanup.
The watchdog reuses the KVM signal-mask technique from the ARM64 proof: the vCPU thread blocks one real-time signal everywhere except inside `KVM_RUN`, so a kick is never lost between iterations.
The PVH `hvm_start_info`, memory map, and diagnostic command line are written at their contract addresses even though the raw guest ignores them, so the layout encoding is exercised before the kernel slice.
The Docker backend now derives its OCI platform from the host architecture instead of assuming `linux/arm64`, and its macOS-only availability helper is target-gated so the workspace compiles on Linux under `-D warnings`.
The retained result is in `docs/evidence/2026-08-29-x86_64-kvm-halt-guest.md` and proves the machine floor only, not a kernel boot, device, sandbox, or latency claim.

## 2026-08-29 - Virtio transport and split queues are hostile-input seams, not devices

`soma-kvm/src/virtio/` implements the modern virtio-mmio version 2 register file and split virtqueues from the minimal device surface as pure, `unsafe`-free, target-independent Rust with 43 host-side tests.
The transport models `read(offset, width)` and `write(offset, width, value, mem)` over one 4 KiB page, and every rejection is a typed violation that is also recorded in a bounded saturating counter that never carries guest bytes.
Status writes are accepted only one new bit at a time in `ACKNOWLEDGE`, `DRIVER`, `FEATURES_OK`, `DRIVER_OK` order, a driver can never clear a bit except by writing zero, and writing zero resets the device, queues, features, selection, and interrupt status.
`FEATURES_OK` stays clear when the driver accepts any bit outside the device allowlist or omits `VIRTIO_F_VERSION_1`, so a modern driver observes the failure on read-back exactly as the specification requires.
Queue geometry is validated at `QueueReady=1` for a power-of-two size within the device maximum, 16-, 2-, and 4-byte alignment, containment of the descriptor table, available ring, and used ring inside registered memory, and pairwise ring disjointness, which is stricter than the specification but costs nothing.
A queue may be activated once per reset, queue configuration is locked after `DRIVER_OK`, and `QueueNotify` returns the bounded queue index only when the device is active and that queue is ready.
`walk_chain` is a pure function over guest memory, a table address, a queue size, a head, and host limits so a later cargo-fuzz target is one line; it rejects out-of-range indexes, repeated indexes via a visited bitmap, chains longer than the limits, indirect and unknown flags, zero-length descriptors, address overflow, unregistered bytes, readable-after-writable order, and aggregate bytes over the limit.
Zero-length descriptors are rejected deliberately so every accepted segment is a nonempty bounded range; Linux drivers never emit them, and a future device that needs them must argue for it.
On a chain violation the available cursor still advances so a hostile head cannot pin the queue, and the device decides between reporting it used with length zero and setting `DEVICE_NEEDS_RESET`.
`add_used` refuses a length above the chain's validated writable capacity rather than clamping, because a device that overstates a length is a device bug that must fail loudly.
Event-index suppression is not negotiated, so only `VIRTQ_AVAIL_F_NO_INTERRUPT` is honored, and the queue issues acquire and release fences around the available-index read and used-index write so the same code stays correct over mapped guest memory later.
`QueueState` and `TransportState` are fixed little-endian records with exact-length decoding, and restore revalidates status order, allowlisted features, interrupt bits, queue count, queue geometry against live memory, cursor consistency, and device activation before any state becomes visible.
`InterruptACK` clears exactly the acknowledged known bits in one store; the atomicity claim rests on single-thread ownership of the transport, which the future event loop must preserve.
Nothing here is an MMIO bus, ioeventfd, irqfd, device backend, event loop, snapshot container, or sandbox, and the tests prove transport and queue behavior only against in-memory guest RAM.

## 2026-08-29 - Pinned x86_64 PVH guest kernel builds reproducibly

Decision-map ticket #4 now has its kernel input: Linux `v6.12.107` built as an uncompressed ELF `vmlinux` with `XEN_ELFNOTE_PHYS32_ENTRY` at `0x01000000`, `CONFIG_RELOCATABLE=n`, no modules, no PCI, no ACPI, and only the five virtio-mmio device drivers plus EROFS, ext4, OverlayFS, and the pseudo filesystems.
`kernel/build.sh` pins the source tarball by SHA-256, compiles inside an Ubuntu 24.04 image pinned by digest with verified gcc 13.3.0 and binutils 2.42, fixes every `KBUILD_*` and `SOURCE_DATE_EPOCH` value, fails closed if `make olddefconfig` changes any pinned symbol, and records a manifest with every digest.
`kernel/verify-pvh.py` parses the ELF with the standard library only and rejects a missing note, a segment below the contract floor, overlapping segments, or an entry outside executable loaded bytes.
Two consecutive builds on the same host produced byte-identical output; the evidence is in `docs/evidence/2026-08-29-x86_64-pvh-kernel-build.md`.
This is a build and layout proof only, not KVM boot evidence, device discovery evidence, or a Generation.
A first build with `CONFIG_DEVMEM=n` was superseded the same day because the guest agent reads the launch page through `/dev/mem`; the retained evidence records both digests.

## 2026-08-29 - Snapshot format v1 codec and ordering contracts

Decision-map ticket #7 now has an implemented codec half under `crates/soma-kvm/src/snapshot/`.
It encodes and decodes the `SOMASNP\0` schema v1 manifest, bounded digest-covered sections, SOMA-owned byte layouts for every x86_64 KVM state group, the five device states, and the memory-object descriptor, with checked conversions to and from `kvm-bindings` on Linux x86_64.
The compatibility check compares a host profile with a manifest by exact equality and returns one typed rejection reason per field, header fields before any section payload.
Tests cover golden header bytes and a pinned whole-manifest digest, every single-byte flip and every prefix length of a full manifest, unknown critical and non-critical roles, absurd lengths, round trips of every state group, per-field compatibility rejection, and private-mapping divergence between two mappings of one file on Linux.
This is a codec and ordering contract only.
Nothing here opens `/dev/kvm`, captures a live machine, restores one, maps guest memory into a VM, or proves restore latency, and `capture.rs` and `restore.rs` are typed step orders rather than implementations.
The crate compiled and passed its gates on Linux x86_64 only; macOS and Windows client compilation of the new module was not exercised in this slice.

## 2026-08-29 - Complete custom VMM architecture map

Decision-map tickets #1 through #15 now have linked architecture assets.
The remaining implementation order is virtio, EROFS and OverlayFS boot, authenticated guest repair, private snapshot restore, VMM jail, Linux networking, reflink storage, prepared workers, complete backend wiring, production admission, and fleet scaling.
Architecture resolution is not implementation evidence.
Linux prototypes, hostile tests, end-to-end lifecycle results, raw latency samples, and signed HostProfile admission remain required by each document's gates.
The implementation roadmap gives coding agents the dependency order and a uniform handoff contract without weakening the portable lifecycle.

## 2026-08-29 - Generation v1 uses immutable EROFS plus private ext4

Decision-map ticket #6 selects a deterministic EROFS image as the immutable OCI-derived root and a separate Instance-private ext4 filesystem as the OverlayFS upper and work storage.
The offline Generation compiler binds the kernel, initramfs, guest agent, both filesystem contracts, machine and device contracts, CPU template, command line, guest protocols, snapshot state, repair policy, and exact builder provenance into one canonical `GenerationId` manifest.
A retained Docker prototype built erofs-utils 1.9.4 at commit `f36cadb5c563995ab3aa8572a60ed6b721b9557d` and proved byte-identical fixture images across opposite host insertion orders.
An ext4 population experiment changed bytes across build seconds because host inode change time leaked into the image, so populated ext4 is rejected as the immutable reproducible root.
The five-device correction keeps Generation bytes immutable and independently reproducible while allowing writable disk capacity to remain an Instance shape selected from certified preformatted overlay classes.

## 2026-08-29 - Minimal device surface uses fixed modern virtio-mmio

Decision-map tickets #5 and #6 select exactly five virtio-mmio version 2 devices for machine contract v1: an immutable EROFS root block device, a private ext4 overlay block device, network, vsock control, and entropy.
Each device has one fixed 4 KiB MMIO page above the 3 GiB RAM ceiling, one dedicated GSI, bounded split queues, and an explicit feature allowlist.
PCI, legacy virtio, hotplug, vhost, packed queues, optional offloads, and separate control or shutdown devices remain outside version 1.
Queue and device state are hostile input, transient I/O and authority never enter a snapshot, and restore attaches fresh disk, TAP, vsock, and entropy resources before vCPU resume.

## 2026-08-29 - x86_64 machine contract v1 uses PVH direct boot

Decision-map ticket #4 selects a pinned uncompressed Linux ELF kernel carrying `XEN_ELFNOTE_PHYS32_ENTRY`.
The first contract enters one bootstrap vCPU in 32-bit protected mode through PVH, uses a fixed low-memory boot layout, and excludes BIOS, UEFI, ACPI, PCI, and general PC emulation.
Snapshot compatibility binds the kernel, command line, CPU template, KVM and host profile, device state, and all immutable artifact digests.
The cold-boot proof is diagnostic evidence only and cannot be presented as a working OCI sandbox or 10 ms restore result.

## 2026-08-29 - Custom VMM research is sequenced by a decision map

The custom VMM work now uses `docs/research/vmm-decision-map.md` as its canonical dependency graph.
Resolved architecture decisions remain linked to their ADRs and architecture assets, while unresolved Linux work is split into focused research or prototype tickets.
The frontier begins with the exact x86_64 machine contract, minimal device surface, and deterministic Generation compiler before snapshot, guest integration, network, disk, allocator, backend wiring, and fleet work.

## 2026-08-29 - Docker is the first local development backend

The first working local SOMA lifecycle uses Docker Desktop's Linux ARM64 engine on macOS.
It creates a constrained container with a read-only root, dropped capabilities, no-new-privileges, a PID limit, bounded command execution, and disabled networking by default.
This is a Linux-container boundary inside Docker's utility VM, not the per-sandbox hardware VM targeted by the future custom Rust VMM on Linux.
Five live `node:22` one-shot runs returned `v22.23.2` and complete cleanup, with approximately 1.19 to 1.24 seconds end to end on the development Mac.

## 2026-08-28 - Project foundation

SOMA expands to Secure Optimized Machine Architecture.
The public brand is SOMA by MIOSA and the public repository name is `SOMA`.
The repository is open source under Apache License 2.0.
The initial production target is Ubuntu 24.04 x86_64 on bare-metal KVM hosts.
The development machine is Apple Silicon macOS, so local results cannot certify Linux KVM behavior.
The host control plane remains outside SOMA and communicates with a native external process.
The BEAM must never own live KVM state through a NIF.
One VMM process owns one sandbox, with one dedicated OS thread for each vCPU.
Arbitrary OCI support means the image pipeline converts a root filesystem into certified artifacts before the launch path.
SOMA does not pull or build OCI images inside the VMM.
The benchmark readiness boundary is a successful authenticated first command after clone repair.
Process start, memory mapping, snapshot load, vCPU resume, console output, and agent connection are intermediate milestones rather than readiness.
Architecture dominates launch latency, while Rust is selected for memory safety, ecosystem maturity, and production VMM precedent rather than faster KVM syscalls.

## 2026-08-28 - Host interface decision

Three external interface shapes were compared before implementation: a direct per-machine command interface, a declarative host reconciler, and a daemon-owned live handle.
The initial interface uses `Launch`, `Execute`, and `Stop` against one per-machine `soma-vmm` process.
This interface keeps restore, identity repair, authenticated readiness, idempotency, and cleanup local to one deep module without introducing a speculative host daemon.
The declarative reconciler is deferred until SOMA has a real host-wide lifecycle responsibility that cannot remain in the operator.
The daemon-owned live handle is deferred because it would add another process and lease authority to the latency-sensitive path before those responsibilities are required.
The public contract remains provider-neutral, and MIOSA-specific admission, placement, billing, and fleet policy stay outside this repository.

## 2026-08-28 - Release line

SOMA follows Semantic Versioning and targets `1.0.0` as its first stable release.
Until the real Linux KVM path passes end-to-end `Launch`, `Execute`, and `Stop` gates, source versions use `1.0.0-alpha.N` or `1.0.0-rc.N` rather than presenting scaffolding as stable.
After `1.0.0`, backward-compatible fixes increment the patch version, backward-compatible capabilities increment the minor version, and incompatible public-contract changes increment the major version.
Stored Generation, guest-agent, snapshot, and wire artifacts retain explicit format versions separate from the product release.

## 2026-08-28 - Brand assets

The repository uses the official MIOSA orb and black-text and white-text wordmarks supplied by the project owner.
The committed PNGs preserve the supplied artwork and trim only transparent outer padding.
SOMA uses an endorsed-brand layout with the MIOSA wordmark above the SOMA product name instead of inventing an unrelated symbol.

## 2026-08-28 - Portability north star

SOMA's long-term product goal is the state-of-the-art hardware-isolated sandbox engine across clouds, bare-metal operators, workload images, machine shapes, and storage sizes.
The first supported substrate remains Ubuntu 24.04 x86_64 KVM so correctness, security, and performance can be proven against one exact host contract.
Public types must not hardcode a provider, product tier, vCPU count, memory size, or disk size.
Each additional architecture, kernel family, filesystem, and cloud substrate must pass the same conformance, isolation, cleanup, and benchmark contracts before it is described as supported.

## 2026-08-28 - Phase 0 semantic alignment

`InstanceId` identifies one globally unique concrete Machine lifetime and must never be reused for another lifetime.
Stable caller resource identity remains an operator concern outside the per-Machine VMM interface.
Phase 0 idempotency compares complete in-process Rust request values structurally because no canonical wire encoding or request fingerprint exists yet.
Terminal Launch, Execute, and successful Stop outcomes replay exactly without repeating their side effects.
An admitted Stop with incomplete cleanup remains in the Reaping state, and replaying that exact Stop is the only Phase 0 path that repeats work under the same `OperationId`.
The Phase 0 Ready receipt contains the operation, Instance, Generation, the Generation's exact `MachineSpec`, and ordered milestones.
The Phase 0 output allowance limits logical bytes retained after an adapter returns rather than proving bounded guest-output ingress.

## 2026-08-28 - Performance admission

The first stable release must establish performance leadership as an admission property rather than a later optimization.
The certified warm-host targets are prepared-worker acquisition and dispatch below 0.10 ms p50 and 0.50 ms p99, server-side create below 5 ms p50 and 10 ms p99, and first bounded command below 10 ms p50 and 20 ms p99 from accepted Launch.
The exact 100-way ComputeSDK Burst TTI target is below 50 ms median and 90 ms p99 with 100 successful commands and cleanups.
Prepared workers may move invariant process, descriptor, allocation, and jail work outside Launch, but they cannot carry tenant identity, writable guest state, or reusable authenticated authority.
Every result must identify whether it used on-demand restore, a prepared worker, a paused pool, or a ready pool.
The initial component targets are additive only when expressed through the ADR 0006 critical-path budget, whose experimental totals are 3.25 ms p50 and 8.90 ms p99.
The exact external latency target applies to a recorded same-region route with persistent connection state and certified pre-reserved capacity rather than every global network path.
Tail engineering requires at least 100 bursts and 10,000 samples even though the authoritative external cohort contains 100 samples.

## 2026-08-28 - macOS VM development backend

Apple Silicon development uses an explicitly development-only adapter to Apple's `container` 1.3 command contract.
The adapter provides one Virtualization.framework Linux VM per OCI container for local run, create, start, exec, stop, delete, and inspect conformance.
The unprivileged bootstrap pins the signed package by SHA-256, verifies the Apple package signature, and uses explicit user-owned install, state, and log roots.
The verified local image matrix includes `node:22`, `ubuntu:24.04`, `python:3.12-slim`, and `kalilinux/kali-rolling` on Linux ARM64.
This evidence proves a real local VM lifecycle but does not satisfy any Ubuntu x86_64 KVM, restore, security-jail, density, or performance gate.

## 2026-08-28 - Prepared host allocation

ADR 0006 introduces a small node-local allocator because reliable sub-5 ms creation cannot start process, jail, network, storage, and allocator state from zero on every request.
The allocator owns only unassigned single-use workers, sterile resource bundles, immutable Generation handles, host admission, and asynchronous replenishment.
One assigned VMM still owns exactly one Machine, and an assigned worker is destroyed instead of being scrubbed for another tenant.
The current critical-path budget is additive at 3.25 ms p50 and 8.90 ms p99 and remains an experimental target rather than a measured claim.

## 2026-08-28 - Portable client and use-case surface

SOMA separates portable caller semantics from local isolation-engine support.
The library and command-line interface target Linux, macOS, and Windows, while local KVM, Apple virtualization, and future backends remain capability-gated.
An explicitly configured remote backend will provide the same bounded use cases on clients without a certified local engine.
Unsupported local execution fails closed and never degrades to a host process, shared Docker VM, or namespace-only sandbox.
Linux OCI images are the first workload format, while arbitrary non-Linux guest operating systems are outside the first stable release.

The public library is organized around one-shot execution, managed Machine lifecycle, and remote execution rather than hypervisor mechanisms.
Future evaluation branching, browser sessions, CI, GPU, confidential computing, and nested workloads extend those use cases through explicit capabilities.
Only modules with real depth become crates, and generic utility or manager dumping grounds remain prohibited.

## 2026-08-28 - Evidence-carrying execution receipts

Every terminal use case will produce one versioned receipt covering exact workload identity, effective isolation and preparation classes, effective shape, request fingerprint, monotonic milestones, command outcome, measurement boundary, and cleanup state.
Receipt construction is portable product logic rather than backend-specific rendering.
A basic receipt is structured host evidence and must not be described as cryptographic attestation.
Signed and hardware-attested profiles require explicit trust, canonical encoding, rotation, and verification decisions.

## 2026-08-28 - Competitive research ledger

`COMPETITORS.md` records dated primary-source facts, external benchmark observations, vendor claims, unknowns, transferable insights, and pitfalls separately.
RunPod is included as a GPU, serverless worker, image-template, and persistent-volume reference rather than being forced into the 1 vCPU ComputeSDK table.
The ledger distinguishes Tenki by Luxor from Tencent CubeSandbox and distinguishes Tencent CubeHypervisor from Ant Group's Dragonball VMM.

## 2026-08-28 - Machine customization contract

Every run and managed launch accepts a provider-neutral requested shape with a nonzero `u16` vCPU count and nonzero `u64` memory and writable-storage values in MiB.
The portable defaults are 1 vCPU, 1024 MiB of memory, and 10240 MiB of writable storage.
Actual host capacity is backend admission rather than a smaller provider-specific limit in the public type.
Receipts distinguish every requested dimension from independently verified effective evidence.
An optional lowercase human-readable Machine name is metadata only and never replaces the globally unique Instance ID.
Changing image, vCPU, memory, storage, network policy, or immutable startup input creates a replacement Instance rather than mutating a shared Machine.
OCI layers and certified Generations are the reproducible incremental-customization path, while persistent workspace data remains a separately owned storage contract.

Network intent uses an explicit unspecified, denied, or allowed policy because an unavailable observation cannot truthfully satisfy a security restriction.
Apple Container 1.3 attaches its default NAT network when no policy is supplied and supports a verified no-network path through `--network none`.

## 2026-08-28 - Durable managed lifecycle

Managed Machine state must survive independent CLI invocations and MCP server restarts.
ADR 0010 therefore requires a bounded versioned durable record, create-if-absent, revisioned compare-and-swap, write-ahead lifecycle states, corruption failure, and safe replay behavior.
An uncertain command is never silently repeated after a crash.
The shared `soma-local` crate accepted in ADR 0011 owns the cross-platform file store and target-gated local adapters so CLI and MCP use one facade-backed implementation.

## 2026-08-28 - Public repository identity

The public GitHub repository is named `SOMA` at `Miosa-osa/SOMA`.
Rust crates, binaries, shell commands, and source paths retain lowercase `soma` where required by platform convention.

## 2026-08-28 - Deployment portability

SOMA separates portable callers from capability-gated engine hosts so one use-case and receipt contract can span local, cloud, and on-premises placement.
Engine support attaches to an exact certified host profile rather than a provider logo or generic virtual-machine product name.
The initial production profile remains Ubuntu 24.04 x86_64 KVM, with public-cloud bare-metal and nested-virtualization profiles admitted only after retained conformance, isolation, cleanup, and performance evidence.
Managed function environments such as AWS Lambda are client-only locations that may call a future authenticated remote SOMA engine.
They are not treated as local VMM hosts and cannot trigger a silent weaker fallback.

## 2026-08-28 - Public alpha benchmark gate

The repository will not be published until the real Apple development backend has repeated retained boot-to-command measurements across multiple images, shapes, network policies, lifecycle modes, cache states, and concurrency levels.
The matrix must include CLI and MCP callers, failures, exact timer boundaries, success rates, and cleanup evidence.
Apple results remain development evidence and must not be cited as Ubuntu KVM, production restore, or ComputeSDK-comparable performance.
The production KVM release gate remains the larger corpus and exact external benchmark contract in `docs/benchmark-contract.md`.

## 2026-08-28 - Core proof before managed integration

Real SOMA sandbox behavior is the immediate release priority and must be proven before control-plane or cloud deployment templates are expanded.
The local proof must exercise real OCI images through one-shot and managed lifecycles, resource shapes, network policies, adverse command outcomes, durable state, cleanup, CLI, and MCP.
The future MIOSA profile represents a managed SOMA service reached through MIOSA authentication and must not imply that an unreleased integration exists today.
Public documentation should describe the stable integration contract and intended operator experience without exposing private platform status or internal repositories.
Launch, inspect, stop, and destroy do not accept caller timeout fields until the facade can honor them without interrupting required cleanup.
Those control operations use a bounded engine-profile policy, while one-shot run and managed execute retain caller-supplied execution limits.

## 2026-08-28 - Fail-closed networking architecture

ADR 0012 separates portable `NetworkPolicy` intent, operator-owned profiles, live `EffectiveNetwork` evidence, and resource-by-resource cleanup evidence.
The portable default denies egress and DNS and publishes no host ports.
Ingress remains unreachable until an authenticated guest readiness result activates an already-reserved publication.
`PublicInternet` denies private and protected destinations, while explicit `Unrestricted` still cannot bypass host, peer, control-plane, or metadata protections.
DNS is independent from attachment and egress, and unavailable DNS evidence cannot satisfy an explicit denial or resolver request.
Every IPv6 host bind carries an explicit `v6_only` value so behavior never depends on an operating-system default.
Operators can define named versioned network and proxy profiles with address pools, resolvers, protected routes, ingress pools, proxies, and custom adapters without putting secrets or raw firewall input in Machine requests.
Custom host implementations use the bounded `acquire`, `activate`, `inspect`, `release`, and `reconcile` network-runtime seam.

Local Apple Container 1.3 probes proved that `--network none` detached networking and that the default network attached NAT egress.
Explicit custom DNS worked on the tested host, while runtime-default DNS timed out.
Apple Container `--no-dns` only declined to configure DNS, and the tested `node:22` image retained resolver configuration that still resolved names.
Apple Container rejected host port `0` and port `1`, staged fixed publications at create time, and did not bind the host endpoint until start.
Two Machines could stage the same fixed host port, with the collision reported only when the second Machine started.
A detached Apple network combined with a publication produced no host listener, so SOMA must reject that combination.
Apple automatic-port activation therefore uses bounded reservation, release, start, inspection, and occupancy verification and is labeled `VerifiedRuntimeRebind` because a race remains.

The initial Linux production design uses a narrow privileged `soma-netd` broker reached through a typed filesystem-protected Unix `SOCK_SEQPACKET` protocol.
The broker owns durable leases, per-Machine network namespaces, TAP and veth devices, IPAM, conntrack zones, nftables sets and maps, DNS policy, port reservations, ingress activation, and reconciliation.
The unprivileged VMM receives only its already-open TAP file descriptor through `SCM_RIGHTS` and never receives `CAP_NET_ADMIN`.
Real Ubuntu 24.04 x86_64 conformance must prove policy, readiness-gated ingress, anti-spoofing, protected destinations, crash recovery, reconciliation, and complete cleanup before production networking is claimed.

## 2026-08-28 - Local ARM64 nested KVM development profile

Apple Container 1.3.0 on the tested M3 Ultra host can expose nested virtualization when given a KVM-enabled ARM64 Linux kernel.
This follows Apple's documented `container run --virtualization --kernel` development path and does not rely on Docker Desktop exposing `/dev/kvm`.
Docker Desktop 28.5.1 on the same host ran cached Ubuntu 24.04 as ARM64 but did not expose `/dev/kvm`.
An explicit Docker `--device /dev/kvm` request failed because the Docker daemon host had no such device.
The kernel was built from apple/containerization commit `2faaf9b4aff48a4745ef3d26c3f1450c1228fdf0`, which pins Linux 6.18.5 and enables `CONFIG_VIRTUALIZATION` and `CONFIG_KVM` for ARM64.
A cached Ubuntu 24.04 guest reported `aarch64`, exposed `/dev/kvm` as a character device, and initialized KVM in Hyp nVHE mode.
A second cached Python 3.12 guest opened `/dev/kvm`, reported KVM API version 12, reported an 8192-byte vCPU mapping, and successfully completed `KVM_CREATE_VM`.
The real `soma-kvm` public probe then passed inside a disposable `rust:1.98-bookworm` nested guest, including its ordinary ARM64 tests and the explicitly selected ignored live test.
The live Rust test completed in 0.02 seconds and proves only capability verification plus empty-VM creation and cleanup, not sandbox launch latency.
Both proof containers exited successfully and were removed automatically.
These checks prove a usable local ARM64 KVM development environment only.
They do not prove that SOMA can boot a guest, execute a command, restore a snapshot, isolate a workload, or meet a latency target.
The release profile remains Ubuntu 24.04 x86_64 KVM and requires separate retained certification evidence.

## 2026-08-28 - Cross-platform checkout and dependency policy

Repository text is forced to LF through `.gitattributes` so Windows checkout settings cannot invalidate the pinned rustfmt contract.
PNG brand assets are explicitly binary.
The dependency policy accepts the OSI-approved Unicode License v3 required by `unicode-ident`.
It records a narrow temporary duplicate-version exception for `syn` 2 because `tracing-attributes` has not yet converged on the `syn` 3 line used by the rest of the current macro graph.
The exception should be removed as soon as the dependency graph converges.

## 2026-08-28 - External benchmark build provenance

The local-alpha runner requires an absolute externally generated v2 build-manifest path and never invokes Cargo during measurement.
A separate controlled entry point runs only the locked release build for `soma-cli` and `soma-mcp`, then writes the manifest with create-exclusive semantics.
The builder removes only those two prior release outputs before Cargo so a false-success or failed build cannot be attributed to stale executables.
Dirty and non-Git checkouts, changed revisions, invalid destinations, failed builds, and missing replacement outputs fail closed before a manifest is published.

## 2026-08-28 - Release artifact integrity

Every public crate carries package-root copies of the repository `LICENSE` and `NOTICE` while retaining the SPDX `Apache-2.0` package metadata.
The release verifier compares those packaged files byte-for-byte with the repository root and rejects missing, changed, duplicated, or unexpectedly rooted entries.
Native client deliveries contain only one compressed tar archive and an outer checksum manifest so GitHub artifact transport cannot discard the executable modes stored by tar.
Each client archive has one target-specific root containing both binaries, `LICENSE`, `NOTICE`, build provenance, and an internal checksum manifest that covers every payload file except itself.
The outer checksum manifest covers the exact tar archive shipped to the artifact uploader.
Release packaging fails closed on unexpected archive structure, incomplete checksum coverage, changed legal files, or binaries without mode `0755`.

## 2026-08-28 - Evidence construction contracts

ADR 0015 makes the original inspection request the source of operation, instance, and workload identity in backend observations.
Network cleanup evidence uses named per-resource builders so the API avoids positional mistakes without losing the independent dispositions required by ADR 0012.

## 2026-08-28 - ARM64 KVM cold-boot proof

ADR 0014 advances the local nested ARM64 KVM profile from empty-VM creation to direct Linux boot with guest RAM registration, vCPU state, GICv3, an architectural timer, a generated device tree, an explicit initramfs, and transmit-only serial emulation.
The proof accepts explicit trusted fixture paths, observes only an unauthenticated serial sentinel, and cannot be described as OCI execution, sandbox readiness, snapshot restore, production cleanup, or a performance result.
The vCPU runs on a sole-owner thread with a fixed boot deadline and bounded cancellation grace.
Timeout containment blocks the reserved signal outside `KVM_RUN`, temporarily unmasks it through KVM's eight-byte signal-mask ioctl, delivers a targeted real-time kick, joins the vCPU thread, and aborts the dedicated VMM process if KVM cannot be contained before registered memory would be released.
The retained cold-boot evidence binds the tested SOMA revision, kernel, generated initramfs, nested runtime, timer boundary, sentinel result, forced timeout, and before-and-after descriptor counts.
It remains diagnostic and does not establish product support or a public performance claim.
The next honest VMM boundary is a bounded challenge-bound guest command proof, followed by Generation-bound guest identity and an authenticated production control channel.

## 2026-08-28 - ARM64 KVM challenge-bound command proof

ADR 0016 adds a test-only ARM64 KVM tracer bullet that boots a trusted static PID1 agent, waits for an exact Hello, sends one challenge-bound direct-exec request over a dedicated second 16550 UART, and accepts only strictly sequenced bounded output plus one typed terminal result.
The diagnostic console has authority only before Hello and stops retaining bytes after the handshake.
The workload never receives the control descriptor, no shell is invoked, and timeout or output containment kills and reaps the entire command process group.
The first live run failed deterministically because the pinned Apple Containerization ARM64 kernel allowed only one 8250 UART, so Linux could not register `/dev/ttyS1`.
A source-identical Linux 6.18.5 kernel with both `CONFIG_SERIAL_8250_NR_UARTS` and `CONFIG_SERIAL_8250_RUNTIME_UARTS` changed from 1 to 2 made the unchanged end-to-end command test pass.
The corrected kernel SHA-256 is `1f750d412c3632a57c8cd6abb76bda53314bff14be5bdca24ece2b649424d0a5`.
The final command fixture is rebuilt twice and compared byte-for-byte before live evidence is retained.
The live matrix covers exact and metacharacter-bearing arguments, delayed and binary output, exit and signal outcomes, child deadlines, closed standard streams, descendant cleanup, exact and exceeded aggregate output limits, a legal 64 KiB response, typed `execve` failure, repeated host descriptor and task cleanup, normal cold boot, and forced watchdog containment.
This remains a cold trusted-fixture proof and does not establish OCI execution, authenticated readiness, snapshot restore, production isolation, or a sandbox latency claim.

## 2026-08-28 - OCI import is not Generation certification

ADR 0018 creates a real independently tested `soma-generation` boundary for bounded import from an existing OCI image layout.
The importer verifies descriptor sizes and SHA-256 digests, selected manifest and configuration identity, ordered layers and expanded `diff_ids`, traversal and byte budgets, descriptor-relative no-follow access, and atomic immutable content-store publication.
Its import output is `ImportedOci`, never `GenerationId`.
The later normalization slice now produces `NormalizedRootfs`, while disk compilation, kernel and guest-agent selection, snapshot capture, compatibility certification, signatures, and launch remain later Generation stages.

Two Apple Container exports of the same cached `node:22` image produced identical selected manifest, configuration, and layer bytes but different synthesized traversal-index bytes because annotation map order changed.
Canonical imported identity therefore excludes export-only traversal indexes while retaining a caller-supplied registry index digest for an exact immutable selection.
The imported traversal index digests remain provenance evidence.
The integrated importer successfully consumed the real 381 MiB nested Apple `node:22` OCI layout and verified all eight compressed layers against their configuration `diff_ids`.
That import is an offline build-path check and is not sandbox launch, first-command, or latency evidence.
The importer now validates each expanded layer as a complete tar stream before any selected layer is published and records the logical entry count in its deterministic completion artifact.
Two independently exported layouts produced the same structurally validated import digest `sha256:7f054135dc1553375fb1e798b902f5580c745741d45c4d6f3088e08bbaac110e` in 27.14 and 27.46 seconds on the development Mac.
Those timings measure cold offline verification of 381 MiB, not Machine creation or command readiness.
The importer now runs a raw streaming preflight before `tar` 0.4.46, limiting GNU long-name and long-link records to 4,097 bytes and each local PAX record to 64 KiB before the complete parser can materialize it.
Local PAX and GNU naming bodies across selected layers share a 64 MiB import budget, and global PAX is rejected from its header before its body is read.
Import also caps all raw tar headers across selected layers at one million, aggregates logical entry and path metadata totals across those layers, and rejects GNU sparse entries before reading their bodies.
Logical entries and their path or link bytes are charged incrementally before validation advances into each entry body, so a later layer cannot consume another full per-layer allowance before aggregate rejection.

## 2026-08-28 - Private workspace crates stay out of public release bundles

Cargo can create a crate archive for a workspace member marked `publish = false`.
The release packager now validates the version of every member, runs one workspace-aware Cargo packaging operation that excludes private members, and copies only public archives using the same Cargo-metadata predicate as the verifier.
A clean temporary Git-workspace regression test proves that an intentionally unbuildable private crate is never packaged, that one public crate can depend on another unpublished-version workspace crate, and that the macOS Bash 3.2 clean-release path works.

## 2026-08-28 - Instance-bound authenticated guest session

ADR 0017 fixes the first authenticated guest-control profile to Noise `NKpsk0` with Curve25519, ChaChaPoly, and BLAKE2s.
The transcript binds exact Generation, Instance, operation, and launch-nonce bytes, while every PSK wrapper is separately scoped to the same Instance identity.
A focused Snow resolver rejects non-contributory X25519 exchanges during public-key admission and every handshake Diffie-Hellman operation.
Bounded encrypted records carry exact directional sequence and payload lengths, and the first peer-controlled rejection poisons both directions of the session.
The crate is only a portable protocol foundation because no guest executable, snapshot-safe secret injection, Repair sequence, or VMM transport integration exists yet.
Snow does not guarantee erasure of every internal key copy, so complete key erasure and production security are not claimed.

## 2026-08-28 - OCI portability and dependency exceptions

Native Windows cannot portably fsync a directory entry through the current capability library, so the OCI store claims synced staged bytes and atomic create-exclusive visibility there but not directory-entry crash durability.
Final OCI layout and store roots are opened without following their final component, while resolution above each ambiently opened parent remains an explicit trusted-parent boundary.
Cargo-deny permits the LLVM exception used by the current capability dependency graph.
Narrow duplicate-version exceptions cover Snow 0.10's older RustCrypto and getrandom lines plus cap-primitives 4's current io-lifetimes and Windows support graph.
Those exceptions remain dependency-specific and should be removed when their upstream graphs converge.
The OCI content store is a single-writer authority boundary because portable Rust cannot hard-link an already verified open handle directly into the final namespace.
Publication revalidates the destination and repairs its read-only attribute, while retained writable handles or an actor with competing store authority remain outside the guarantee.

## 2026-08-29 - Owned authenticated guest control is one fail-closed lifecycle

ADR 0020 defines a canonical 4,096-byte launch page, fixed bounded application messages, direct argument-vector commands, and exact output accounting.
Host launch material and guest session material are single-use owned states, while raw PSKs, handshakes, and encrypted sessions remain crate-private.
ADR 0021 composes the Noise handshake, byte transport, repair commit, fixed `/proc/self/exe --soma-ready-probe-v1` check, Execute exchanges, Shutdown, and poisoning behind `HostControl` and `GuestControl`.
Every operation identity is single-use within one session and a private ledger caps the session at 65,536 identities, preventing a late terminal from becoming a later result through identity reuse.
Every control read, write, and repair commit carries one absolute monotonic deadline that adapters must honor, with host ceilings of 10 seconds for handshake, 5 seconds for repair, 2 seconds for the fixed probe, 5 seconds for Shutdown, and command timeout plus 1 second for Execute delivery.
Guest receive and report calls take caller-supplied absolute deadlines so the future VMM retains sandbox TTL and cancellation policy.
An authenticated peer can still send a newly authenticated late record after any acknowledgement, so the static guest agent and exclusive control channel remain a trust boundary and the next owner read detects and poisons that violation.
The current code does not map the launch page into non-snapshot guest memory, retire a KVM memory slot, perform real clone repair, execute inside a guest, or establish sandbox Ready.

## 2026-08-29 - Normalized rootfs is a logical artifact, not a Generation

ADR 0019 adds `normalize_oci_rootfs` as the deep portable seam from one verified `ImportedOci` to one immutable `NormalizedRootfs` completion artifact.
The implementation reopens and verifies the import manifest and each selected layer, applies supported OCI whiteouts and filesystem metadata in a raw-byte logical tree, streams regular-file contents into CAS, and publishes a canonical binary tree manifest last.
The canonical identity excludes OCI compression, layer partitioning, tar order, and traversal provenance while retaining hard-link topology, supported metadata, symlink targets, FIFO nodes, and content digests.
Every input, extension record, expanded stream, path, entry, metadata total, file, aggregate content total, and completion manifest is explicitly bounded.
All raw tar headers across selected layers share the rootfs entry ceiling, local PAX and GNU naming bodies share its metadata ceiling, and GNU sparse entries fail from their raw header before body processing.
Version 1 accepts only byte-preserving local PAX `path` and `linkpath` values and rejects global, malformed, duplicate, xattr, timestamp, security, and unknown PAX metadata.
It rejects mixed local PAX and GNU naming extensions instead of choosing tar 0.4.46's format-specific precedence.
It also rejects devices, sockets, sparse and contiguous files, unknown node types, malformed whiteouts, unsafe paths, and unresolved or cyclic hard links.
Same-layer hard-link chains resolve through an iterative reverse-dependency queue, so a one-million-entry ceiling cannot create recursive stack growth or quadratic rescanning.
Two independent pinned `node:22` normalization runs produced the same rootfs digest `sha256:5dac6c571b970375a978c3f2f8777883e5bdd582fb4b43a5b872f929a2c7adf6`, 3,678,098 manifest bytes, 33,534 entries, and 1,125,654,269 logical file bytes.
Their normalization sections took 537.280 and 508.693 seconds on the development Mac because this offline path revalidates, decompresses, hashes, fsyncs, and republishes file objects twice in the ignored determinism test.
Those times are Generation build-path observations and make no claim about Machine launch, restore, readiness, or first-command latency.
Late-invalid normalization can leave unreachable content objects without exposing a partial rootfs completion artifact.
Private pre-alpha use therefore requires an operator-enforced job or store quota plus out-of-band garbage collection, and tenant admission remains prohibited until internal quota or reachability cleanup is implemented and tested.
`NormalizedRootfs` is not a mounted filesystem, disk image, bootable root, `GenerationId`, snapshot, compatibility certificate, readiness result, or sandbox performance result.
The next honest Generation step is a pinned deterministic disk-filesystem compiler and a separate KVM block-device mount and file-read proof.

## 2026-08-29 - Authenticated control deadlines are absolute adapter contracts

ADR 0021 now requires every control read, write, and host repair commit to receive an absolute `std::time::Instant` that the adapter MUST honor through cancellation and bounded teardown.
One deadline covers both reads of a frame and the complete host exchange, so partial frames or repeated output chunks cannot renew a liveness budget.
Host ceilings are ten seconds for Handshake, five seconds for Repair, fixed probe timeout plus one second of delivery grace, five seconds for Shutdown, and validated Execute timeout plus one second of delivery grace.
These are failure-containment ceilings rather than latency targets.
Guest connect, receive, and report calls take caller-supplied deadlines so sandbox TTL and control-plane cancellation remain outside the codec.
An authenticated guest agent can still send a late record after any acknowledgement, so the guest-agent channel remains a trust boundary and the next owner read detects and poisons that violation.

## 2026-08-29 - Beginner architecture model

The architecture now distinguishes four different meanings of foundation.
CPU virtualization and the Linux kernel are the physical foundation, `soma-kvm` is the lowest SOMA-owned production KVM layer, `soma-vmm` is the center of one sandbox data plane, and the lifecycle facade is the center of the public product.
A user-facing Template is a recipe that produces an immutable Generation, while Launch realizes that Generation as a fresh Instance of a Machine.
This layered language prevents libraries, processes, build artifacts, and running sandboxes from being treated as synonyms.
The visual teaching order begins at physical virtualization, enters the Machine, distinguishes host-side Generation artifacts from the guest `/` tree, and only then adds a workload such as Node 22.
Capacity education treats vCPU scheduling, resident memory, shared immutable pages, private dirty pages, sparse storage, network state, and host objects as independent limits whose minimum bounds safe admission.
Capacity language now distinguishes cumulative creations, queued requests, resident Instances, and simultaneously active Instances because only the latter three consume concurrent Host capacity and they consume it differently.
The README links directly to the 200-vCPU-on-80-thread explanation, and the visual atlas begins with a task-oriented contents list so capacity education is not buried inside the full machine walkthrough.
The capacity lesson now holds one Machine shape constant while moving from one Instance through 4, 16, 49, 64, and 200, then introduces larger NUMA Hosts, atomic admission, resource-specific failure modes, workload classes, and the evidence required before increasing density.
The visual model distinguishes the external calling agent, the mandatory SOMA Guest agent, and the user workload program.
Node.js, Python, shells, and other Workload runtimes come only from the selected workload image and are never implicit Launch prerequisites.
The capacity ladder continues through 300, 500, 800, and 1,000 on a fixed large Host, then through 1,000, 2,500, 5,000, 10,000, 25,000, 50,000, and 100,000 across a fleet with explicit spare capacity.

## 2026-08-29 - Repository-owned README branding

The README header uses transparent brand assets committed under `assets/brand` rather than website app icons or remotely named wordmarks.
The orb and MIOSA wordmark form one centered horizontal lockup rather than two vertically stacked marks.
The marks are intentionally unlinked because GitHub underlined residual inline anchor whitespace when either mark was linked.
The README restores current version, CI, security, Rust toolchain, platform, and license badges and replaces the opening documentation paragraph with a task-oriented file map.
The redundant top-level warning block was removed at the project owner's request, while implementation maturity remains stated in Project status and the platform evidence table.

## 2026-08-29 - Pinned PVH kernel boots on KVM to a challenge-bound serial sentinel

Decision-map ticket #4's acceptance test now passes on a real Ubuntu 24.04 x86_64 host: `run_kernel_boot` parses the pinned `vmlinux` with an owned bounded ELF64 parser, loads its segments and a top-down initramfs, writes the PVH pages, and enters the `XEN_ELFNOTE_PHYS32_ENTRY` address on one protected-mode vCPU.
The parser rejects a missing or duplicated Xen note, a malformed note segment, a segment below `0x01000000`, overlapping or overflowing segments, file bytes outside the image, and an entry outside executable loaded bytes; a truncation sweep and a bit-flip sweep over a synthetic image never panic, and the live negative test proves a corrupted note fails in `LoadGuest` before any vCPU exists.
The diagnostic 16550 model is output-only and bounded at 64 KiB: `LSR` always reports an empty transmitter, `IIR` acknowledges the transmit interrupt on read, `IER` is masked to its four defined bits, the scratch register and loopback `MSR` satisfy the 8250 autoconfig probe, and an irqfd on GSI 4 delivers the transmit interrupt so tty writes never stall.
The port bus answers only the eight UART ports and the keyboard-controller pair; the `0xfe` reset pulse from `reboot=k` is the orderly `Reset` exit, and every other port reads as a floating bus and is counted.
The kernel boot needs the in-kernel PIT: without it Linux still reaches `/init` and prints the sentinel, but the local APIC timer is never calibrated and a 20 ms `nanosleep` in `/init` never returns, so the PIT is part of the version 1 profile.
The CPUID template requires KVM's paravirtual signature leaf so the guest selects `kvm-clock`, and pins the bootstrap APIC identifiers to zero.
The command line is composed only in `cmdline.rs` from the fixed contract set plus `rdinit=/init` and `soma.nonce=<hex>`, which is the seam that later becomes part of `GenerationId`.
The retained result, including single-sample host residency numbers, is in `docs/evidence/2026-08-29-x86_64-pvh-kernel-boot.md`; it proves the cold-boot machine contract only, not a device, root filesystem, guest agent, readiness, snapshot, or latency claim.

## 2026-08-29 - Generation compiler compiles uncertified x86_64 machine artifacts

`soma-generation` now compiles one `TemplateRevision` plus one `NormalizedRootfs` into an immutable EROFS root, sterile ext4 overlay templates, a verified kernel, a deterministic `newc` initramfs, and a canonical `SOMAGEN` v1 manifest whose SHA-256 is the `GenerationId`.
The canonical tree is decoded by a hostile bounded decoder and streamed as an ordered tar with local PAX overrides into the pinned `erofs-utils` 1.9.4 `--tar=f` mode through standard input, so guest paths never become host paths.
A crate-private EROFS reader independently walks the produced image and requires exact path, type, mode, owner, epoch time, link target, hard-link group, link count, size, and content-digest equality with the tree before the image is stored.
Two fixture builds in different stores more than one second apart produced the same root image `sha256:39d989d2a75546c211a2a4cb0aad3b38358209a7ee319311c5e12b78010dd71f`, the same 64 MiB and 128 MiB overlay templates, and the same `GenerationId` `sha256:89c8c6fce6f15959cdd12b04518673b7985bb02aeb7616818378958843702b8d` on the Linux x86_64 development host with erofs-utils built from commit `f36cadb5c563995ab3aa8572a60ed6b721b9557d` in the pinned Ubuntu 24.04 image and e2fsprogs 1.47.0.
Reversing the OCI layer order produced identical machine artifacts and a different `GenerationId` because the source OCI manifest digest is a bound field.
`mke2fs -d` on an empty staged directory was measured to differ across seconds under `E2FSPROGS_FAKE_TIME` because it copies host change times, while `mke2fs` plus `debugfs -w -R mkdir` was byte-identical, so the compiler uses the latter.
The pinned `--all-time` option sets every EROFS modification time to the profile epoch, which discards per-file times that the normalized tree still carries.
The host build binds no builder-image digest, the normalized `node:22` store exists only on the development Mac, and guest boot, snapshot capture, and certification remain unimplemented, so no compiled Generation is launchable and none of this is a KVM, mount, or launch-latency result.

## 2026-08-29 - First authenticated command inside a cold-booted Generation

The `soma-kvm` x86_64 machine now wires the five-slot virtio bus to KVM: `KVM_EXIT_MMIO` is dispatched to the shared bus on the vCPU thread, every queue-notify register has an ioeventfd with `datamatch` on the queue index, every slot has an edge-triggered irqfd on GSIs 5 through 9 over KVM's default routing, and one epoll device thread services queues with a bounded budget per wakeup.
The dedicated launch page is a second KVM memory slot at `0xd0100000` that is written once, verified all-zero after the guest consumed it, and removed with a zero-length `KVM_SET_USER_MEMORY_REGION` at the repair commit.
The vsock `HostEndpoint` is exposed as a deadline-bounded byte channel so `soma-guest`'s `HostControl` drives the handshake, repair, probe, Execute, and Shutdown exchanges unchanged; the protocol glue lives in the live test because `soma-guest` is a private crate that the public `soma-kvm` package cannot depend on.
On a real Ubuntu 24.04 x86_64 host a Generation compiled from `busybox:stable-musl` booted, mounted EROFS and the private ext4 overlay, switched root, consumed the page, authenticated, passed the probe, returned the exact bytes of `/bin/busybox uname -a` with exit status 0, acknowledged shutdown, and exited through the keyboard-controller reset with descriptors and threads balanced; `Ready` came 164 ms and 193 ms after `KVM_RUN` in two debug-build samples, which is a cold-boot observation and not a latency claim.
The same test compiled the locally cached `node:22` image into a 1 GiB machine and returned `v22.23.2` from `/usr/local/bin/node --version` with `Ready` at 129 ms and a 40 ms command round trip; its normalized tree digest `sha256:2e48535f...` with 33,512 entries did not reproduce the Mac's `sha256:5dac6c57...` with 33,534 entries because the two hosts hold different `node:22` image revisions, so a same-input cross-host comparison is still open.
The `soma-kvm` live tests take `soma`, `soma-generation`, and `soma-guest` as path-only dev-dependencies, and `deny.toml` now sets `allow-wildcard-paths` so the wildcard ban still applies to every published dependency while `cargo package` strips the private path-only ones and the release packager keeps excluding private members.
The first boots exposed three guest-side contract bugs that are now fixed: early init must create or verify the `upper` and `work` directories the sterile template already carries, tmpfs session mounts must not repeat `nosuid,nodev` inside the option string, and the stop path must use the restart command because power-off degrades to an invisible in-kernel halt without ACPI.
Initramfs layout v2 carries `/dev/console`, `/dev/null`, and the Generation-scoped responder private key as a fifth compiler input, since the Rust runtime aborts PID 1 without standard descriptors and the agent takes the key from the initramfs before switching root; the responder public key is still carried out of band rather than bound into the manifest.
The run happened inside an `ubuntu:24.04` container with `/dev/kvm` passed through because the host seat session ended mid-work and `logind` moved the device's `uaccess` ACL to the display manager; the container adds no privilege and the evidence document records this.
Nothing here proves network egress, snapshot restore, a jail, prepared workers, certification, or any latency objective, and the retained result is `docs/evidence/2026-08-29-x86_64-first-sandbox-command.md`.

## 2026-08-29 - The VMM jail launcher constrains the probe on Ubuntu 24.04

Decision-map ticket #9 now has an implementation: `crates/soma-jail` records ownership, creates one cgroup v2 leaf with `memory.max`, `memory.swap.max=0`, `memory.oom.group=1`, `cpu.max`, and `pids.max`, clones the child with `clone3` directly into fresh user, mount, PID, network, IPC, and UTS namespaces and into that leaf with a pidfd, writes single-entry identity maps, and releases the child only after namespace, interface, and membership evidence has been read from the parent side.
The allocation-free pre-exec child sets the parent-death signal, drops to the ephemeral identity, clears dumpable, applies rlimits, enters an empty read-only tmpfs through `pivot_root` with the old root detached, seals the fixed descriptor table with `dup3` and `close_range`, verifies every slot by `fstat` and device number, installs `no_new_privs` and the startup seccomp filter, and executes from an open descriptor with `execveat(AT_EMPTY_PATH)`; a failed step is reported through a twelve-byte pipe message before `_exit`.
The seccomp filters are hand-assembled classic BPF with no libseccomp dependency, default to kill-process, reject every other architecture and the x32 bit, filter `ioctl` on the request number, require `CLONE_THREAD` and forbid every namespace flag on `clone`, answer `clone3` with `ENOSYS`, and drop the setup-only syscalls and ioctls in steady state; golden tests pin the startup program at 222 instructions with fingerprint `0x40b7c33a9001c79b` and the steady program at 135 instructions with fingerprint `0xe748c586d5877538`.
`JailLedger` records every resource before its effect, `reconcile` is idempotent and also runs on drop, and `recover` releases a crashed launcher's leaf and jail root from the durable record alone.
Unprivileged user namespaces are blocked on this host by Ubuntu's AppArmor restriction, so the fifteen live acceptance tests run as root inside a digest-pinned privileged Ubuntu 24.04 container with a private cgroup namespace and a delegated cgroup2 subtree; `scripts/jail-live-tests.sh` builds the static musl probe and test binary on the host and drives the container, and the retained result is in `docs/evidence/2026-08-29-vmm-jail-live.md`.
The inventory is labeled measured or reserved per entry: the probe trace measured `open`, `close`, `stat`, `fstat`, `poll`, `mmap`, `mprotect`, `munmap`, `brk`, `rt_sigaction`, `rt_sigprocmask`, `sendto`, `recvfrom`, `clone`, `fcntl`, the four identity getters, `sigaltstack`, `prctl`, `arch_prctl`, `gettid`, `futex`, `getdents64`, `set_tid_address`, `exit_group`, `prlimit64`, and `seccomp`; `soma-kvm` code measured `read`, `write`, `rt_sigreturn`, `ioctl`, `getpid`, `exit`, `tkill`, `tgkill`, and `eventfd2`; the disk backend, virtio devices, descriptor transfer, event loop, glibc runtime, allocator, and snapshot ioctl entries are reserved and were never observed.
Two musl facts changed the table during the run: musl issues the legacy `open` and `stat` syscalls where glibc issues `openat` and `newfstatat` or `statx`, and every new Rust thread calls `gettid` first, so those are admitted and `open` and `stat` are startup-only.
`pivot_root` needs one descriptor above the inherited table, so `RLIMIT_NOFILE` is applied right after the seal instead of before it, while the other rlimits keep the profile's order.
The KVM ioctl allowlist is exactly the set the current `soma-kvm` code issues plus the snapshot state groups reserved for restore; `TUNSETIFF` and every other request are killed, and `FIONBIO` is the only non-KVM request.
The launcher constrains the static `jail-probe` stand-in and has not wrapped the real `soma-vmm` binary, transferred a TAP endpoint, bounded `io.max`, installed a filter into a multi-threaded process, or served prepared workers; a glibc-linked VMM must be traced before its startup can be trusted, and the vsock accept path and `SCM_RIGHTS` claim transfer still need an allowlist decision.
The next dependency is wrapping the real `soma-vmm` executable and the prepared-worker claim path behind this launcher.

## 2026-08-29 - Second implementation audit blocks production networking

The fixed review range `4879517...d790555` materially closes the previous responder-authority, output-bounding, process-tree, Candidate-publication, hostile-validation, entropy, architecture, address-validation, provenance, and structured-command findings.
The new privileged network broker nevertheless has two production blockers: activation fabricates a repair attestation without authenticated guest evidence, and the daemon socket authorizes no peer before granting TAP and lifecycle authority.
Restore readiness is still caller-asserted, privileged network tools have no complete deadline and capture bounds, network reply delivery does not reject a short send, and the new burst tests break the portable repository gate through incompatible relative imports.
Two accepted ADR files also share number 0024 while specifying incompatible responder-key models, and retained snapshot evidence describes the obsolete reusable-key implementation rather than current bytes.
The required order is to restore the portable gate, secure the network authority boundary, bind readiness to authenticated evidence, repair the decision record, rerun current snapshot evidence, and then integrate one complete KVM lifecycle before adding more subsystems or publishing performance claims.
The detailed handoff is `docs/reviews/2026-08-29-implementation-reaudit.md`.

## 2026-08-29 - Production sandbox research favors one deep lifecycle and multiple preparation classes

Research across Firecracker, crosvm, Kata and Dragonball, libkrun, AWS SnapStart, Linux cgroup v2, and snapshot systems supports one SOMA hardware-isolated lifecycle rather than multiple unrelated sandbox products.
Cold build, cold boot, warm restore, prepared worker, paused pool, and ready pool are preparation classes for the same Machine and security contract.
The public seam should be one small `SandboxBackend` interface backed by a deep Instance Lifecycle module that owns ordering, deadlines, compensation, receipts, crash recovery, and terminal cleanup proof.
KVM is the primary tenant boundary but production isolation also requires a jailed VMM, narrow privileged brokers, authenticated guest control, network policy, storage ownership, cgroups, and proven cleanup.
The credible 10 ms target is prepared claim to authenticated Ready on a memory-local Linux Host, not OCI conversion or cold boot.
The detailed source-backed recommendation is `docs/research/production-sandbox-deep-research.md`.

## 2026-08-29 - SOMA already owns a substantial custom Rust VMM

The Linux x86_64 `soma-kvm` dependency graph uses rust-vmm for KVM bindings, ioctl wrappers, and system utilities while SOMA owns roughly 10,687 lines of virtio, 7,200 lines of generic snapshot logic, and 8,628 lines of x86_64 machine code.
The strategic direction remains a custom minimal VMM, but the crate currently exposes too many low-level details and must become one deep Machine module.
The highest-risk technical question is guest-memory soundness under concurrent KVM and device access, followed by custom virtqueue semantic correctness, the existence of multiple Machine ownership shapes, reactor lock behavior, and complete restore ordering.
SOMA should retain `kvm-bindings`, `kvm-ioctls`, and `vmm-sys-util`, then evaluate `vm-memory` and `virtio-queue` through adapters and differential evidence rather than immediately rewriting or rejecting them.
The implementation order is interface contraction, memory soundness, queue differential testing, one Machine owner, bounded reactor behavior, restore typestate, real VMM jail integration, hostile guest testing, restore profiling, and production admission.
The complete research and ten implementation tickets are in `docs/research/rust-vmm-engineering-deep-dive.md`.

## 2026-08-29 - Hidden Rust VMM research favors Dillo-shaped boundaries

Source inspection found that `pichi-vm/dillo` has the strongest small cross-platform machine boundary among the obscure Rust VMM repositories reviewed, with host-neutral Machine, Memory, CpuState, and Cpu traits and separate KVM, HVF, WHP, transport, and device crates.
Vibemon contains the broadest relevant mechanism set, including versioned cross-platform snapshots, delta memory, copy-on-write forks, userfaultfd paging, restore validation, and real lifecycle tests, but its multi-thousand-line VMM files make it a mechanism reference rather than a topology to copy.
Panorama is explicitly broken but demonstrates a valuable dirty-only reset loop that merges KVM dirty logging with device-originated writes before coalesced restore.
Hyperlight, Ignition, deterministic-vmm, ai-vmm, alvm, teaching VMMs, Firecracker runtimes, and type-1 hypervisors were classified separately so SOMA does not confuse a narrower guest ABI, an orchestrator, or a different virtualization layer with its production Linux KVM machine.
The resulting design direction is a Dillo-like backend seam, selected Vibemon mechanisms behind SOMA-owned interfaces, Panorama-inspired dirty-reset evidence, bounded-model proofs for hostile arithmetic, and independent measurement of every imported idea.
The detailed pinned-source review is `docs/research/rust-vmm-github-hidden-gems.md`.

## 2026-08-29 - Expanded GitHub census changes the diamond from one project to four

Overlapping GitHub repository and code searches across VMM, KVM, microVM, hypervisor, HVF, WHP, rust-vmm, virtio, snapshot, userfaultfd, deterministic execution, Firecracker forks, libkrun, KVM ioctls, dirty logging, and Host hypervisor calls found important projects omitted from the first curated pass.
Dillo remains the best minimal backend-interface reference, while Google Alioth is the strongest newly found low-level cross-platform VMM reference, Nanvix is the strongest co-designed VMM and guest-OS density reference, and Clone is the strongest newly found warm-fork mechanism reference.
The source audit also found recurring mistakes: warning and continuing after snapshot failures, substituting empty state, omitting device-originated dirty writes, incomplete Host external-state restoration, measuring KVM object restoration as spawn latency, ignoring KVM restore errors, sharing all tenants inside one unsafe process, mixing dead planned paths with performance targets, and cloning identity or PRNG state.
SOMA therefore requires fail-closed state capture, a unified dirty-producer ledger, explicit external-resource recreation, checked restore counts, bounded hostile snapshot parsers, one jailed VMM process per Machine, backend-specific evidence, and separate restore, Ready, command, and cleanup measurements.
The expanded pinned-source inventory, failure atlas, guardrails, and implementation tickets are in `docs/research/rust-vmm-github-census-and-failure-atlas.md`.
## 2026-08-30 - The x86_64 warm path optimization pass, end to end

The warm restore of a `node:22` Generation was measured, changed three times, and re-measured, and the consolidated retained result is `docs/evidence/2026-08-30-warm-path-optimization.md`.
Two baselines bound the pass: the 2026-08-29 debug-build loop at `Ready` p50 27.18 ms and p99 30.00 ms, and the same test with an optimized host build and no source change at p50 17.80 ms and p99 19.44 ms; the 9.4 ms between them is host-side device, serial, and vsock emulation running optimized and is a build profile rather than a change to SOMA.
The three code changes were each measured against the state immediately before it and are all measured against the release baseline: the launch-page poll on both sides of the resume fell from 2 ms and 1 ms to 100 us, the executor's flat 5 ms wait for a reapable child became a 50 us first check doubling to the same ceiling, and the launch-page memory slot moved to before the vCPU exists, where the identical ioctl costs tens of microseconds instead of two milliseconds.
At the branch head, ten iterations gave `Ready` p50 12.20 ms and p99 13.86 ms with a spread of 10.78 ms to 13.86 ms; with ten samples the nearest-rank p99 is the largest sample rather than an interior order statistic, and neither endpoint reproduces to four figures across three sessions, so the supported claim is a reduction between 30% and 39% from 17.80 ms.
Three things were refused: the Noise handshake, whose 447 us of responder work is X25519, BLAKE2s, and ChaCha20-Poly1305 and cannot be reduced without fewer or weaker Diffie-Hellman operations; pre-warming the identity and network overlay copy-ups, which would put placeholder identity files into an overlay template that is asserted sterile and is a Generation contract change; and moving `KVM_CREATE_VM` or `KVM_CREATE_VCPU` off the request path, which belongs to a prepared worker in `soma-hostd` because a pre-created vCPU must follow `KVM_CREATE_IRQCHIP` on a platform fixed in advance and then verified against each manifest.
No bound was relaxed: readiness still requires authenticated repair plus the fixed self-probe through the production executor, the launch page is still consumed once, zeroed, verified erased, and retired, and every deadline, kill, and descendant sweep is unchanged.
Supporting commits made the claim worth its words: until the branch head seven of the thirteen live KVM tests returned `ok` while executing nothing when the `node:22` layout was absent, each live test process now holds its own scratch tree, and the guest timing instrumentation sits behind a non-default feature so the shipped agent does not carry it.
The coordinator's Isorun figures are recorded beside these only as external context, measured over the network on 2026-08-30 from another continent: single create p50 22 ms and p99 27 ms, concurrency 100 p50 73 ms and p99 207 ms, and their `create_ms` excludes image preparation, since one cold image reported 52 ms while the caller waited 4,808 ms.
This is a single-restore measurement on one loaded host in a container with a warm page cache, `--test-threads=1`, one machine shape; it says nothing about concurrent restores and is not a certified budget or a latency objective.

## 2026-08-30 - The second implementation audit is answered in code, with one finding left open

Every finding of `docs/reviews/2026-08-29-implementation-reaudit.md` now maps to a commit and to the regression test that proves each acceptance gate, and the mapping is `docs/reviews/2026-08-29-implementation-reaudit-response.md`.
P0.1 removed the assertion-style repair attestation and made the broker mint a single-use challenge that only the claiming peer can convert into a receipt bound to Instance, assignment generation, operation, network intent and live transcript; P0.2 put the control socket in an owned directory with verified owner, group and mode, admitted peers only on a kernel-derived identity holding the capability the operation requires, bounded the receive side, and bound a release to the peer recorded in the durable assignment.
P1.2 extracted the process-group, absolute-deadline, bounded-capture primitive into `soma-supervise` and put the privileged `nft` and `conntrack` invocations behind it, P1.3 made every reply require complete delivery with the lifecycle mutation committing before delivery and replay answered from the operation identity, and P1.1 replaced `Restored::ready` with a transition that consumes a readiness receipt naming the session the published launch page binds.
P0.3 renumbered the duplicate pre-launch capture decision to ADR 0030 and marked its responder-key provisions superseded, and P1.6 defined one five-term status vocabulary in the engineering standard and added `docs/claim-ledger.md` as the single place that states what SOMA can do today; both are now enforced by `scripts/check-architecture.sh` rather than by review.
P1.4, the portable benchmark gate the burst tests broke with relative imports, was fixed on `main` in `e7a076a` and this branch is rebased on it.
Two closures carry a residual that the documentation and a named test now pin rather than hide: the activation capability proves claimant continuity and single use and not guest repair, because the broker itself hands the claiming peer the only secret in the scheme, and the readiness receipt records the restored ready transition but gates nothing yet, because no execution or network-activation seam consumes it.
P1.5 remains open and no commit can close it: it needs a fresh capture-and-restore run on a KVM host at the current commit with fresh per-Instance authority, so the retained snapshot artifact is labeled historical at `7c1127d` and the ledger records the current design as component-tested.
The incomplete production gates the audit lists are capability gaps rather than defects, and each of them is a designed or component-tested ledger row: real guest networking, Generation certification, a jail around the real `soma-vmm`, production connectivity, prepared overlay heads in the launch path, Host composition, the public KVM Backend, and an admitted KVM burst campaign.
## 2026-08-30 - Public KVM Backend audit at 08e4d45

The public KVM Backend now reaches a real cold-boot KVM lifecycle, but the audit in `docs/reviews/2026-08-30-public-kvm-backend-audit.md` records two P0 identity violations and six P1 lifecycle, ownership, cleanup, and production-composition gaps.
No performance result from this path is admissible until certified Generation admission and exact public-to-guest Instance identity are enforced.

## 2026-08-30 - Prepared-store follow-up closes selection ambiguity, not immutable admission

Commit `174dc18` makes duplicate reference claims fail as ambiguous and makes the ADR-number checker portable across GNU and BSD userlands.
The prepared-store gate remains open because it still checks and reopens mutable pathnames, retains a store path across Resolve and Launch, reads hostile files without explicit bounds, discards directory-iteration errors, and admits decoded Candidate data without certification or complete artifact verification.
The updated gate status and required descriptor-relative correction are recorded in `docs/reviews/2026-08-30-public-kvm-backend-audit.md`.
Commit `6f149d6` subsequently added default refusal for uncertified Candidates unless `SOMA_ALLOW_UNCERTIFIED_GENERATION=1`, which is a useful development mitigation but not the required structural Candidate-to-Generation admission barrier.
Its named refusal test uses malformed bytes and stops at `Damaged`, so a valid Candidate reaching the `Uncertified` branch still needs public-boundary evidence.

## 2026-08-30 - Competitor speed evidence is indexed by measurement boundary

`COMPETITORS.md` now consolidates hosted-provider, VMM, runtime, and research-repository speed evidence into one table.
The table keeps vendor claims, independent observations, project benchmarks, source comments, and engineering targets distinct.
Restore-only, boot-only, authenticated Ready, first-command, and external TTI measurements remain separate because combining them would produce a false leaderboard.

## 2026-08-30 - Full KVM and server-setup re-audit found executable-flow blockers

The comprehensive re-audit from `08e4d45` through `50cd82e` is recorded in `docs/reviews/2026-08-30-kvm-and-server-setup-full-reaudit.md`.
The audit confirms real fixes to Instance binding, second-Launch rejection, unknown cleanup reporting, reference ambiguity, bounded prepared reads, and checker portability.
It also records that certified admission, descriptor-pinned store identity, bounded timeout reclamation, descriptor-relative head cleanup, current live evidence, and the production composition remain open.
The new server flow is not executable as documented because repository acquisition is ordered after its first script, `/srv/soma` is never provisioned, failed readiness checks return success, strict doctor overclaims its checks, and OCI naming is incomplete.

## 2026-08-30 - Immediate audit remediation and CI portability repair

The audited server flow now obtains the repository first, provisions `/srv/soma`, fails required preflight checks closed, invokes the built CLI by path, and describes the KVM doctor and development backend without production overclaiming.
Generation preparation now normalizes qualified OCI references, keys entries by the SHA-256 of the exact reference, compiles in a private sibling staging directory, refuses replacement, and publishes with Linux `RENAME_NOREPLACE` plus parent-directory fsync.
The portable CI repair removed an impossible uninhabited-backend match and corrected target gates across KVM readiness, integration fixtures, ingress, guest-agent Unix operands, block devices, and process-control tests.
The Linux ARM64, Intel macOS, and Windows ARM64 workspace checks pass locally, the macOS portable Rust and documentation suites pass after four load-sensitive host tests were corrected, and Linux amd64 Docker passes the publication example plus a locked all-target workspace check.
The spell checker passes after integrating the separately pushed repository vocabulary update.
The filesystem-tool builder must force `linux/amd64`; otherwise Docker Desktop on an Apple Silicon development machine silently emits ARM64 executables for the x86_64 server path.
The forced-platform build produced x86-64 ELF tools and reported erofs-utils 1.9.4 and e2fsprogs 1.47.0 when executed inside Linux amd64.
Windows Clippy evaluates non-Unix fallback code that ordinary cross-target `cargo check` does not lint, so portable validation must include target-specific Clippy when changing a fallback implementation.
Windows rejects Unix directory-fsync operations with `PermissionDenied`; host and network ledger record files are flushed everywhere, while parent-directory metadata is flushed only on Unix production hosts.
None of these results is a live KVM-host proof, a certified Generation proof, or a performance claim.

## 2026-08-30 - Persistent managed ownership belongs in the Host Runtime

ADR 0031 makes `soma-hostd` the persistent owner of managed Instances across CLI, MCP, and provider-adapter process lifetimes.
The external interface remains the provider-neutral lifecycle, while durable idempotency, admission, prepared claims, one jailed VMM per Machine, authenticated guest sessions, cleanup, and reconciliation remain private implementation modules.
The failed KVM burst attempt was a cold-boot lifecycle probe rather than a restore experiment, and it cannot become exact ComputeSDK evidence until the local persistent lifecycle and the external provider adapter both exist.
New burst artifacts use `soma.burst.v2` so effective runtime-engine identities are mandatory, while the reader preserves compatibility with `soma.burst.v1` artifacts that predate that provenance field.

## 2026-08-30 - CI execution follows proof type and change scope

Routine code changes use one Ubuntu 24.04 correctness job instead of the full hosted operating-system matrix.
Documentation changes use a five-minute policy job, portability runs weekly or manually, security runs weekly or on relevant dependency and policy changes, and real KVM work runs only for release tags or explicit dispatch.
Release packaging remains tag-driven or manual.
This preserves platform, security, KVM, and release gates while stopping documentation and ordinary source pushes from launching unrelated expensive jobs.

## 2026-08-30 - Snapshot identity is Candidate-first and binds the overlay

ADR 0032 removes a circular identity contract by making snapshot schema 2 bind the exact Candidate that was booted and captured.
The ready Generation identity is derived only after certification adds the memory, overlay, and state descriptors to `SOMAGEN` schema 2.
The captured overlay is now a typed `OverlaySnapshot` artifact, so every artifact required by restore is covered by the final Generation identity.
All schema 1 pre-alpha artifacts fail closed and must be rebuilt.

## 2026-08-30 - Snapshot certification closes the Candidate-to-Generation identity loop

Snapshot installation now publishes memory, overlay, and state objects under exact typed descriptors before certification.
Certification re-verifies the immutable Candidate, all three store objects, the schema 2 state manifest, its embedded Candidate identity, and its memory and overlay bindings before minting the non-forgeable promotion token.
Ready Generation verification repeats snapshot admission and can report launchable only after those checks pass.
The Linux x86_64 live capture suite now compiles an ignored full capture-to-install-to-certify-to-promote-to-reverify test, but a fresh hardware run is still required before recording new live evidence.
## 2026-08-30 - Sterile restore holds no readiness authority before assignment

The prepared restore seam now keeps the stopped KVM machine, immutable snapshot facts, and inactive captured device configuration behind a consuming `Sterile::assign` transition.
It does not sample the single-use readiness challenge until private-disk and CID assignment succeeds.
The captured CID is an inert snapshot-format placeholder rather than an assigned host identity because the sterile type cannot start the vCPU, publish a launch page, or expose the device bus.
ADR 0033 records this boundary and requires a later live network assignment to replace the placeholder MAC with fresh per-Instance network authority.
The current `soma-local` request path still restores on demand and must not report a prepared-worker latency class.

## 2026-08-30 - MIOSA rollout remains a gated side-by-side experiment

SOMA must enter MIOSA as an explicit experimental engine on separate hosts while the Firecracker path remains the control and rollback target.
The MIOSA adapter stays outside this repository, and no customer deployment begins until certified admission, jailed one-process-per-VM ownership, fresh networking, prepared-worker transfer, restart reconciliation, density tests, the exact ComputeSDK campaign, and soak evidence all pass on one immutable release.

## 2026-09-01 - The Linux gate owns its descriptor prerequisite

The hostd latency test prepares 1,000 workers and intentionally retains three descriptors per worker, so an ordinary SSH soft limit of 1,024 makes the first socket-pair allocation report a disk resource failure even when disk space and inodes are healthy.
The Linux check now raises its own soft open-file limit to 4,096 before running tests and fails with a direct prerequisite error when the host hard limit cannot support that value.

## 2026-09-01 - Persistent MCP KVM burst evidence on every east host

The burst harness originally placed its managed state root beside the results, which made `state_root/machines/<instance>.sock` exceed Linux's 108-byte Unix socket address limit and caused a typed `unsupported` refusal before VM creation.
KVM benchmark state now uses a securely generated short directory under `/tmp`, validates the final socket path budget, and retains typed MCP refusal codes instead of collapsing them to `tool_error`.
At clean revision `4688224`, the 34/33/33 distributed cohort completed 100 of 100 Node 22 commands and cleanups at p50 62.27 ms and p99 78.80 ms through one persistent MCP session per host.
Separate 100-concurrent cohorts also completed 100 of 100 on each host at p50 164.69 ms on host 03, 154.77 ms on host 04, and 139.77 ms on host 10.
These are warm-cache on-demand snapshot restores, not prepared-worker results and not evidence for the 10 ms objective.

## 2026-08-30 - Public KVM admission consumes only certified Generations

The Linux capture workflow now completes the existing Candidate lifecycle instead of leaving the prepared entry permanently uncertified.
After snapshot capture it installs the memory, overlay, and state objects into the immutable store, certifies their binding to the exact Candidate, promotes the Candidate into a ready Generation, and publishes `generation.id` last with create-exclusive owner-only semantics.
The public KVM resolver no longer has an environment-variable escape hatch for Candidate launch.
It parses the ready `GenerationId`, independently runs `verify_generation` over the manifest and every artifact, reports that identity in workload evidence, and binds the same certified identity into the guest launch page.
Linux x86_64 tests for Candidate refusal run inside an amd64 Docker environment on the Apple Silicon development host; live capture and KVM launch still require the production Linux host.
## 2026-09-01 - Certified artifacts remain verified at use

The Linux KVM launch path now opens every Generation and captured-snapshot object through its content descriptor and consumes the same handle whose digest and size were verified.
Direct restore, prepared-pool refill, private overlay cloning, and jailed-worker construction no longer treat the sibling `snapshot/` directory as a launch capability.
ADR 0041 records the trust-boundary decision.

## 2026-09-01 - Exact HTTP burst qualification exposes the real fast-storage invariant

The retained exact ComputeSDK-boundary harness starts before public create, executes a real `node -v` in the guest, stops after command success, and destroys outside the timing boundary.
Three consecutive 40/24/36 east-host cohorts completed 300 of 300 commands and cleanups with medians from 41.56 ms to 44.86 ms and p99 values from 53.30 ms to 56.66 ms through host-local HTTP.
These numbers are qualification evidence and exclude MIOSA edge, authentication, placement, load balancing, and GitHub-runner network latency.
Host10 exposed that the generic disk-copy fallback can add approximately one second even when KVM reaches Ready in approximately 5.45 ms.
The fast host profile therefore requires prepared artifacts and private heads on the same reflink-enabled XFS device.
The operator proof is implemented by `scripts/check-fast-storage.sh`, while production HostProfile admission still needs a durable machine-readable proof instead of relying on the generic clone path's correctness-first copy fallback.
Repeated qualification also exposed successful machine-host children without a wait owner, so the KVM host path now transfers those children to one process reaper after the authenticated handshake.
The generation process test no longer counts every pipe owned by the parallel Rust test harness as if it belonged to one invocation; bounded return plus descendant exit remain the scoped containment assertions.

## 2026-09-01 - Verified descriptors cross the machine-host boundary

Full artifact hashing inside every machine-host made an exact create plus `node -v` smoke sample take approximately 8.72 seconds.
The hosted API now cryptographically admits installed Generations before listening, retains the admitted open files, opens an independent description of every retained inode per launch, and transfers those descriptions to each child with a bounded `SCM_RIGHTS` handoff.
Semantic artifact certification remains the installer's publication responsibility because installed admission is intentionally narrower than `verify_generation`.
Using `File::try_clone` here was incorrect because both it and `SCM_RIGHTS` preserved a shared open-file-description offset across concurrent children.
The exact east-host campaign proved that defect caused the intermittent `backend_unavailable` refusals: independent descriptions produced repeated 100/100 create, command, and cleanup cohorts at the original 40/24/36 placement.
ADR 0044 moves only empty machine-host process creation before API readiness; it does not prepare a VM, memory, identity, or Generation assignment.
The best observed secure exact cohort was 62.32 ms median, 75.43 ms p95, and 75.94 ms p99, while the following three medians were 69.34, 67.42, and 68.32 ms, so the best run is demonstrated capability and not yet a stable public claim.
The child revalidates manifest identity, compiler profile, descriptor order, file kind, and size and never reopens artifact paths.
The first handoff receiver mistakenly read a 13-byte buffer for a 12-byte header and could consume the first JSON byte under concurrency; the exact-size receive frame fixed that race.
The final secure 1/1/98 east-host cohort completed 100 of 100 commands and cleanups at median 109.65 ms, p95 149.33 ms, and p99 157.23 ms.
The exact x86_64 `soma-api` release built from pushed commit `0aff1c5` has SHA-256 `e565e3f24905f1b498ad9ff6a42e5e7a280bf228681f40fc38fd3f8f106708a5` and completed the synchronized 40/20/40 east-host cohort with 100 of 100 commands and cleanups, 61.09 ms median, 70.61 ms p95, and 71.45 ms p99.
The earlier 41-45 ms cohorts used the removed size-only shortcut and remain legacy investigation evidence only.

## 2026-09-01 - CPU policy and placement are measured benchmark inputs

The fully validated sterile-host binary SHA-256 is `ebeaadfaee2902547399969b7e0d27cd38a8c3849f59524157bae18dc4b98850`.
Its first 40/24/36 exact cohort completed 100 commands and cleanups at 70.96 ms median, 83.76 ms p95, and 84.43 ms p99.
Host03 and host10 were using the AMD P-state `powersave` governor while host04 used `performance`, despite all three already selecting the `performance` energy preference.
Normalizing all CPUs to the `performance` governor reduced the repeated 40/24/36 tail to 77.50 ms p95 and 79.56 ms p99.
Placement probes found host-specific contention knees rather than one interchangeable slot count: host10 degraded at 48 concurrent launches and host04 degraded at 32.
The 34/32/34 split produced the best validated median at 66.43 ms but overloaded host04, while 40/20/40 produced the best validated tail at 76.14 ms p95 and 79.21 ms p99 with a 69.23 ms median.
The remaining stable median gap is split nearly evenly between on-demand machine restore and the first real Node command, so further progress requires a prepared-machine assignment seam or measured guest-command work rather than more placement guessing.

## 2026-09-01 - Hosted preparation now includes the stopped VM and private disk

Each configured HTTP worker now starts one dedicated machine-host child, transfers independently opened verified Generation descriptors, restores one stopped identity-free KVM VM, and creates one unique unlinked private overlay head before the listener accepts traffic.
Launch assigns Instance identity, guest CID, readiness authority, networking, secrets, and the already-private disk, then resumes and authenticates the guest.
ADR 0045 records the authority boundary and the honest on-demand fallback for depletion or a nonmatching shape.
The HTTP server now supports at most sixteen correctly framed requests per HTTP/1.1 connection, and the exact harness reuses that connection across create, `node -v`, and excluded cleanup.
Final binary SHA-256 `fbeb7229640c56876799196752daf2ed787e2ca545b38c4fa9aa5105324bff90` produced two consecutive clean 100-way cohorts at 60.01/69.27/69.85 ms and 63.78/70.75/71.04 ms for median/p95/p99 with 200 of 200 commands and cleanups.
A third cohort kept 100 percent success but host10 was simultaneously contended by encrypted-disk and control-plane work, producing an approximately 1.17 second tail that is retained as resilience evidence and excluded from clean comparison.
The production prepared API unit is versioned at `deploy/systemd/soma-api.service`; host-specific certified Generation and head paths live in `/etc/soma/api.env`, while the exact release binary lives at `/opt/soma/bin/soma-api`.

## 2026-09-03 - Hosted machine state-root namespace

`LocalRuntime` places durable instance records and the hosted machine-process directory under the same operator-owned state root.
`FileStateStore` enumeration must skip the exact reserved `machines` directory after proving that it is a real directory and not a symlink.
Every other non-instance entry remains corruption so enumeration never silently hides unknown state.
