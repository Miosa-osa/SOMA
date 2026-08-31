# SOMA state-of-the-art engineering standard

This document defines what SOMA means by state of the art.
It is an admission standard, not marketing copy and not a claim about the current implementation.

SOMA reaches the standard only when architecture, correctness, isolation, performance, modularity, operability, portability, supply-chain integrity, and usability pass together.
Winning one benchmark while weakening another property does not qualify.

## Governing order

When requirements conflict, resolve them in this order:

1. Correct ownership and cleanup.
2. Isolation and security.
3. Compatibility and reproducibility.
4. Honest evidence.
5. Tail latency and throughput.
6. Density and scalability.
7. Portability.
8. Operator and developer usability.

This order does not make performance optional.
It prevents performance work from hiding incomplete execution, reusing tenant authority, dropping failures, or moving work outside a measurement without disclosure.

## One product architecture

SOMA builds one production sandbox architecture for the initial host profile:

```text
Ubuntu 24.04 x86_64 Host
        |
        v
Linux KVM
        |
        v
one jailed SOMA VMM process
        |
        v
one fresh hardware-isolated Linux VM
        |
        +-- private vCPUs and memory
        +-- immutable Generation root
        +-- private writable storage
        +-- minimal virtual devices
        +-- fresh identity and authority
        +-- authenticated soma-guest
        `-- selected workload or agent
