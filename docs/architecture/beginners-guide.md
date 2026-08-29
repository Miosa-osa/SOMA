# From hardware to an agent sandbox

This guide builds one mental model from the bottom up.
It is for readers who need to understand what the sandbox actually is, which SOMA component creates it, where a Template fits, and how an agent reaches the workload.
Use the [SOMA visual atlas](visual-atlas.md) for exploded machine views, filesystem anatomy, workload layering, and source-tree assembly diagrams.

## The shortest correct answer

The actual sandbox is the isolated environment that runs the workload.
On the production KVM path, that environment is one Linux virtual machine with private memory, virtual CPUs, devices, storage, networking, guest identity, and a bounded control channel.

`soma-vmm` is the host process that constructs and owns one such Machine.
`soma-kvm` is a lower-level Rust library used by `soma-vmm` to ask Linux KVM to create and run it.
Linux KVM asks the processor's virtualization hardware to execute guest code safely.

These are related, but they are not synonyms.

```text
agent workload
      |
Linux guest operating system
      |
one SOMA Machine and Instance lifetime       <- the actual KVM sandbox
      |
soma-vmm process                             <- constructs and owns it
      |
soma-kvm library                             <- safe, narrow KVM mechanics
      |
Linux KVM                                    <- host kernel virtualization API
      |
CPU virtualization hardware                 <- physical foundation
```

There are therefore three useful answers to "what is the foundation?"

| Viewpoint | Foundation |
|---|---|
| Physical machine | CPU virtualization hardware and the Linux kernel |
| SOMA production KVM implementation | `soma-kvm` |
| One sandbox's data plane | `soma-vmm`, which owns the complete Machine lifecycle |
| Public product | The stable Launch, Execute, Inspect, Stop, and Destroy contract |

Everything does not literally revolve around one crate.
The production data plane is centered on `soma-vmm`, but the product is centered on a stable lifecycle contract so that KVM, Apple, Docker development, and remote Backends can be used without pretending they have identical isolation.

## The six objects people commonly mix up

| Object | What it is | Does it run? | Mutable? |
|---|---|---:|---:|
| OCI image | Portable workload filesystem and configuration input | No | No |
| Template | User recipe that selects inputs and policy | No | Edited by creating a new revision |
| Generation | Certified immutable launch material | No | No |
| Snapshot | Captured memory and device state inside a Generation | No | No |
| Machine | Hardware-virtualized guest environment | Yes | Yes, during its lifetime |
| Instance | One unique lifetime and identity of a Machine | Yes | Ends permanently |

The simplest relationship is:

```text
Template revision
      |
      | build and certify
      v
Generation
      |
      | Launch
      v
fresh Instance of a Machine
```

A hundred sandboxes can use one Generation without sharing mutable identity or writable state.
The immutable material is reusable.
Memory changes, disk changes, credentials, entropy, networking, and Instance identity must be private to each launch.

## What a Template actually does

Template is the user-facing recipe layer.
It answers questions such as:

- Which OCI image should be used?
- Which command or agent should start?
- How many vCPUs, how much memory, and how much writable storage are requested?
- Which network policy, environment inputs, mounts, and lifetime limits apply?
- Which preparation profile should produce the reusable launch material?

A Template is compiled and certified into a Generation before it enters the fast launch path.
Cold registry downloads, layer extraction, filesystem construction, guest installation, and snapshot capture must not be disguised as millisecond sandbox creation.

```text
BUILD TIME
Template + OCI image + kernel + guest agent + machine profile
                           |
                           v
                    soma-generation
                           |
                           v
Generation = immutable artifacts + compatibility manifest + optional Snapshot

LAUNCH TIME
Launch request + Generation + fresh identity + private resources
                           |
                           v
                       soma-vmm
                           |
                           v
                 authenticated Ready Instance
```

## What happens during Launch

The exact mechanism varies by Backend, but the lifecycle meaning stays stable.

1. The caller submits a validated Launch request referencing a Generation and Machine shape.
2. The selected Backend reserves private memory, writable storage, networking, identity, and process ownership.
3. On Linux KVM, `soma-vmm` uses `soma-kvm` to create the VM, register memory, create virtual CPUs, and restore machine state.
4. `soma-vmm` attaches the minimal virtual devices required by the fixed machine contract.
5. The guest resumes and `soma-guest` repairs cloned identity, entropy, time, network, and transport state.
6. The host and guest establish an authenticated control session.
7. SOMA runs a bounded no-op command inside the guest.
8. Only then does SOMA return Ready with a Receipt.

