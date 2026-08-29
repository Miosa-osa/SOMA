# SOMA glossary

Virtualization language is fragmented across cloud products, Linux internals, OCI tooling, security engineering, and performance work.
This glossary connects SOMA's canonical terms across those layers so a contributor can read it linearly or follow related concepts as a small hypertext reference.

The glossary describes SOMA terminology.
It does not redefine an external standard, turn an accepted design into an implementation claim, or imply that similarly named provider features have identical semantics.

## Stack map

| Layer | Start with |
| --- | --- |
| Product | [SOMA](#soma), [sandbox](#sandbox), [Machine](#machine) |
| Workload | [OCI image](#oci-image), [workload identity](#workload-identity), [Generation](#generation) |
| Lifecycle | [Operation](#operation), [Instance](#instance), [Repair](#repair), [command readiness](#command-readiness) |
| Resources | [Machine shape](#machine-shape), [requested shape](#requested-shape), [effective shape](#effective-shape) |
| Runtime | [VMM](#vmm), [microVM](#microvm), [KVM](#kvm), [backend](#backend) |
| Deployment | [client adapter](#client-adapter), [engine host](#engine-host), [certified host profile](#certified-host-profile), [remote engine](#remote-engine) |
| Performance | [prepared worker](#prepared-worker), [warm path](#warm-path), [measurement boundary](#measurement-boundary) |
| Evidence | [execution receipt](#execution-receipt), [digest binding](#digest-binding), [cleanup evidence](#cleanup-evidence) |
| Persistence | [durable Machine state](#durable-machine-state), [state store](#state-store), [workspace volume](#workspace-volume) |

## Terms

### Backend

A Backend translates SOMA's portable use cases into one real isolation substrate and returns typed observations.
The Apple development adapter, future Linux KVM engine, and future authenticated remote engine are different Backends.
A Backend does not own provider billing, placement, public product tiers, or receipt semantics.

Related: [effective shape](#effective-shape), [VMM](#vmm), [execution receipt](#execution-receipt).

### Basic backend-reported evidence

Basic backend-reported evidence is a structured claim returned by an accepted Backend and validated for internal consistency by the facade.
It is useful operational evidence, but it is not a signature, trusted-platform attestation, or independent proof against a malicious host.

Related: [evidence class](#evidence-class), [execution receipt](#execution-receipt).

### Capability

A Capability is a requested behavior whose meaning stays independent of a provider's product vocabulary.
Network policy is the first portable Capability and distinguishes unspecified, denied, and allowed intent.
A security restriction such as denied network access requires positive enforcement evidence and cannot be satisfied by an unavailable observation.

Related: [Machine shape](#machine-shape), [effective shape](#effective-shape).

### Certified host profile

A Certified host profile is one exact provider or on-premises substrate whose operating system, kernel, architecture, CPU class, KVM interface, filesystem, network mode, and SOMA release passed the required conformance gates.
Certification belongs to that complete profile rather than a cloud logo or generic instance family.

Related: [engine host](#engine-host), [Host](#host), [KVM](#kvm).

### Client adapter

A Client adapter translates one caller environment into SOMA's portable use cases without implementing isolation itself.
The CLI, MCP server, Rust caller surface, and a future authenticated remote transport are Client adapters.

Related: [Backend](#backend), [remote engine](#remote-engine), [execution receipt](#execution-receipt).

### Cleanup evidence

Cleanup evidence records the terminal disposition of the Machine, memory, storage, network, and guest authority owned by an operation.
Complete, incomplete, not-owned, and unsupported-verification are distinct states.
Deleting a visible runtime object is not enough when separately owned resources can remain.

Related: [execution receipt](#execution-receipt), [terminal status](#terminal-status).

### Command readiness

Command readiness means an authenticated guest channel completed the required Repair sequence and a real bounded command transaction can succeed.
Process start, VM restore, console output, or an open socket are intermediate observations rather than command readiness.
The Apple development backend supplies a narrower local lifecycle observation and cannot certify the production KVM readiness contract.

Related: [guest agent](#guest-agent), [Repair](#repair), [Restore](#restore).

### ComputeSDK Burst TTI

ComputeSDK Burst Time to Interactive is the external create-through-first-command benchmark cohort that SOMA plans to reproduce without changing the upstream boundary.
The target cohort contains 100 concurrent sandboxes, and every sample must include success, failure, and cleanup outcomes.
Targets are not measurements until raw samples and the exact environment are retained.

Related: [measurement boundary](#measurement-boundary), [prepared worker](#prepared-worker), [warm path](#warm-path).

### Control plane

A Control plane admits requests, chooses hosts, manages durable intent, and coordinates lifecycle without executing guest device logic itself.
MIOSA-specific placement, billing, tenancy, and fleet policy remain outside the SOMA repository.

Related: [state store](#state-store), [Backend](#backend), [VMM](#vmm).

### Copy-on-write

Copy-on-write lets many Machines share immutable memory or disk backing until one Machine modifies a page or block.
Each Machine receives private mutations without copying an entire Generation during launch.
The backing artifact must remain immutable, and shared writable guest state is forbidden.

Related: [Generation](#generation), [private writable root](#private-writable-root), [Snapshot](#snapshot).

### Digest binding

Digest binding describes how strongly an exact observed OCI manifest digest constrains the workload that actually launched.
Launch-enforced binding is stronger than observed-only binding because a mutable image alias can change between inspection and launch.
The Apple Container 1.3 adapter reports observed-only binding and never presents it as immutable enforcement.

Related: [OCI manifest](#oci-manifest), [workload identity](#workload-identity).

### Direct command

A Direct command is one executable path plus a bounded argument vector passed without an implicit host shell.
Shell behavior is available only when the caller deliberately chooses a shell inside the guest as the executable.

Related: [command readiness](#command-readiness), [Operation](#operation).

### Durable Machine state

Durable Machine state is the versioned control record that lets managed lifecycle survive CLI and MCP process restarts.
It records write-ahead intent, active evidence, execution ownership, termination ownership, revisions, and bounded replay information.
It contains control evidence rather than guest memory or a persistent project filesystem.

Related: [state store](#state-store), [Operation ID](#operation-id), [Instance ID](#instance-id).

### Effective shape

Effective shape records the CPU, memory, storage, and network values a Backend actually observed or verified.
Each dimension can be observed or explicitly unavailable.
An effective resource observation that contradicts the requested value invalidates the Backend result.

Related: [Machine shape](#machine-shape), [requested shape](#requested-shape).

### Engine host

An Engine host is a physical or virtual Linux host authorized to run a capability-gated SOMA backend and own VMM processes.
A machine that can run the CLI is not automatically an Engine host.

Related: [certified host profile](#certified-host-profile), [Host](#host), [Backend](#backend).

### Evidence class

Evidence class states the trust strength of a receipt's observations.
The alpha uses basic backend-reported evidence.
Signed or hardware-attested evidence requires separate canonical encoding, key management, verification, and trust decisions.

Related: [basic backend-reported evidence](#basic-backend-reported-evidence), [execution receipt](#execution-receipt).

### Execution receipt

An Execution receipt is a versioned validated record of one terminal use-case outcome.
It binds operation and Instance identity, workload evidence, requested and effective shape, isolation and preparation classes, milestones, output metadata, terminal status, measurement boundary, and cleanup evidence.
It is the common evidence format returned to humans, agents, SDKs, and future control planes.

Related: [request fingerprint](#request-fingerprint), [terminal status](#terminal-status), [cleanup evidence](#cleanup-evidence).

### Generation

A Generation is a certified immutable execution artifact derived from an exact OCI workload and bound to a compatible kernel, root filesystem, machine state, guest agent, device layout, and restore contract.
An OCI image is an input to Generation construction rather than a synonym for a Generation.
Generation construction happens outside request-time launch latency.

Related: [Generation ID](#generation-id), [OCI image](#oci-image), [Snapshot](#snapshot).

### Generation ID

A Generation ID is the content identity of one certified Generation.
It must change when any security-critical artifact or compatibility input changes.

Related: [Generation](#generation), [workload identity](#workload-identity).

### Guest agent

The Guest agent is the minimal authenticated component inside a SOMA guest that performs Repair and bounded command exchange.
It authenticates the exact Instance lifetime rather than trusting an inherited channel from a captured Snapshot.

Related: [Repair](#repair), [command readiness](#command-readiness), [Instance](#instance).

### Hardware-isolated sandbox

A hardware-isolated sandbox places the workload behind a virtual-machine boundary rather than relying only on a shared host kernel namespace.
Hardware isolation narrows some classes of cross-workload risk but does not remove VMM, device, host-kernel, configuration, or control-plane vulnerabilities.

Related: [microVM](#microvm), [VMM](#vmm), [sandbox](#sandbox).

### Host

A Host is the physical or virtual Linux machine that owns KVM, memory, networking, storage, and SOMA processes for a set of independent Machines.
A Host admission decision is separate from a portable Machine shape.

Related: [Backend](#backend), [KVM](#kvm), [prepared resource bundle](#prepared-resource-bundle).

### Instance

An Instance is one concrete Machine lifetime with fresh identity, mutable state, entropy, network identity, and guest authority.
Changing the image, shape, or immutable startup configuration creates a new Instance rather than mutating an existing lifetime.

Related: [Instance ID](#instance-id), [Machine](#machine), [Generation](#generation).

### Instance ID

An Instance ID is the globally unique opaque identity for one Instance lifetime.
It is the only local lifecycle and ownership key.
A human Machine name never replaces it.

Related: [Machine name](#machine-name), [Operation ID](#operation-id).

### KVM

Kernel-based Virtual Machine is the Linux kernel interface SOMA uses to create and run the production x86_64 virtual-machine boundary.
Opening `/dev/kvm` and creating a VM proves host capability only, not guest boot, isolation, restore, readiness, or cleanup.

Related: [VMM](#vmm), [microVM](#microvm), [Host](#host).

### Machine

A Machine is one hardware-isolated runtime governed by the SOMA lifecycle contract.
The term names the technical resource, while sandbox names the user-facing execution product.

Related: [sandbox](#sandbox), [Instance](#instance), [Machine shape](#machine-shape).

### Machine name

A Machine name is optional bounded human-readable metadata.
It participates in the request fingerprint but never selects, owns, or authorizes a runtime object.
Different Instances may reuse a name according to operator policy without sharing identity.

Related: [Instance ID](#instance-id), [request fingerprint](#request-fingerprint).

### Machine shape

A Machine shape is the provider-neutral CPU, memory, writable-storage, and Capability request attached immutably to one Instance.
It uses technical dimensions rather than cloud instance-type names.
The Backend performs real capacity admission and reports effective evidence per dimension.

Related: [requested shape](#requested-shape), [effective shape](#effective-shape), [Capability](#capability).

### MCP

Model Context Protocol is the tool protocol used by `soma-mcp` to expose bounded SOMA operations over standard input and output.
Claude Code, Codex, OSA, Hermes, and other compatible clients can use the same tool schemas.
MCP is a caller adapter and does not define the isolation boundary.

Related: [Backend](#backend), [execution receipt](#execution-receipt).

### Measurement boundary

A Measurement boundary states exactly when an operation timer starts and stops.
Pulling an OCI image, building a Generation, preparing an unassigned worker, launching a Machine, reaching command readiness, and completing a command are different boundaries.
Results from different boundaries cannot be compared as though they measured the same work.

Related: [ComputeSDK Burst TTI](#computesdk-burst-tti), [preparation class](#preparation-class), [warm path](#warm-path).

### microVM

A microVM is a virtual machine with a deliberately small device and boot surface designed for isolated workloads rather than general-purpose PC emulation.
The word describes a design class and does not by itself prove startup latency, security, density, or snapshot correctness.

Related: [VMM](#vmm), [Hardware-isolated sandbox](#hardware-isolated-sandbox).

### OCI image

An OCI image is a content-addressed configuration and filesystem-layer graph distributed through OCI-compatible registries.
SOMA accepts familiar references such as `ubuntu:24.04` or `node:22`, resolves an exact platform manifest, and later converts that content into a certified Generation.

Related: [OCI index](#oci-index), [OCI layer](#oci-layer), [OCI manifest](#oci-manifest), [Generation](#generation).

### OCI index

An OCI index maps platform selections such as Linux ARM64 or Linux AMD64 to exact manifest descriptors.
Selecting the correct manifest is part of workload identity and cannot be inferred only from a mutable tag.

Related: [OCI image](#oci-image), [OCI manifest](#oci-manifest).

### OCI layer

An OCI layer is one content-addressed filesystem change in an image.
Unchanged layers can be cached and reused across incremental image builds without reusing mutable Machine state.

Related: [OCI image](#oci-image), [Generation](#generation).

### OCI manifest

An OCI manifest binds one platform-specific image configuration and ordered layer set by digest.
Its exact digest is the portable workload identity observed before launch.

Related: [digest binding](#digest-binding), [workload identity](#workload-identity).

### Operation

An Operation is one requested lifecycle transaction such as run, launch, execute, inspect, stop, or destroy.
Every terminal Operation produces a receipt or a typed failure carrying available evidence.

Related: [Operation ID](#operation-id), [execution receipt](#execution-receipt).

### Operation ID

An Operation ID is the caller-controlled identity used to make retries safe.
The same ID with the same canonical request replays retained evidence or returns an explicit replay-unavailable result, while the same ID with different input is a conflict.
An uncertain command is never silently repeated.

Related: [request fingerprint](#request-fingerprint), [durable Machine state](#durable-machine-state).

### Preparation class

Preparation class states how much work existed before an operation entered its measured boundary.
On-demand restore, prepared worker, paused lease, and ready lease are distinct classes.
Performance reports must not combine them without labeling every sample.

Related: [prepared worker](#prepared-worker), [measurement boundary](#measurement-boundary).

### Prepared resource bundle

A Prepared resource bundle is an unassigned set of sterile host resources such as cgroup, namespace, network, disk-head, and control-channel state.
It contains no tenant identity, writable guest data, or reusable guest authority before assignment.

Related: [prepared worker](#prepared-worker), [Host](#host).

### Prepared worker

A Prepared worker is a single-use unassigned VMM process or launch resource whose invariant setup was moved outside the request-time critical path.
Assignment attaches fresh Instance state exactly once, and cleanup destroys the worker instead of scrubbing it for another tenant.

Related: [prepared resource bundle](#prepared-resource-bundle), [warm path](#warm-path).

### Private writable root

The Private writable root is one Instance's disposable mutable filesystem view over immutable Generation storage.
Its requested logical size is part of Machine shape.
It is different from a persistent workspace volume and must not be shared writable across Instances.

Related: [Copy-on-write](#copy-on-write), [workspace volume](#workspace-volume).

### Repair

Repair is the authenticated pre-readiness sequence that replaces cloned identity, entropy, time, network, and transport state after Restore.
A Machine cannot become command-ready until Repair is complete for its exact Instance.

Related: [guest agent](#guest-agent), [Restore](#restore), [command readiness](#command-readiness).

### Request fingerprint

A Request fingerprint is the canonical digest of every field that defines one operation's behavior.
It lets SOMA distinguish an exact retry from Operation ID reuse with changed input.
Machine name, workload identity, shape, command, and limits participate where relevant.

Related: [Operation ID](#operation-id), [execution receipt](#execution-receipt).

### Requested shape

Requested shape is the exact Machine shape supplied by the caller before backend admission.
It remains in the receipt even when a development Backend cannot verify one effective dimension.

Related: [Machine shape](#machine-shape), [effective shape](#effective-shape).

### Remote engine

A Remote engine is an authenticated SOMA execution endpoint reached through a portable Client adapter when the caller cannot or should not run a local engine.
It must preserve operation identity, bounds, lifecycle semantics, and receipt validation rather than silently selecting weaker isolation.
The remote transport is an accepted future design and is not implemented in the alpha.

Related: [client adapter](#client-adapter), [Backend](#backend), [execution receipt](#execution-receipt).

### Restore

Restore recreates a Machine from certified immutable Generation state without a general cold boot from power-on.
Restore is not Ready until the guest authenticates, completes Repair, and passes the first-command readiness gate.

Related: [Generation](#generation), [Snapshot](#snapshot), [Repair](#repair).

### Sandbox

A Sandbox is the user-facing isolated environment in which an agent or program executes work.
In SOMA's first stable contract, every local Sandbox maps to one hardware-isolated Machine.
The term does not imply a specific provider API or weaken the underlying Machine lifecycle.

Related: [Machine](#machine), [Hardware-isolated sandbox](#hardware-isolated-sandbox).

### Snapshot

A Snapshot is captured memory and machine state contained within a certified Generation.
Snapshot bytes are hostile input until format, integrity, compatibility, and artifact identity checks pass.
A Snapshot is not itself a Generation because it does not name every root, kernel, agent, and compatibility artifact.

Related: [Generation](#generation), [Restore](#restore).

### SOMA

SOMA is the Secure Optimized Machine Architecture by MIOSA.
The model is the mind, while SOMA is the disposable machine body that executes its work.
Technically, SOMA is an open-source hardware-isolated sandbox runtime and custom VMM architecture for Linux workloads from OCI inputs.

Related: [sandbox](#sandbox), [VMM](#vmm), [Generation](#generation).

### State store

A State store atomically persists bounded facade-owned Machine documents by Instance ID.
It supplies create-if-absent, load, and revisioned compare-and-swap semantics across processes.
The local implementation is file-backed, while a future fleet may implement the same contract with a durable database.

Related: [durable Machine state](#durable-machine-state), [Instance ID](#instance-id).

### Terminal status

Terminal status classifies the final outcome of one Operation, including Ready, exited, signaled, timed out, output-limit exceeded, inspected, stopped, destroyed, or failed.
Terminal status and cleanup evidence are separate because a command can finish while cleanup remains incomplete.

Related: [execution receipt](#execution-receipt), [cleanup evidence](#cleanup-evidence).

### VMM

Virtual Machine Monitor is the host software that creates, configures, runs, and stops a virtual machine through a virtualization interface such as KVM.
SOMA's production design uses one constrained native `soma-vmm` process per Machine.
A VMM is one technical layer inside the complete sandbox product.

Related: [KVM](#kvm), [microVM](#microvm), [Backend](#backend).

### Warm path

A Warm path reuses immutable artifacts, cached pages, or sterile prepared resources to reduce request-time work.
It may not reuse tenant identity, writable state, or guest authority across Instances.
Warm performance must always name the exact preparation class and cache state.

Related: [prepared worker](#prepared-worker), [preparation class](#preparation-class), [measurement boundary](#measurement-boundary).

### Workload identity

Workload identity binds an exact OCI platform manifest digest, platform, and optional certified Generation ID.
It is stronger than a human image tag and remains separate from Instance identity.

Related: [OCI manifest](#oci-manifest), [Generation ID](#generation-id), [digest binding](#digest-binding).

### Workspace volume

A Workspace volume is separately owned persistent project data that may outlive one disposable Instance.
It needs explicit size, mount, durability, snapshot, sharing, and cleanup semantics.
It is not silently included in writable-root sizing and is outside the current alpha implementation.

Related: [private writable root](#private-writable-root), [Machine shape](#machine-shape).

## Source of truth

The shorter canonical naming rules remain in [the architecture naming document](docs/architecture/naming.md).
Accepted design decisions live in [the ADR directory](docs/adr), and implementation status lives in [the roadmap](ROADMAP.md).
External provider facts and hypotheses remain isolated in [the competitor ledger](COMPETITORS.md).
