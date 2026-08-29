<p align="center">
  <a href="https://miosa.ai">
    <img alt="MIOSA orb" src="https://miosa.ai/apple-touch-icon.png" width="96" height="96">
  </a>
</p>

<p align="center">
  <a href="https://miosa.ai">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://miosa.ai/miosa-logo-white.png">
      <source media="(prefers-color-scheme: light)" srcset="https://miosa.ai/miosa-logo-black.png">
      <img alt="MIOSA" src="https://miosa.ai/miosa-logo-black.png" width="300" height="129">
    </picture>
  </a>
</p>

<h1 align="center">SOMA</h1>

<p align="center">
  <strong>Secure Optimized Machine Architecture</strong><br>
  Give every agent a body built to be trusted.
</p>

<p align="center">
  <a href="VERSION"><img alt="Version 1.0.0 alpha 1" src="https://img.shields.io/badge/version-1.0.0--alpha.1-7c3aed?style=flat-square"></a>
  <a href="LICENSE"><img alt="Apache 2.0 license" src="https://img.shields.io/badge/license-Apache--2.0-111111?style=flat-square"></a>
</p>

The name describes the role.
The model is the mind, while SOMA is the disposable machine body that executes its work.

For engineers, SOMA is an open-source sandbox engine for Linux workloads from OCI images.
It provides a small CLI, a portable Rust interface, and an MCP server for agents, backed by explicit lifecycle and execution evidence.
Start with the [architecture diagrams](docs/architecture/diagrams.md) for the complete flow or use the [SOMA glossary](GLOSSARY.md) to connect terminology across OCI, virtualization, security, and performance.
Read [Local sandbox reality](docs/architecture/local-sandbox-reality.md) for the exact distinction between SOMA configuration, Docker containers, Apple VMs, and the future Linux custom VMM.
Linux implementation work should start with the [custom VMM handoff](docs/operations/linux-vmm-handoff.md).
The [custom VMM decision map](docs/research/vmm-decision-map.md) sequences the remaining research and implementation work by dependency.

> [!WARNING]
> SOMA is alpha software and is not safe for untrusted production workloads.
> The local Docker backend runs Linux containers for development today.
> The Linux custom-VMM engine remains under construction.
> Dedicated ignored tests can direct-boot a trusted ARM64 Linux fixture, execute one challenge-bound direct command, and prove bounded teardown, but that test-only path is not linked into the library, does not execute OCI workloads, and does not expose a sandbox lifecycle.

## What makes SOMA different

- One explicitly identified Linux container per local Docker sandbox today, with a custom hardware-isolated VMM planned for Linux hosts.
- Direct argument-vector execution without a host shell.
- Bounded commands, time, output, and control responses.
- User-selected CPU, memory, storage, image, and human-readable Machine metadata.
- Exact OCI manifest identity with explicit observed-only or launch-enforced binding evidence.
- Fail-closed platform selection with no silent downgrade to host processes or namespace-only isolation.
- Evidence-carrying receipts for workload identity, Instance identity, isolation, preparation, shape, timing, command outcome, and cleanup.
- One portable use-case surface for humans, agents, and cloud control planes.

## Try it locally with Docker Desktop

The current working local path uses Docker Desktop on macOS or Docker Engine on Linux.
Docker must be running and the host must be able to run Linux ARM64 images.

```sh
git clone https://github.com/Miosa-osa/SOMA.git
cd SOMA
cargo run --locked -p soma-cli -- doctor
cargo run --locked -p soma-cli -- run node:22 -- /usr/local/bin/node --version
```

The final command pulls or reuses `node:22`, creates a constrained Docker container, runs Node directly, returns its exact bytes, and proves cleanup.
Use `--backend docker` to select Docker explicitly.
Ubuntu, Python, Kali, and other compatible Linux ARM64 OCI images use the same interface.
This local path is a container boundary inside Docker Desktop's Linux VM, not a per-sandbox hardware VM.

### What works on macOS

