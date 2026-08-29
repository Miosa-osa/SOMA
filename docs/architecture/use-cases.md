# SOMA use-case architecture

## Design rule

SOMA organizes its public library around caller outcomes rather than hypervisor mechanisms.
A caller asks to run work, manage a workspace, branch an evaluation, or retain evidence.
The library owns the lifecycle required to produce that outcome and delegates hardware details to a capability-gated backend.

The public surface must stay small enough for humans and agents to discover without learning KVM, Virtualization.framework, snapshot formats, TAP devices, cgroups, or provider APIs.
Internal modules remain deep and cohesive so performance and security mechanisms can evolve without changing every caller.

## Stable use cases

### Run one isolated command

Input:

- An OCI image reference or certified Generation.
- A validated CPU, memory, storage, and capability shape.
- One bounded direct command.
- Backend and preparation policy.

Outcome:

- Resolve the image to an immutable platform digest.
- Create a fresh hardware-isolated Machine.
- Reach authenticated command readiness.
- Execute the command without an implicit shell.
- Stop and clean every owned resource.
- Return one evidence-carrying execution receipt.

This is the primary agent, automation, and benchmark path for the first stable release.

### Manage an isolated workspace

Input:

- The same immutable workload and shape contract.
- An explicit lifetime policy.
- Zero or more bounded Execute operations.

Outcome:

- Launch one Machine and return a managed handle.
- Inspect its stable identity and effective capabilities.
- Execute multiple direct commands against the same authenticated guest session.
- Stop or destroy it idempotently.
- Return lifecycle and cleanup receipts for every terminal operation.

This path supports interactive agent sessions and build workflows without creating a different sandbox abstraction.

### Execute remotely from any supported client OS

Input and outcome match the local use cases.
The portable client sends bounded versioned operations to an authenticated SOMA endpoint and preserves idempotency across disconnects.
Remote transport does not weaken isolation, readiness, output bounds, cleanup, or receipt semantics.

This is the universal path for Windows, unsupported local hosts, thin clients, and fleet control planes.

## Planned use cases

### Branch an evaluation tree

A caller creates multiple fresh Instances from one immutable Generation or a newly certified checkpoint.
Branches never share writable memory, disk, identity, entropy, authentication, or network state.
Receipts let an evaluator prove that every branch used the same source Generation and a comparable preparation class.

### Run a browser or computer session

A longer-lived Machine exposes explicitly authorized ports, display transport, input, and persistent-volume leases.
The browser layer remains outside the VMM and cannot redefine Machine readiness or isolation.

### Run CI and builds

A caller attaches source and cache inputs through bounded, content-addressed transfers.
The execution receipt records input identities, image digest, command outcome, and cleanup without embedding source secrets.

### Run accelerators and nested workloads

GPU assignment, confidential computing, nested virtualization, and other device classes require explicit capability contracts and independent conformance evidence.
They do not enter the first stable CPU sandbox contract through a generic device map.

## Library shape

The intended dependency direction is:

```text
human, agent, SDK, or control plane
                 |
                 v
        soma portable facade
        | run use case
        | machine use case
        | backend selection
        | receipt construction
                 |
        +--------+---------+
        |                  |
        v                  v
 local backend       remote backend
        |
        +--------------------------+
                                   v
                           soma-vmm contract
                                   |
                          target host modules
```

The initial and planned modules have these owners:

| Module | Responsibility |
|---|---|
| `soma` | Portable use-case facade, backend selection, validated outcomes, and receipt assembly |
| `soma-cli` | Human and agent command-line parsing, rendering, and exit codes over the `soma` facade |
| `soma-vmm` | One Machine's lifecycle, idempotency, readiness, faults, and cleanup semantics |
| `soma-kvm` | Linux x86_64 KVM ownership, restore mechanics, vCPU execution, and target safety invariants |
| `soma-macos` | Development-only Apple VM-per-OCI lifecycle adapter |
| `soma-host` | Future node-local admission, single-use worker allocation, sterile resource bundles, and replenishment |
| `soma-generation` | Verified bounded OCI-layout import now, with root conversion, snapshot capture, certification, and compatibility metadata still to follow |
| `soma-protocol` | Future bounded canonical wire values shared by local and remote transports |
| `soma-guest` | Portable authenticated-session primitives now, with the guest executable, secret injection, repair, and readiness still to follow |

Only modules with implemented depth become crates.
SOMA does not create empty crates to make the tree appear modular.
Generic `utils`, `helpers`, `common`, `manager`, and `core` modules are prohibited.

## Backend contract

A backend supplies capabilities and typed observations, not product policy.
The portable library owns:

- Selection among explicit, automatic, local, and remote policies.
- Request validation and immutable image resolution requirements.
- Use-case transaction ordering and cleanup on partial failure.
- Stable result and execution-receipt construction.
- Redaction and portable error classification.

A backend owns:

- Its real isolation mechanism.
- Target-specific lifecycle operations.
- Effective resource enforcement.
- Bounded command transport.
- Typed timing, identity, isolation, and cleanup observations.

Provider billing, fleet placement, commercial plans, and cloud account credentials remain outside both layers.

## Distinctive execution evidence

Every use case returns the receipt defined by ADR 0008.
This gives SOMA one comparable evidence model across a local Apple VM, the custom Linux KVM engine, and a remote host.
The receipt makes the execution boundary inspectable without exposing backend internals or secrets.

The receipt is part of the use-case result rather than an optional logging integration.
A backend that cannot state its effective isolation class or cleanup outcome cannot report a successful certified execution.

## API quality rules

- Provide a simple path with safe defaults for one-shot execution.
- Make isolation and preparation policy explicit in inspectable results.
- Use typed builders only where they prevent invalid requests.
- Require direct executable paths and never insert an implicit shell.
- Bound time, output, input, and message size before allocation.
- Keep backend-specific options out of the portable request whenever a capability contract can express the requirement.
- Return typed unsupported capability errors instead of silently changing behavior.
- Make cleanup an owned transaction rather than caller folklore.
- Keep CLI behavior as an adapter over library behavior, not a second implementation.

## First stable release boundary

The `1.0.0` library must provide one-shot command execution and managed Machine lifecycle through the custom Linux x86_64 KVM path.
It must produce stable execution receipts and pass the published security, cleanup, OCI, and performance gates.

The macOS adapter supplies real local development and conformance before that production path is complete.
Remote execution and Windows client support are part of the portability architecture, but their release status must be stated truthfully in the support matrix at each release.