Stopping is not merely killing a PID.
SOMA must stop execution, revoke authority, release network and storage resources, remove private state, and record cleanup evidence.

## Where each repository module belongs

```text
Public use cases
  crates/soma              stable Launch, Execute, Inspect, Stop, Destroy contract
  crates/soma-cli          human command-line client
  crates/soma-mcp          agent-facing MCP tools

Backend selection
  crates/soma-local        chooses a supported local Backend and fails closed
  crates/soma-macos        Apple development Backend

Production KVM data plane
  crates/soma-vmm          owns one Machine and its lifecycle
  crates/soma-kvm          performs checked Linux KVM operations
  crates/soma-guest        runs inside the guest and proves authenticated readiness

Preparation path
  crates/soma-generation   converts validated workload inputs into a Generation
```

If you want to understand the product, begin at `crates/soma`.
If you want to understand what creates one production KVM sandbox, begin at `crates/soma-vmm` and then follow its narrow calls into `crates/soma-kvm`.
If you want to understand what runs inside that sandbox, read `crates/soma-guest`.
If you want to understand how reusable launch material is built, read `crates/soma-generation`.

## The same lifecycle across different Backends

| Backend | Actual isolated object | Host requirement | Intended role |
|---|---|---|---|
| Linux KVM | One hardware virtual machine per Instance | Linux with usable `/dev/kvm` | Production target |
| Apple | One Apple Virtualization Linux VM per Instance | Supported macOS host | Local hardware-VM development |
| Docker | One Linux container per Instance inside the Docker engine | Docker Desktop or Docker Engine | Convenient local development |
| Remote | A Machine created by a remote SOMA engine | Authenticated network access | Clients without a local engine |

Calling all four a sandbox describes the use case, not identical mechanics.
A Receipt must state the effective isolation and preparation class so callers can distinguish a KVM VM from a container.
SOMA must never silently replace requested hardware isolation with a weaker host process or container boundary.

## How an agent uses the sandbox

An agent should not manage KVM file descriptors, VM memory, TAP interfaces, or snapshots.
It uses the CLI, Rust facade, or MCP server to request a lifecycle operation.

```text
Claude Code, Codex, OSA, Hermes, or another agent
                         |
                 CLI, Rust API, or MCP
                         |
                SOMA lifecycle facade
                         |
                  selected Backend
                         |
                one isolated Instance
                         |
            authenticated guest control
                         |
                  bounded command
```

This is the modular boundary that keeps agent integrations simple while allowing the engine beneath them to evolve.

## What belongs outside the VMM

The VMM creates and owns one Machine.
It should not absorb every concern in a sandbox platform.

| Concern | Correct owner |
|---|---|
| VM creation, memory, vCPUs, devices, execution loop | `soma-vmm` and `soma-kvm` |
| Guest repair and authenticated command execution | `soma-guest` |
| OCI conversion and immutable launch artifacts | `soma-generation` |
| User and agent lifecycle interface | `soma`, CLI, and MCP |
| Tenant accounts, billing, regional placement, quotas | External control plane |
| Host inventory, admission, pools, fleet scheduling | Node allocator and external control plane |
| Cloud-specific installation and networking | Deployment adapters |

At fleet scale, a control plane chooses a suitable host and sends it a Launch request.
The host-local data plane remains responsible for producing one truthful Ready Instance.
Keeping those responsibilities separate prevents a VMM from becoming a god process and prevents cloud policy from leaking into the machine engine.

## A practical reading order

1. Read this guide to establish the objects and layers.
2. Open the [visual atlas](visual-atlas.md) to see the machine, guest filesystem, Node layer, and codebase as nested shapes.
3. Read [SOMA naming](naming.md) for exact technical vocabulary.
4. Read the [module map](module-map.md) to navigate the source tree.
5. Read the [architecture diagrams](diagrams.md) for one-shot, burst, managed, customization, and security flows.
6. Read [local sandbox reality](local-sandbox-reality.md) before comparing macOS, Docker, and Linux KVM results.
7. Read the [fast path](fast-path.md) to understand which work must be prepared before a millisecond Launch.

## The rule to remember

A Template describes what should be prepared.
A Generation is the immutable prepared material.
An Instance is the fresh running lifetime.
`soma-vmm` owns that lifetime.
`soma-kvm` supplies the production Linux virtualization mechanics.
The VM is the sandbox, while the surrounding SOMA modules make that sandbox safe, reusable, observable, and usable by agents.