The Apple backend creates a Linux VM through Apple Virtualization.
The Docker backend creates a Linux container inside Docker Desktop's Linux VM.
Both are usable local SOMA sandbox paths, with different isolation guarantees.

The custom Rust KVM VMM and other Linux-KVM VMMs require Linux `/dev/kvm` and cannot run natively on macOS.
A Docker image can compile the VMM and run KVM-independent tests, but Docker Desktop does not turn the Mac into a reliable nested-KVM host.
Real VMM execution and latency benchmarks belong on a Linux KVM host.

## Shape and customize a sandbox

Every run or managed launch carries an explicit Machine shape with vCPU, memory, and writable-storage dimensions.
It can also carry a bounded human display name, while a globally unique Instance ID remains the only lifecycle and ownership identity.

OCI layers are the reproducible customization mechanism.
Change a Dockerfile or build input, produce a new image digest, and SOMA resolves that digest into a separately identified Generation instead of mutating a shared base VM.
Persistent mutable project data will use an explicitly sized workspace volume with its own ownership contract, while disposable Machine state is destroyed after use.

Backends must report each effective dimension independently.
If a backend cannot prove that a requested disk, CPU, memory, or network property was enforced, the receipt reports that dimension as unavailable rather than inventing a value.
The current Apple development backend enforces requested CPU and memory but reports root writable-storage enforcement as unavailable.

## Built for agents

SOMA exposes bounded stdio MCP tools for one-shot execution and managed launch, execute, inspect, stop, and destroy operations.
Guest commands are structured argument arrays rather than shell strings, and arbitrary binary output is returned with explicit base64 encoding.

Claude Code, Codex, OSA, Hermes, and other MCP clients can use the same local server.
See the [agent integration guide](docs/integrations/agents.md) for exact setup and tool contracts.

## Security model

SOMA treats security state as evidence instead of a marketing label.
The interface records what was observed, what was enforced, what remained unavailable, and whether owned resources were cleaned up.

The current implementation includes constrained Docker-container execution for local development, direct process invocation, strict input bounds, output and timeout enforcement, ownership checks before lifecycle mutations, redacted diagnostics, typed failures, and dependency and secret scanning.
The production design additionally requires authenticated guest readiness, fresh identity repair, private copy-on-write memory and disk state, a constrained VMM process, and certified immutable Generations before stable `1.0.0`.

Read the [threat model](docs/threat-model.md) and [security policy](SECURITY.md) before evaluating trust claims.
Report vulnerabilities privately rather than through a public issue.

## Platform status

Portable clients and local isolation engines earn support independently.
An unsupported local engine returns an explicit error and never runs the workload on the host as a fallback.

| Host | CLI, library, and MCP | Local sandbox engine | Current evidence |
| --- | --- | --- | --- |
| macOS with Docker Desktop | Native validation | Linux container per OCI sandbox inside Docker's Linux VM | Live Ubuntu and Node 22 lifecycle, command, and cleanup validation |
| Ubuntu 24.04 and 26.04 x86_64 | Native CI | KVM capability probe and raw halt-guest proof (test-only) | One memory slot, one protected-mode vCPU, port-I/O capture, `hlt`, and cleanup proven on a real host; no kernel boot, device, or sandbox lifecycle yet |
| Windows Server 2025 x86_64 | Native CI | None | Portable client only |
| Linux ARM64 | Native development validation | KVM capability probe | Explicit-fixture cold boot and direct command execution exist only as dedicated ignored tests, not a custom sandbox lifecycle |
| Intel macOS, Windows ARM64 | Compile gate | None | Portable client only |

Linux OCI guests are the first workload contract.
An authenticated remote engine is planned for clients without a supported local engine, but it is not implemented in this alpha.
The [deployment portability contract](docs/operations/deployment-portability.md) explains how suitable AWS, Google Cloud, other Linux cloud, and on-premises hosts earn engine support while managed function runtimes remain remote callers.

## Architecture

