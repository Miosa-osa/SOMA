# Notes

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
