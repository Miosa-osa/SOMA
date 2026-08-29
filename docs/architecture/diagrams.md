# SOMA architecture diagrams

## How to read these diagrams

Solid paths describe implemented alpha boundaries or mandatory contract transitions.
Dashed paths describe accepted production designs that still require implementation and retained evidence.
The diagrams distinguish current behavior from performance targets so an architectural picture is never mistaken for a benchmark result.

## Product and dependency topology

```mermaid
flowchart LR
    Human[Human operator] --> CLI[soma CLI]
    Agent[AI agent] --> MCP[soma-mcp stdio server]
    SDK[Rust caller] --> Facade[soma portable facade]

    CLI --> Local[soma-local shared runtime]
    MCP --> Local
    Local --> Facade
    Local --> Store[(Durable lifecycle state)]

    Local --> Select{Explicit backend selection}
    Select --> Apple[soma-macos adapter]
    Select --> Probe[soma-kvm capability probe]

    Apple --> AppleVM[Apple Virtualization.framework VM per OCI sandbox]
    Probe -. production implementation .-> VMM[soma-vmm]
    VMM -. Linux x86_64 KVM .-> LinuxVM[One Linux microVM per Machine]

    Facade --> Receipt[Evidence-carrying execution receipt]
    Store --> Facade
    Apple --> Facade
    VMM -. typed observations .-> Facade
```

The CLI and MCP server differ only at their protocol and rendering boundaries.
Local lifecycle orchestration, durable state, target selection, and evidence mapping remain shared so human and agent behavior cannot drift.
The current Apple path is development-only, and the dashed KVM path remains a production target until its real guest lifecycle passes the required gates.

## Portable cloud and on-premises deployment

```mermaid
flowchart LR
    subgraph Callers[Callers on any supported client OS]
        Workstation[Human workstation]
        Agent[Agent runtime]
        Function[Managed function or CI controller]
    end

    Workstation --> Local{Certified local engine?}
    Agent --> Local
    Function -. authenticated remote request .-> Control[Operator-owned control plane]
    Local -->|yes| LocalEngine[Local capability-gated engine]
    Local -. no, remote configured .-> Control

    Control -. accepted future transport .-> Placement{Certified host placement}
    Placement -.-> AWS[AWS KVM host profile]
    Placement -.-> GCP[Google Cloud KVM host profile]
    Placement -.-> Other[Other cloud KVM host profile]
    Placement -.-> Prem[On-premises KVM host profile]

    AWS --> Machine[One VMM process per Machine]
    GCP --> Machine
    Other --> Machine
    Prem --> Machine
    LocalEngine --> Machine
    Machine --> Receipt[Same validated execution receipt]
```

The public caller interface does not change with host placement.
Cloud and on-premises engine support attaches to an exact certified host profile, while environments without virtualization authority remain clients.
The remote transport and production KVM profiles are dashed because they are accepted designs rather than implemented alpha claims.
See the [deployment portability contract](../operations/deployment-portability.md) for admission requirements and evidence levels.

## One-shot execution contract

```mermaid
sequenceDiagram
    autonumber
    participant Caller
    participant Facade as soma facade
    participant Backend as selected backend
    participant Image as OCI registry or cache
    participant Machine as fresh Machine

    Caller->>Facade: Run(operation, instance, image, shape, command, limits)
    Facade->>Backend: Resolve exact platform workload
    Backend->>Image: Pull or reuse, then inspect
    Image-->>Backend: Index, manifest digest, and platform
    Backend-->>Facade: Workload identity and binding strength
    Facade->>Backend: Launch exact Instance and requested shape
    Backend->>Machine: Create isolated VM
    Backend->>Machine: Verify ownership and effective properties
    Backend-->>Facade: Ready observation
    Facade->>Backend: Execute direct bounded argv
    Backend->>Machine: Run without an implicit host shell
    Machine-->>Backend: Status and bounded exact bytes
    Backend-->>Facade: Command observation
    Facade->>Backend: Clean every owned resource
    Backend-->>Facade: Cleanup evidence
    Facade-->>Caller: Output plus validated execution receipt
```

The portable transaction never treats process creation alone as command readiness.
The production KVM backend must add authenticated guest Repair and a successful authenticated first command before its Ready observation.
The Apple backend reports only the properties that Apple Container 1.3 can actually enforce or verify.

## Target path for a 100-sandbox burst