```

Apple virtualization, Docker, and future remote adapters exercise portable semantics or development workflows.
They do not silently substitute for the certified Linux KVM isolation class.

## Evidence classes

Every statement about SOMA must identify its evidence class.

| Evidence class | Meaning | Allowed claim |
|---|---|---|
| Design | Architecture or ADR exists | Intended behavior only |
| Unit proof | Pure logic passes focused tests | That module's logic only |
| Integration proof | Real modules cross their production seam | That exact integrated path |
| Host proof | Real certified Host executes the path | That HostProfile and artifact set |
| Conformance proof | Required matrix passes | Supported capability on named profiles |
| Performance proof | Raw controlled samples pass declared gates | Exact boundary, preparation class, and environment |
| Security review | Threat-driven independent review passes | Reviewed scope and revision only |

A lower evidence class must never be described using a higher class's language.

## Status vocabulary

Every capability statement in this repository uses exactly one of five status terms.
No other word may stand in for them, and the [claim ledger](../claim-ledger.md) records the term and the evidence for each capability.

| Status | Meaning | Required support |
|---|---|---|
| Designed | An ADR or research document decides the behavior | The decision document, and no implementation claim |
| Component-tested | The code exists and its own automated tests pass in the workspace | Named crate and tests; no real Host run is claimed |
| Live-proved | The path ran on a real Host or privileged container | The exact commit and the retained evidence artifact |
| Integrated | The capability runs through its real production seam with the rest of the lifecycle | An end-to-end receipt rather than a test harness driving crate internals |
| Production-admitted | Every applicable admission scorecard row is green | A signed immutable report naming the HostProfile |

The terms are ordered and cumulative in intent but not automatic: a live-proved result stays live-proved only for the commit it names.
When code changes underneath a retained result, the result becomes historical, the capability falls back to component-tested, and the ledger says so until the run is repeated.

## Architecture standard

The architecture must make ownership visible.
Every resource has exactly one owner during each lifecycle phase and an explicit transfer when ownership changes.

The canonical flow is:

```text
Template
-> Template Lock
-> Generation Candidate
-> certified Generation
-> placement and admission
-> sterile prepared resources
-> fresh Instance
-> authenticated Ready
-> Execute or agent session
-> Stop or Destroy
-> revocation and complete cleanup
```

Required properties:

- Preparation never hides inside Launch.
- A Candidate cannot be resolved as a ready Generation.
- A Generation contains no reusable tenant authority.
- A snapshot contains no reusable Instance authority.
- Placement selects only compatible certified Hosts.
- Partial acquisition always has a reverse cleanup path.
- Ambiguous authority or ownership destroys the affected Instance.
- A timeout never implies that an operation did not happen.

Primary references are [sandbox stack](../architecture/sandbox-stack.md), [topology](../architecture/topology.md), and [roadmap](../../ROADMAP.md).

## Deep module standard

SOMA prefers deep modules: substantial behavior behind a small interface.
Module depth is measured by leverage for callers and locality for maintainers, not by line count.

Every public module must answer:

1. Which complexity does its interface hide?
2. Which invariants does it own completely?
3. Which callers gain leverage from it?
4. Where is its seam tested?
5. What would scatter across callers if the module were deleted?

A new crate is justified only when it owns an independently meaningful contract, platform gate, dependency policy, release surface, or hostile-input seam.
A folder is preferred when behavior belongs to an existing deep module.

Required rules:

- One module has one coherent interface.
- The interface includes invariants, ordering, errors, configuration, and performance behavior.
- Callers do not reproduce validation or lifecycle state machines.
- Internal adapters remain private until behavior actually varies.
- One adapter does not justify a hypothetical public seam.
- Generic `core`, `common`, `manager`, `helpers`, and `utils` dumping grounds are prohibited.
- Root files remain shallow composition roots.
- Platform-specific code is gated at compilation and capability admission.
- Unsafe code stays in the smallest auditable module with a documented invariant.
- Agent brands remain data or Template modules rather than VMM branches.

The [module map](../architecture/module-map.md) is the canonical ownership reference.

## Interface standard

Public interfaces should make safe behavior easy and unsafe ambiguity impossible.

Required properties:

- Provider-neutral identities and resource shapes.
- Exact direct executable plus arguments instead of implicit shell strings.
- Explicit timeout and output bounds.
- Explicit unspecified, denied, or allowed capability intent.
- Typed failures and recovery directives.
- Idempotency based on complete canonical request identity.
- Binary-safe input and output.
- No host paths, raw descriptors, registry credentials, or provider secrets in portable requests.
- No silent fallback to a weaker isolation class.
- Versioned wire, storage, manifest, snapshot, and receipt contracts.

## Isolation and security standard

The threat model assumes hostile workloads, hostile OCI input, hostile guest-controlled device state, malformed snapshots, compromised guest user space, client disappearance, and partial Host failure.

Every production sandbox requires:

- Hardware virtualization.
- Private guest memory.
- Private writable state.
- One constrained VMM owner.
- Fresh Instance identity.
- Fresh authenticated authority.
- Fresh entropy and time repair.
- Network isolation when networking exists.
- Resource enforcement.
- Complete authority revocation.
- Idempotent cleanup.

Every optional capability activates mandatory safety work:

| Optional capability | Mandatory accompanying primitives |
|---|---|
| Networking | Namespace, addressing, policy, protected destinations, cleanup, evidence |
| Egress | Destination enforcement, DNS semantics, metadata blocking, bypass tests, evidence |
| Ingress | Explicit publication, authorization, port ownership, revocation, evidence |
| Persistent storage | Ownership, mount policy, quotas, detach, crash recovery, deletion semantics |
| Checkpoint | Authority exclusion, compatibility, identity repair, corruption rejection |
| PTY | Terminal bounds, signals, disconnect semantics, cleanup |
| Secret delivery | Scoped reference, fresh delivery, redaction, rotation, revocation |

No reusable private key, PSK, token, credential, network lease, writable disk identity, or session state may enter a reusable Generation or snapshot.

The [threat model](../threat-model.md) and accepted security ADRs define the detailed requirements.

## Virtual machine correctness standard

The VMM must implement the smallest machine contract that satisfies the product.
Additional emulation is a liability until justified by a required capability.

Required properties:

- Exact architecture, CPU, boot, memory, interrupt, timer, and device contracts.
- Checked KVM state transitions.
- One OS thread per vCPU unless a later measured design proves a better contract.
- Bounded guest-memory access.
- Bounded device queues and descriptor traversal.
- Versioned snapshot state for every persisted device.
- Fail-closed restore compatibility.
- Deadline-aware vCPU interruption.
- No BIOS, UEFI, PCI, ACPI, USB, graphics, or hotplug in version 1 unless admitted through a new machine profile.

Real Host tests must cover boot, stop, crash, timeout, malformed state, repeated cleanup, and descriptor balance.

## Guest-control standard

Readiness is an authenticated state, not serial output, process existence, an open port, or successful decryption alone.

The required sequence is:

```text
restore
-> accept one-use launch material
-> repair entropy, identity, time, and network state
-> erase and retire launch authority
-> authenticate a fresh control session
-> report repair complete under that session
-> Ready
```

Required properties:

- PID 1 survives hostile child behavior.
- Output is bounded before allocation and queueing.
- Entire process groups obey timeouts and cancellation.
- Commands never pass through an implicit shell.
- Operation identities cannot be reused.
- Protocol violations poison the session.
- Guest secrets do not enter diagnostics or receipts.
- Shutdown and cleanup have authenticated terminal evidence.

## Template and Generation standard

Templates optimize authoring.
Generations optimize reproducible Launch.
They remain distinct.

Required properties:

- Mutable OCI references resolve to exact platform digests.
- Ordered flat module composition has deterministic conflict detection.
- Every resulting field and permission has explainable provenance.
- A canonical Template Lock binds exact inputs and policy ceilings.
- Builds run in isolated pinned environments.
- Every material tool and input is cryptographically bound.
- Filesystem construction is reproducible.
- SBOM, vulnerability policy, build provenance, revocation identity, and signatures are retained.
- Boot, capture, repair contract, snapshot, and compatibility are certified before publication.
- Publication is atomic and manifest-last.
- Candidates, failures, revoked artifacts, and ready Generations are distinct states.
- Launch consumes only a ready compatible Generation.

The [Template implementation map](../research/template-implementation-map.md) and [Generation compiler](../research/generation-compiler.md) define the implementation order.

## Storage standard

Immutable data may be shared.
Mutable tenant state may not.

Required properties:

- Immutable root identity is content-addressed and reverified.
- Every Instance receives private writable state.
- Copy-on-write behavior is proven rather than inferred from an operation's success.
- Capacity admission accounts for future private writes.
- Storage creation, attachment, detach, release, and reconciliation have explicit owners.
- Filesystem and block metadata are bounded hostile input.
- Snapshot inclusion and persistent-volume ownership are explicit.
- Cleanup and garbage collection remain outside the Launch critical path.

## Networking standard

Networking is optional.
Fail-closed enforcement is mandatory whenever networking is selected.

Required properties:

- One private network attachment and identity per Instance.
- IPv4 and IPv6 semantics are explicit.
- DNS behavior is explicit.
- Network, broadcast, multicast, loopback, link-local, metadata, Host, peer, and control-plane destinations follow the declared profile.
- TCP, UDP, QUIC, raw-socket, redirect, SNI, Host-header, and proxy limitations are explicit.
- Domain policy resists DNS rebinding and proxy bypass.
- Ingress is independently authorized from egress.
- Network activation occurs only after repair permits it.
- Release removes routes, rules, conntrack state, addresses, ports, namespaces, and authority.
- A Backend that cannot enforce the effective policy rejects Launch.

## Reliability and recovery standard

Every mutating operation is a recoverable transaction.

Required properties:

- Intent is durable before side effects where crash recovery requires it.
- State transitions are monotonic and versioned.
- Exact retries replay or resume safely.
- Different requests cannot reuse one operation identity.
- Every acquired resource is recorded before ownership becomes ambiguous.
- Cleanup is idempotent.
- Parent death, process crash, Host restart, timeout, and client disconnect have explicit recovery behavior.
- Reconciliation never guesses that missing evidence means successful cleanup.
- Capacity is not returned until cleanup is proven.

## Performance standard

SOMA optimizes trustworthy time to an authenticated useful command.
It does not optimize a convenient internal milestone and call that sandbox readiness.

Required measurement boundaries include:

- Template resolution.
- Cold and cached Generation construction.
- Generation installation.
- Prepared-worker acquisition.
- Restore and device activation.
- Guest repair and authenticated Ready.
- First bounded command.
- Complete one-shot lifecycle and cleanup.
- Burst behavior.
- Pool replenishment and garbage collection.

Required evidence:

- Monotonic raw samples.
- Median, p95, p99, maximum, errors, and cleanup failures.
- Exact HostProfile, Generation, shape, workload, preparation class, cache state, and software revisions.
- Failures retained in the cohort.
- No silent retry or fallback.
- Separate latency, throughput, and amortized-rate reporting.
- At least 100 engineering bursts and 10,000 samples before stable performance claims.
- Exact external benchmark reproduction without moving the timing boundary.
- An external comparison must match the external product's **flow**, not only its timing boundary.
  Where the external sample creates an addressable sandbox and then commands it by identity, a SOMA
  figure measured by a single one-shot process is not a comparison and may not be printed beside
  it. See [why the Isorun pairing is invalid](../evidence/2026-08-31-isorun-comparison-is-not-like-for-like.md).

### Mechanism claims

A statement about *why* something is slow is a claim, and carries the same evidence burden as the
latency itself. A mechanism may be asserted only when a measurement **moves when that mechanism is
manipulated and does not move when something else is**. Plausibility is not evidence.

This rule exists because it was earned. Over one optimisation session, roughly ten mechanism
hypotheses were asserted from reasoning and then contradicted by measurement: the cost of running
`nft`, the cost of entering a network namespace, which receipt segment the private head clone lives
in, whether less guest memory is faster, whether the head clone cost was the clone or the `fsync`,
whether host demand paging explained the restore resume, whether huge pages would help it, whether
retiring the launch page earlier would help, whether a shape mismatch explained a benchmark scoring
zero, and which of two device sets was faster at concurrency. Every one was reasonable. Three were
only settled by building the change and measuring it **worse** than what it replaced.

Two corollaries:

- A single cohort is not a distribution. One hundred-way cohort per arm ranked two configurations
  backwards; six cohorts per arm reversed the result. Repeat before reporting an ordering.
- A negative result is a result. "Pre-faulting moves nothing and costs 57 ms" is retained evidence,
  not a failed experiment, and prevents the same hypothesis being paid for twice.

The numerical targets remain in [MISSION.md](../../MISSION.md) and the [benchmark contract](../benchmark-contract.md).

## Density and fleet standard

Density is not the number of VM identifiers a control plane can allocate.
It is the measured number of useful isolated workloads a Host can sustain under a declared workload distribution and service objective.

Required accounting includes:

- Reserved and dirty memory.
- Runnable vCPU pressure.
- VMM processes and threads.
- File descriptors.
- Page-fault rate.
- Storage extents and write amplification.
- TAP devices, namespaces, conntrack entries, ports, and bandwidth.
- Restore and cleanup concurrency.
- Generation-cache working set.

Fleet scale requires bounded cells, capability-aware placement, Host-authoritative admission, explicit overload, immutable artifact distribution, failure containment, revocation, and staged scale evidence.

## Portability standard

Portable semantics and local engine support are separate claims.

Required properties:

- Client and interface compilation on Linux, macOS, and Windows.
- Exact local-engine capability detection.
- No silent weaker fallback.
- Remote execution preserves lifecycle, idempotency, evidence, and cleanup semantics.
- Every Host architecture, kernel, filesystem, cloud, nested-virtualization mode, and device profile passes independent conformance.
- Cross-compilation does not imply runtime support.
- Architecture-specific ioctl, boot, interrupt, timer, and snapshot layouts are target-gated and tested.

## Supply-chain standard

Every launchable byte must have accountable provenance.

Required properties:

- Digest-pinned source and builder inputs.
- Exact tool identity for the binary that executes, not merely the path checked earlier.
- Dependency policy and minimal feature selection.
- Reproducible or explainably nonreproducible outputs.
- Signed manifests and retained build evidence.
- SBOM and vulnerability-policy result.
- Revocation and expiry behavior.
- No secret in build arguments, logs, artifact metadata, or committed Templates.
- Atomic publication and corruption rejection.

## Observability and evidence standard

Observability proves behavior without exposing tenant data or secrets.

Every terminal operation should report:

- Request and operation identity.
- Exact Generation and Template Lock identities when applicable.
- Effective isolation, preparation, shape, and policy classes.
- Monotonic lifecycle milestones.
- Command outcome and bounded binary-output identity.
- Cleanup state and remaining uncertainty.
- Host and compatibility evidence appropriate to the trust class.

A receipt is structured evidence, not automatically cryptographic attestation.
Signed or hardware-attested claims require separate trust and verification contracts.

## Developer and operator usability standard

SOTA infrastructure must be understandable and difficult to misuse.

Required properties:

- One clear public lifecycle.
- Small interfaces with typed diagnostics.
- Explain commands for Template composition, policy, placement rejection, and capacity rejection.
- Human and stable machine-readable output.
- Safe defaults and explicit dangerous capabilities.
- Agent compatibility through standard direct execution, stdio, PTY, and MCP transports.
- Documentation that shows containment, dependency, ownership, optionality, and data flow.
- Examples that state their required image, executable, policy, Backend, and evidence class.

## Testing standard

Tests must cross the same seams production callers cross.

The required ladder is:

```text
pure invariant tests
-> hostile decoder and property tests
-> adapter contract tests
-> real module integration
-> real Linux KVM lifecycle
-> cross-Instance isolation
-> failure injection
-> concurrency and saturation
-> performance evidence
-> independent security review
```

Unit tests cannot substitute for real Host integration.
A live happy path cannot substitute for hostile-input, failure, and cleanup testing.
Ignored live tests provide no evidence until a retained run executes them on the named HostProfile.

## Review stop conditions

Reviewers must block progression when any of these are true:

- Reusable secret authority appears in a Generation or snapshot.
- Hostile input can allocate or queue without a proven bound.
- A deadline can leave descendants, descriptors, threads, or resources alive indefinitely.
- A Candidate can be discovered as a ready Generation.
- A manifest or snapshot field is consumed without validation.
- Target-specific unsafe layouts compile for an unverified target.
- Cleanup uncertainty returns capacity to a pool.
- A weaker Backend silently satisfies a stronger request.
- Evidence omits failures or changes the declared timing boundary.
- Documentation claims a higher evidence class than tests prove.

## Admission scorecard

A capability is admitted only when every applicable row is green.

| Dimension | Required result |
|---|---|
| Architecture | Ownership and dependency map reviewed |
| Modularity | Deep interface and deletion test pass |
| Correctness | Positive, negative, boundary, and cross-field tests pass |
| Security | Threat cases and authority lifecycle pass |
| Resources | Memory, output, processes, descriptors, and queues are bounded |
| Recovery | Partial failure and restart cleanup pass |
| Compatibility | Exact Host and Generation profile passes |
| Supply chain | Inputs, tools, artifacts, and revocation are bound |
| Integration | Real production seam passes |
| Isolation | Cross-Instance mutation and authority tests pass |
| Performance | Raw tail results meet the declared boundary |
| Portability | Each claimed target passes its own conformance profile |
| Usability | Safe defaults, typed diagnostics, and explainability pass |
| Evidence | Receipts and retained artifacts support every claim |

No average score exists.
One red applicable row blocks admission.