```text
human CLI      agent MCP      Rust caller
    \              |              /
       portable SOMA use-case facade
                    |
       capability-gated backend seam
              /                 \
 Apple Container 1.3       SOMA KVM path
 development backend       production target
              |                 |
       Linux ARM64 VM      soma-vmm + Linux VM
```

The latency-sensitive production design uses a node-local prepared-worker allocator, one VMM process per Machine, immutable Generation artifacts, private copy-on-write state, and one fused authenticated repair and readiness operation.
Provider placement, billing, tenant policy, and public control planes remain outside this repository.
The [complete architecture diagrams](docs/architecture/diagrams.md) include the one-shot transaction, 100-sandbox burst path, durable managed lifecycle, customization flow, and security boundaries.

## Performance contract

These are admission targets for the future certified KVM engine, not current benchmark claims.

| Measured boundary | Target p50 | Target p99 |
| --- | ---: | ---: |
| Complete server-side create | Below 5 ms | Below 10 ms |
| First bounded command from accepted launch | Below 10 ms | Below 20 ms |
| Exact 100-way ComputeSDK Burst TTI | Below 50 ms median | Below 90 ms |

Every published result must retain raw samples, failures, cleanup outcomes, cache state, and the exact timer boundary described in the [benchmark contract](docs/benchmark-contract.md).

## Project status

The source version is `1.0.0-alpha.1`.
The first stable release will be `1.0.0` only after the custom Ubuntu 24.04 x86_64 KVM path can build an OCI-derived Generation and complete real launch, authenticated command readiness, execution, cleanup, isolation, and burst-performance gates.
The current custom-VMM tracer bullets are the test-only [ARM64 explicit-fixture cold-boot proof](docs/adr/0014-arm64-kvm-cold-boot-proof.md) and [challenge-bound guest-command proof](docs/adr/0016-challenge-bound-arm64-guest-command-proof.md).
Their retained [cold-boot](docs/evidence/2026-08-28-arm64-kvm-cold-boot.md) and [command](docs/evidence/2026-08-28-arm64-kvm-command-proof.md) results are diagnostic evidence, not published performance benchmarks.
The workspace now also contains a bounded deterministic OCI-layout importer, a canonical logical rootfs normalizer, and an owned authenticated guest-control lifecycle.
The normalizer applies supported OCI filesystem semantics without host extraction, streams file content into CAS, and reproduced one pinned real `node:22` rootfs identity across independent runs.
Those modules verify Generation inputs, normalized tree identity, and protocol behavior, but they do not yet construct a bootable Generation, inject a fresh secret after snapshot restore, run the authenticated protocol inside a guest, or establish sandbox readiness.
The retained [real Node 22 OCI-import verification](docs/evidence/2026-08-29-node22-oci-import.md) and [Apple hardware-VM one-shot validation](docs/evidence/2026-08-29-apple-node22-one-shot.md) document the exact current evidence and its nonclaims.
The next implementation boundary is deterministic disk-filesystem compilation and read-only KVM block-device consumption, followed by a static Rust guest agent, snapshot-safe launch-secret injection, and real VMM control-channel wiring.

The [roadmap](ROADMAP.md) lists the evidence required for each phase.
The [competitor and prior-art ledger](COMPETITORS.md) separates primary-source facts, external claims, unknowns, transferable lessons, and measured results.

## Contributing

SOMA welcomes contributors in virtualization, Linux systems, Rust, security, OCI tooling, performance engineering, agent protocols, testing, and documentation.
Start with the [contribution guide](CONTRIBUTING.md), [mission](MISSION.md), [module map](docs/architecture/module-map.md), and accepted [architecture decisions](docs/adr).

Please use the design issue template for interface, topology, snapshot, trust, or compatibility changes.
Security findings follow the private process in [SECURITY.md](SECURITY.md).

## License

SOMA source code is licensed under [Apache License 2.0](LICENSE).
MIOSA and SOMA names and logos remain subject to the attribution and marks terms in [BRAND.md](BRAND.md).