```mermaid
flowchart TB
    subgraph Preparation[Outside the measured create boundary]
        OCI[OCI platform digest] --> Build[Generation construction and certification]
        Build --> Generation[Immutable Generation]
        Generation --> Shards[Sharded prepared-worker allocator]
        Resources[Sterile cgroup, network, disk-head, and control bundles] --> Shards
        Shards --> Reserve[Single-use unassigned workers]
    end

    Burst[100 concurrent create and first-command requests] --> Admit[Bounded host admission]
    Admit --> Fanout{Parallel fan-out with no silent retry}
    Reserve --> Fanout

    subgraph Measured[Measured independently for every request]
        Fanout --> Lease[Atomically lease one worker and resource bundle]
        Lease --> Identity[Attach fresh Instance, entropy, network, disk, and authority]
        Identity --> Restore[Private copy-on-write memory and disk restore]
        Restore --> Repair[Authenticate guest and complete Repair]
        Repair --> First[Execute first bounded command]
        First --> Ready[Record command-ready result]
        Ready --> Cleanup[Destroy single-use Machine and verify cleanup]
        Cleanup --> Sample[Emit one receipt and one latency sample]
    end

    Sample --> Cohort{Exactly 100 terminal samples}
    Cohort --> Report[Median, p95, p99, failures, cleanup, and raw evidence]
    Shards --> Refill[Asynchronous replenishment]
    Refill --> Reserve
```

This is the accepted production fast-path design, not a current performance claim.
Preparation may move invariant work outside the timer, but it cannot carry tenant identity, mutable guest state, or reusable guest authority.
Every request still receives a unique Instance, private writable state, an authenticated first command, cleanup evidence, and an included sample.
The authoritative measurement rules are in the [benchmark contract](../benchmark-contract.md), while provider facts and unknowns remain in [the competitor ledger](../../COMPETITORS.md).

## Durable managed lifecycle

```mermaid
stateDiagram-v2
    [*] --> Launching: Persist launch intent before side effects
    Launching --> Active: Verified Ready receipt committed
    Launching --> Terminating: Recovery cannot prove Ready
    Active --> Executing: Compare-and-swap owns command
    Executing --> Active: Terminal command evidence committed
    Executing --> Terminating: Outcome uncertain or Machine invalidated
    Active --> Terminating: Stop or destroy intent committed
    Terminating --> Terminal: Owned cleanup verified
    Terminal --> Terminal: Exact retained retry replays
```

Every arrow is a durable revisioned compare-and-swap rather than an in-memory assumption.
A process crash in `Executing` never authorizes a second copy of an uncertain command.
A crash in `Terminating` resumes idempotent cleanup, and a corrupt or unsupported state record fails closed.

## Reproducible customization and replacement

```mermaid
flowchart LR
    Base[Base OCI image] --> Layers[Content-addressed OCI layers]
    Source[Dockerfile and build inputs] --> Layers
    Layers --> Manifest[Exact platform manifest digest]
    Manifest -. future certification pipeline .-> Generation[Certified immutable Generation]

    Shape[Requested vCPU, memory, storage, and network policy] --> Request[Launch request]
    Name[Optional human Machine name] --> Request
    Manifest --> Request
    Generation -. when available .-> Request
    Request --> Instance[Fresh globally unique Instance]
    Instance --> Root[Disposable private writable root]
    Workspace[Separately owned workspace volume] -. explicit future attachment .-> Instance

    Change[Change image input or Machine shape] --> NewManifest[New digest or request fingerprint]
    NewManifest --> Replacement[New Instance, never in-place mutation]
```

Incremental customization reuses unchanged OCI layers and later Generation artifacts without reusing mutable Machine identity.
CPU, memory, storage, image, and network changes create a replacement Instance.
Persistent workspace data remains a separate lifecycle and ownership contract rather than being confused with the disposable root size.

## Security boundary summary

```mermaid
flowchart LR
    Caller[Untrusted caller input] --> Validate[Portable validation and bounds]
    Validate --> State[Durable intent and operation ownership]
    State --> Adapter[Target adapter]
    Adapter --> VMBoundary[Hardware VM boundary]
    VMBoundary --> Guest[Hostile guest workload]
    Guest --> Bounded[Authenticated and bounded command channel]
    Bounded --> Evidence[Typed observations]
    Evidence --> Receipt[Validated receipt]

    Guest -. no direct authority .-> State
    Guest -. no host paths or descriptors .-> Caller
```

The threat model treats guest memory, device queues, image metadata, state documents, process output, and retry input as hostile.
The [threat model](../threat-model.md) defines the intended security invariants and current implementation limits.
