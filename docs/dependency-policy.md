# SOMA dependency and platform policy

## Verified baseline

The dependency baseline was checked against the Rust registry and installed toolchain on 2026-08-28.
The current Rust compiler is `1.98.0`, built from stable revision `88d9e12ae` dated 2026-08-18.

Direct registry dependencies are pinned exactly:

| Crate | Version | Role |
|---|---:|---|
| `base64` | 0.23.1 | Lossless binary output transport through JSON interfaces |
| `cap-fs-ext` | 4.0.3 | Descriptor-relative no-follow filesystem operations for OCI import |
| `cap-std` | 4.0.3 | Capability-oriented OCI layout and content-store access, and descriptor-relative head release and reconciliation |
| `clap` | 4.6.6 | Command-line parsing |
| `flate2` | 1.1.10 | Bounded gzip OCI layer decoding with the pure Rust backend |
| `futures-util` | 0.3.34 | Bounded asynchronous MCP stream adapters |
| `kvm-bindings` | 0.14.1 | Linux KVM ABI structures for the x86_64 machine floor and ARM constants |
| `kvm-ioctls` | 0.25.0 | Linux KVM ownership and ioctl wrappers |
| `libc` | 0.2.189 | Narrow Linux descriptor, signal, mount, socket, and ioctl operations for the KVM adapter, the static guest agent, the network broker's namespace, TAP, netlink, and descriptor-transfer mechanisms, and the XFS reflink storage profile |
| `linux-loader` | 0.14.0 | Checked Linux ARM64 Image loading |
| `rmcp` | 3.1.4 | Model Context Protocol server and test client |
| `schemars` | 1.2.2 | MCP tool input schemas |
| `semver` | 1.0.28 | Apple runtime compatibility policy |
| `serde` | 1.0.229 | Stable structured values and receipts |
| `serde_json` | 1.0.151 | CLI envelopes, Apple runtime documents, and test fixtures |
| `sha2` | 0.11.0 | Canonical request fingerprints, network profile and intent digests, and sterile template digests |
| `snow` | 0.10.0 | Fixed Noise guest-session handshake and transport implementation |
| `tar` | 0.4.46 | Streaming structural validation of expanded OCI layer archives |
| `tempfile` | 3.27.0 | Isolated OCI importer content stores and network broker ledger directories in tests |
| `tokio` | 1.53.1 | Bounded asynchronous MCP runtime |
| `tokio-util` | 0.7.19 | Length-delimited MCP framing |
| `uuid` | 1.26.0 | Fresh command-line operation and Instance identifiers |
| `vm-fdt` | 0.3.0 | Generated ARM64 flattened device trees |
| `vm-memory` | 0.18.0 | Checked guest-address and anonymous-memory mappings |
| `vm-superio` | 0.8.1 | Bounded 16550 UART model for the ARM64 proof path |
| `vmm-sys-util` | 0.15.0 | Targeted vCPU thread signaling and the x86_64 serial-interrupt eventfd registered with `KVM_IRQFD` |
| `zeroize` | 1.9.0 | Best-effort erasure of crate-owned secret wrapper buffers |

The versions in this table were the latest registry releases returned by the project verification on the research date.
That statement is historical evidence, not permission to assume they remain latest later.

## Selection rules

SOMA prefers the standard library and small focused dependencies.
A dependency must hide meaningful complexity, have a compatible license, have active maintenance evidence, and avoid pulling provider or control-plane policy into low-level crates.

Direct dependencies use exact version requirements and the repository commits `Cargo.lock`.
Exact pins make source review, security evidence, cross-target behavior, and release reproduction refer to one dependency graph.
Automated weekly updates propose reviewed graph changes instead of moving the graph during an unrelated build.

New dependencies require review of:

- Maintenance and release provenance.
- License compatibility with Apache License 2.0.
- Unsafe code and native build surface.
- Transitive dependency count and duplicate versions.
- Host, architecture, and feature gating.
- Untrusted-input exposure.
- Effect on binary size, startup, allocation, and the launch critical path.

The dependency graph denies unknown registries, unknown Git sources, wildcard requirements, unreviewed duplicate versions, yanked releases, advisories, and unapproved licenses.

## Update rules

Every dependency update must pass formatting, lint, tests, documentation tests, architecture checks, license checks, advisory checks, spelling, workflow validation, and secret scanning.
Target-specific changes also require their real target-host tests.
A newer version is not accepted when it weakens safety, increases the trusted computing base without benefit, changes a wire or artifact contract accidentally, or regresses the measured critical path.

Security updates take priority over the normal weekly cadence.
If a safe security update requires a public compatibility break after `1.0.0`, SOMA follows the major-version policy instead of disguising it as a patch.

## Platform baseline

The production certification target is Ubuntu 24.04 x86_64 on bare-metal KVM.
Ubuntu 26.04 x86_64 is a forward-compatibility preview and cannot replace the certified baseline.
Apple Silicon macOS supplies the development-only VM-per-OCI backend.
Linux, macOS, and Windows are portable client targets, with local engines documented separately from remote client support.

Cross-compilation proves only that target-gated source and dependencies compile.
It does not prove virtualization, isolation, cleanup, or performance on the target operating system.

## Reverification

Before a release, rerun registry, license, advisory, and provenance checks and update this document only when the verified baseline changes.
Retain the audit date so readers can distinguish a pinned reproducible graph from an eternal latest-version claim.
