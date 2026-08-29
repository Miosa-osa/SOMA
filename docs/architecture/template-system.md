# SOMA template system

The binding architectural decision is [ADR 0022](../adr/0022-compose-templates-into-generation-locks.md).

A SOMA Template is a small, composable preparation recipe.
It is not a running sandbox, a mutable virtual machine, or an artifact on the Launch critical path.
SOMA compiles a resolved Template into one immutable, content-addressed Generation before Launch.

This separation keeps the authoring experience easy without making the VMM understand package managers, agent brands, credentials, or mutable image tags.

## The complete model

```text
human-friendly inputs
        |
        v
+----------------------+     resolve and validate     +----------------------+
| Template             | ---------------------------> | Template Lock        |
| base + modules       |                              | exact inputs         |
+----------------------+                              +----------------------+
                                                              |
                                                              | build and certify
                                                              v
                                                       +----------------------+
                                                       | Generation           |
                                                       | immutable artifacts  |
                                                       +----------------------+
                                                              |
                                                              | Launch with
                                                              | fresh state
                                                              v
                                                       +----------------------+
                                                       | Instance             |
                                                       | running sandbox      |
                                                       +----------------------+
```

The Template answers what should be prepared.
The Template Lock answers exactly which inputs were selected.
The Generation answers which verified bytes and compatibility contract are ready to launch.
The Instance is the fresh sandbox created from that Generation.

## Composition instead of inheritance

SOMA Templates compose a base workload with focused modules.
They do not use an open-ended inheritance tree.

```text
base workload
  + agent module
  + tools module
  + workspace module
  + network module
  + environment contract
  + secret references
  + lifecycle policy
  + resource profile
  = resolved Template
```

Each module owns one concern and has its own schema version.
Modules are reusable across Templates.
Module order is explicit and affects identity whenever order affects the resulting filesystem or configuration.
Two modules that write incompatible values to the same exclusive field produce a validation error instead of silently choosing a winner.

The first version should support a flat ordered module list rather than nested inheritance.
This makes the complete result inspectable and prevents distant parent changes from silently altering a sandbox.

## Minimum Template document

```toml
schema = "soma.template/v1alpha1"
name = "claude-code-python"

[workload]
image = "python:3.12-slim"
platform = "linux/amd64"

modules = [
  "soma://agent/claude-code@1",
  "soma://tools/git@1",
]

[command]
program = "claude"
args = []
working_directory = "/workspace"

[resources]
vcpus = 2
memory_mib = 2048
writable_storage_mib = 10240

[network]
egress = "deny"
allow_domains = ["api.anthropic.com"]
ingress = "deny"

[lifecycle]
idle_timeout_seconds = 900
maximum_lifetime_seconds = 14400
on_idle = "destroy"

[[environment]]
name = "CI"
value = "true"

[[secrets]]
name = "ANTHROPIC_API_KEY"
source = "secret://anthropic/default"
delivery = "environment"
```

Mutable tags such as `python:3.12-slim` are authoring conveniences only.
Resolution must pin the exact OCI manifest digest and platform in the Template Lock before a Generation build begins.

## Focused modules

### Workload

The workload selects the base OCI filesystem and platform.
It supplies operating-system files, language runtimes, package managers, libraries, and default process metadata.
Selecting a Node image makes Node available.
SOMA does not install Node during Launch.

### Agent

An agent module installs or stages one agent and declares its executable, required environment names, optional network destinations, health probe, and supported architectures.
Claude Code, OSA, Hermes, Codex, and future agents use the same module contract.
An agent module is convenience configuration, not privileged VMM logic.

Multiple agent modules may coexist when their files, ports, process names, and environment contracts do not conflict.
The Template must still declare one default command, while callers may Execute other installed agents later.

### Tools

A tool module adds a focused capability such as Git, a shell, browser automation, or a compiler.
Tool modules should be narrow so users do not need one oversized `ai-agent` image containing every ecosystem.

### Workspace

The workspace module declares paths and ownership, then classifies content as immutable build input, launch-time upload, ephemeral writable data, or separately owned persistent storage.
Host paths are never mounted implicitly.

### Environment

Ordinary environment values may be stored in the Template when they are not secret.
The schema distinguishes a literal value from a required name and a secret reference.
Environment values supplied at Launch may fill declared slots but may not introduce forbidden names or override sealed values.

Receipts may record environment names and policy decisions but must never contain secret values.

### Secrets

A Template stores secret references, never secret values.
The initial delivery modes are:

- `environment` for programs that must read a credential directly.
- `file` for programs that require a credential file with controlled ownership and mode.
- `egress-proxy` for credentials injected outside the guest and scoped to approved destinations.

