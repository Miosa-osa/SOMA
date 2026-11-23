<p align="center">
  <img alt="MIOSA orb" src="assets/brand/miosa-icon.png#gh-dark-mode-only" width="72" align="middle">
  &nbsp;&nbsp;
  <img alt="MIOSA" src="assets/brand/miosa-logo-white-text.png#gh-dark-mode-only" width="260" align="middle">
</p>
<p align="center">
  <img alt="MIOSA orb" src="assets/brand/miosa-icon.png#gh-light-mode-only" width="72" align="middle">
  &nbsp;&nbsp;
  <img alt="MIOSA" src="assets/brand/miosa-logo-black-text.png#gh-light-mode-only" width="260" align="middle">
</p>

<h1 align="center">SOMA</h1>

<p align="center">
  <strong>Secure Optimized Machine Architecture</strong><br>
  An open-source runtime for isolated Linux workloads.
</p>

<p align="center">
  <a href="VERSION"><img alt="Alpha release" src="https://img.shields.io/badge/release-alpha-7c3aed?style=flat-square"></a>
  <a href="https://github.com/Miosa-osa/SOMA/actions/workflows/ci.yml"><img alt="CI workflow" src="https://img.shields.io/badge/CI-workflow-555555?style=flat-square&logo=githubactions"></a>
  <a href="https://github.com/Miosa-osa/SOMA/actions/workflows/security.yml"><img alt="Security workflow" src="https://img.shields.io/badge/security-workflow-555555?style=flat-square&logo=githubactions"></a>
  <a href="rust-toolchain.toml"><img alt="Rust 1.98" src="https://img.shields.io/badge/rust-1.98-000000?style=flat-square&logo=rust"></a>
  <a href="#platform-status"><img alt="Linux, macOS, and Windows" src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-2563eb?style=flat-square"></a>
  <a href="LICENSE"><img alt="Apache-2.0 license" src="https://img.shields.io/badge/license-Apache--2.0-111111?style=flat-square"></a>
</p>

SOMA starts and manages isolated Linux workloads from OCI images.
On Linux x86_64, it uses its Rust virtual machine monitor and KVM to run each workload in a microVM.
SOMA exposes a CLI, Rust library, HTTP API, and MCP server.

A **SOMA instance** is one concrete, independently identified lifetime of a machine launched by SOMA.
The same machine type can run an agent sandbox, an interactive desktop, or a web service.
The workload image supplies the software inside the instance.
The platform or caller supplies product features such as deployment URLs, release management, scheduling, and billing.

