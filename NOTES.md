# Notes

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