The egress-proxy mode provides the strongest containment when the upstream protocol can be safely mediated because the secret never enters the guest.
Secret delivery happens after fresh Instance identity is established and is never captured in a reusable snapshot.

### Network

Network intent is deny, allowlist, or unrestricted.
The secure default for agent modules and MCP servers is deny-all egress and deny-all ingress.
Users explicitly allow DNS behavior, domain destinations, IP or CIDR destinations, protocols, ports, and public ingress.

Template network policy is a maximum permission envelope.
A Launch request may narrow it.
A Launch request may not widen it without a separately authorized policy decision.

Domain allowlists require DNS rebinding defenses, cloud metadata blocking, IPv4 and IPv6 parity, explicit UDP and QUIC behavior, and enforcement outside the guest.
An HTTP Host header or TLS SNI check alone is not a complete network boundary.

### Lifecycle

Lifecycle policy separates idle timeout from absolute lifetime.
The initial terminal actions are destroy, stop, and checkpoint when the selected Backend proves the required semantics.
Cleanup remains idempotent even if the client disconnects or the agent process crashes.

### Resources

Resource profiles provide editable defaults for vCPU count, memory, writable storage, process count, open files, and output limits.
The effective Launch shape remains explicit and is recorded in the receipt.
Host admission may reject a shape but must not silently reduce it.

## Agent launch sequence

```text
Claude Code, OSA, Hermes, Codex, or another controller
        |
        | select Template and supply declared inputs
        v
resolve modules -> lock exact digests -> use certified Generation
        |
        | Launch
        v
fresh Instance -> repair identity -> attach private network and storage
        |
        | deliver allowed environment and secrets
        v
authenticated soma-guest Ready
        |
        | start selected agent command
        v
stdio, PTY, MCP, or bounded Execute transport
        |
        | disconnect, timeout, or explicit stop
        v
idempotent cleanup and execution receipt
```

The external agent or coding client remains outside the sandbox unless the Template explicitly installs that agent inside the workload.
`soma-guest` is infrastructure and is never the user-selected coding agent.

## Build and Launch boundaries

Template resolution, OCI pulls, package installation, file copying, vulnerability policy, kernel selection, snapshot capture, and certification occur before Launch.
Launch consumes only a ready compatible Generation plus fresh Instance-specific inputs.
This keeps user convenience away from the millisecond fast path.

Build state is explicit:

```text
draft -> resolving -> building -> certifying -> ready
             |            |           |
             +----------> failed <-----+
```

A ready Generation is immutable.
Changing any content-affecting Template field produces a new Template Lock and normally a new Generation identity.
Rebuilding the same locked inputs must either reproduce the expected identity or fail certification.

## What SOMA should reuse

SOMA should reuse OCI images, OCI distribution, content-addressed storage, established package managers during build, rust-vmm components, Linux KVM, Linux network primitives, and standard agent transports.
SOMA should own only the contracts needed for deterministic composition, immutable preparation, isolation, launch, policy enforcement, evidence, and cleanup.

This avoids rebuilding Dockerfile ecosystems or agent installers while keeping the runtime independent of Docker Engine.

## Required validation

A Template compiler must reject:

- Mutable image input that cannot be resolved to an exact digest.
- Unsupported architecture or incompatible agent module.
- Duplicate exclusive file ownership or conflicting default commands.
- A secret literal in a committed Template.
- A secret reference without a declared delivery and destination scope.
- Network permissions wider than an organization's policy ceiling.
- An agent command whose executable is absent from the resolved filesystem.
- An invalid working directory, user, ownership mode, port, timeout, or resource dimension.
- A lifecycle action unsupported by the selected Backend.
- A module graph containing a cycle or an unpinned transitive input.

Validation must explain the exact module and field responsible for a conflict.

## Implementation boundary

Template parsing, module resolution, lock generation, and build planning belong in the preparation plane beside `soma-generation`.
They do not belong in `soma-vmm`, `soma-kvm`, or the Launch request.
The portable public surface may accept a Template reference for convenience, but placement resolves it to an exact certified Generation before contacting a VMM.

The first implementation slice should compile one local Template document into a canonical Template Lock without building a VM.
The next slice should resolve an OCI digest and validate one agent module.
Later slices add deterministic filesystem construction, Generation certification, registry publication, and remote resolution.

## Clean-room research note

Declaw's public documentation demonstrates useful product behaviors such as named rootfs templates, immutable completed builds, environment values, network allowlists, lifecycle timeouts, and an MCP wrapper.
SOMA adopts independently designed contracts and implementation boundaries rather than copying proprietary implementation code.
The detailed evidence and differences are recorded in [Declaw research](../research/declaw.md).