**Start here** · [Product and usage docs](docs/product/README.md) · [Engineering docs](docs/engineering/README.md) · [Management-plane security](#zero-trust-in-the-management-plane)

## What you can run

| Use case | What runs in the instance |
| --- | --- |
| Agent sandbox, build, or test | A headless Linux image with the tools needed to execute commands and work with files |
| Browser automation | A Linux image with a browser and its automation tools |
| Reinforcement learning | A simulator or evaluation environment running isolated episodes |
| Interactive desktop | A Linux desktop image with a graphical session, computer-use driver, and browser viewer |
| Web service or API | A Linux image with a server process and an exposed guest port |
| Function or scheduled job | A Linux image with a handler or command invoked by the integrating platform |
| Database process | A Linux image with the database and a storage configuration suited to its data lifecycle |

See the [desktop workload](workloads/desktop/README.md) for the reference desktop and the [RL environment guide](docs/product/guides/rl-environments.md) for an example episode.
The [use-case architecture](docs/engineering/architecture/use-cases.md) describes the lifecycle available to callers.
Public routing, replica management, and stable application identity belong to the platform integrating SOMA.

## How an instance starts

1. Choose an OCI image and the vCPU, memory, and writable-storage shape.
   Use a [Template](docs/product/guides/templates.md) to reuse the recipe.
2. Prepare a **Generation**, the immutable set of machine artifacts used to launch instances.
   A prepared Generation can carry a captured snapshot for restore.
3. **Launch** a fresh instance with its own identity and private writable state.
   The guest agent completes repair and authenticates before the instance is reported **Ready**.
4. Address the instance by its **Instance ID** to execute commands, work with files, or perform lifecycle operations.
   A **Receipt** records the effective isolation, operation outcome, timings, and cleanup where applicable.

An instance's display name is descriptive metadata.
A platform can keep an application or workspace identity stable across replacement instances.
The glossary defines [Instance](GLOSSARY.md#instance), [Machine](GLOSSARY.md#machine), and [Generation](GLOSSARY.md#generation).

## Try one isolated command

This development example requires a running Docker Desktop or Docker Engine.
It executes one command and cleans up the local workload.

```sh
git clone https://github.com/Miosa-osa/SOMA.git
cd SOMA
cargo run --locked -p soma-cli -- --backend docker doctor
cargo run --locked -p soma-cli -- --backend docker run node:22 -- /usr/local/bin/node --version
```

The Docker backend uses a Linux container for local development.
For the hardware-isolated Linux path, follow the [KVM host setup](docs/product/operations/server-setup.md) and [end-to-end check](docs/product/guides/end-to-end-check.md).

## Control and inspect instances

| Interface | Use it for |
| --- | --- |
| CLI | Run a one-shot command with `soma run` or manage an instance with `soma machine` |
| HTTP | Create, inspect, execute, change resources, suspend, resume, and destroy through `soma-api` |
| MCP | Give an agent access to bounded command execution and managed lifecycle operations |
| Rust | Embed the portable SOMA interface in a local tool or runtime integration |

The [HTTP API surface](docs/product/reference/api-surface.md) describes the routes, fields, and versioning.
Existing API paths such as `/v1/sandboxes/{instance}` address SOMA instances.
The [agent integration guide](docs/product/integrations/agents.md) documents the MCP tool contracts.

Supported backends can report effective CPU, memory, storage, and network properties.
The HTTP service exposes capability-gated live CPU and memory resizing, autosizing, host capacity, and encrypted suspension.
Resuming a suspension creates a fresh instance identity.

## Platform status

| Host | Engine and scope |
| --- | --- |
| Linux x86_64 with KVM | Native microVM path and the target for production admission |
| Apple Silicon macOS | Local Linux VM development through Apple Virtualization, with Docker available as a separate container backend |
| Linux or macOS with Docker | Local container development and portable interface testing |
| Other hosts | Portable client support varies by target, and local execution requires a supported backend |

SOMA refuses an unavailable backend explicitly.
The [portability guide](docs/product/operations/deployment-portability.md) explains host requirements and the difference between client portability and local engine support.

## Architecture and security

```text
CLI, HTTP, MCP, or Rust caller
             |
      SOMA lifecycle API
             |
   Capability-gated backend
             |
   Isolated guest and workload
```

The [architecture diagrams](docs/engineering/architecture/diagrams.md) trace launch, command execution, and cleanup.

<a name="how-soma-applies-zero-trust"></a>

### Zero trust in the management plane

The management plane controls who may launch, connect to, inspect, or stop a SOMA instance.
In a zero-trust deployment, the management plane authenticates the caller and authorizes access to the requested resource based on identity and policy, regardless of network location.
Agents running inside an instance use ordinary guest permissions for their work.
When a human, service, or agent calls an instance management API, the platform applies its access policy at that boundary.

The [management-plane security guide](docs/engineering/management-plane-security.md) explains the existing runner checks, the trust assumption on the host API, and what belongs to the platform.

### Instance isolation

SOMA treats workload code, guest messages, and guest-controlled device state as untrusted at the host boundary.
Code running as root inside an instance still runs on the guest side of that boundary.

- **Verify before launch.** SOMA binds a prepared Generation to immutable artifacts and checks compatibility before restore.
  An incompatible artifact or unavailable backend fails explicitly.
- **Isolate each instance.** On Linux KVM, a VMM process owns the instance and its virtual devices.
  Instances can share immutable Generation artifacts, and each gets private writable memory and disk state.
- **Authenticate readiness.** Every launch creates fresh guest authority.
  The guest agent completes identity, entropy, time, network, and transport repair before it can prove readiness over its own authenticated session.
- **Scope execution and cleanup.** Guest commands use direct argument arrays with bounded time and output.
  Lifecycle operations check ownership, and receipts report the result and cleanup state where applicable.

The host kernel, hardware virtualization, SOMA release, and operator policy are part of the trusted base.
The [threat model](docs/engineering/threat-model.md) describes those assumptions, the adversaries, and the evidence required before production use.

### How we verify it during development

The [contribution guide](CONTRIBUTING.md) describes the test workflow and review process.
Changes to public interfaces, snapshot compatibility, or trust assumptions require an [architecture decision](docs/engineering/adr).
Rust `unsafe` code requires a local safety explanation and a test or invariant that supports it.

The repository checks correctness and dependency policy in CI, scans for secrets and vulnerable dependencies, and runs a separate smoke workflow on real KVM.
The [end-to-end check](docs/product/guides/end-to-end-check.md) verifies launch, guest execution, and cleanup on a prepared host.
The [claim ledger](docs/engineering/claim-ledger.md) records what evidence exists and the revisions it applies to.

Report suspected vulnerabilities through the private process in the [security policy](SECURITY.md).

## Release status and performance

The Linux x86_64 alpha.4 draft host package is **7.70 MB compressed**.
Its VMM executable is **1.93 MB** on disk, and its guest agent is **0.91 MB**.
The [size and memory footprint guide](docs/product/reference/footprint.md) covers package sizes and workload memory.
The [runtime comparison](docs/product/reference/runtime-comparison.md) compares executable sizes and capabilities with Firecracker, Cloud Hypervisor, gVisor, and Kata Containers.

SOMA is in alpha.
The [source version](VERSION), [versioning policy](docs/product/reference/versioning.md), and [claim ledger](docs/engineering/claim-ledger.md) describe the release, compatibility rules, and retained evidence.
The repository includes a [real KVM runner file-contract test](docs/engineering/evidence/2026-11-20-runner-file-contract-live.md) and a [desktop proof harness](scripts/desktop-proof.sh).
Production admission requires host-specific evidence and the gates in the [roadmap](ROADMAP.md).

These are engineering targets for the certified KVM path.

| Measured boundary | Target p50 | Target p99 |
| --- | --- | --- |
| Complete server-side create | Below 5 ms | Below 10 ms |
| First bounded command from accepted Launch | Below 10 ms | Below 20 ms |
| 100-way ComputeSDK Burst TTI | Below 50 ms | Below 90 ms |

The [benchmark contract](docs/engineering/benchmark-contract.md) defines the timing boundaries, raw samples, cache state, failures, and cleanup results required to support a performance claim.

## Where to go next

The [documentation index](docs/README.md) groups the guides by task.

| I want to | Read |
| --- | --- |
| Understand the machine from the host to the guest | [Beginner architecture guide](docs/engineering/architecture/beginners-guide.md) |
| Follow the original KVM bring-up and trace the boot path | [How one SOMA instance works](docs/engineering/architecture/how-it-works.md) |
| Build a reusable workload | [Creating a Template](docs/product/guides/creating-templates.md) |
| Check the meaning of a term | [Glossary](GLOSSARY.md) and [naming guide](docs/engineering/architecture/naming.md) |
| Find the owner of a Rust crate or subsystem | [Module map](docs/engineering/architecture/module-map.md) |
| Learn the engineering and evidence standards | [SOMA engineering standard](docs/engineering/standards/sota-engineering-standard.md) |
| Explore capacity, isolation, and the workload stack | [SOMA visual atlas](docs/engineering/architecture/visual-atlas.md) |

## Contributing

Start with the [contribution guide](CONTRIBUTING.md), [mission](MISSION.md), and accepted [architecture decisions](docs/engineering/adr).
Use the design issue template for interface, topology, snapshot, trust, or compatibility changes.

## License

SOMA source code is licensed under [Apache License 2.0](LICENSE).
MIOSA and SOMA names and logos follow the terms in [BRAND.md](BRAND.md).
