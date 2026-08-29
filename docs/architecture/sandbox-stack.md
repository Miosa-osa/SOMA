# What makes one SOMA sandbox

This guide explains which piece sits on top of which piece, what connects them, which pieces are inside the sandbox, and which pieces merely prepare or support it.
It describes one production SOMA sandbox architecture rather than several competing sandbox designs.

## The shortest correct definition

A production SOMA sandbox is one hardware-isolated Linux virtual machine created and owned by one SOMA VMM process.
The virtual machine contains private compute, memory, storage, devices, guest identity, a control agent, and the selected user workload.
Host-side modules prepare, isolate, connect, observe, and eventually destroy that machine.

## Read the stack from the hardware upward

```text
+------------------------------------------------------------------+
| USER WORKLOAD                                                    |
| Claude Code, Codex, OSA, Hermes, Node, Python, shell, or binary   |
+------------------------------------------------------------------+
                              |
                              | started and supervised by
                              v
+------------------------------------------------------------------+
| soma-guest                                                       |
| readiness, authenticated commands, output, signals, shutdown     |
+------------------------------------------------------------------+
                              |
                              | runs inside
                              v
+------------------------------------------------------------------+
| GUEST USER SPACE                                                 |
| root filesystem, libraries, runtimes, tools, writable workspace  |
+------------------------------------------------------------------+
                              |
                              | system calls
                              v
+------------------------------------------------------------------+
| GUEST LINUX KERNEL                                               |
| processes, memory, filesystems, networking, virtio drivers       |
+------------------------------------------------------------------+
                              |
                              | uses virtual hardware
                              v
+------------------------------------------------------------------+
| VIRTUAL MACHINE                                                  |
| vCPUs | private RAM | machine state | virtual devices             |
+------------------------------------------------------------------+
                              |
                              | constructed and owned by
                              v
+------------------------------------------------------------------+
| SOMA VMM PROCESS                                                 |
| maps memory, configures devices, runs vCPUs, owns lifecycle       |
+------------------------------------------------------------------+
                              |
                              | requests virtualization from
                              v
+------------------------------------------------------------------+
| LINUX KVM                                                        |
| kernel interface that creates and executes virtual CPUs          |
+------------------------------------------------------------------+
                              |
                              | implemented by
                              v
+------------------------------------------------------------------+
| HOST LINUX KERNEL                                                |
| scheduling, memory, files, networking, cgroups, namespaces       |
+------------------------------------------------------------------+
                              |
                              | controls
                              v
+------------------------------------------------------------------+
| PHYSICAL HARDWARE                                                |
| CPU virtualization | physical RAM | disks | network adapters      |
+------------------------------------------------------------------+
```

Every layer uses the layer below it.
The user workload does not talk directly to KVM or physical hardware.
Guest Linux translates application operations into kernel and device operations.
The VMM translates virtual-machine operations into bounded host operations.

## Containment versus dependency

Two statements can both be true:

- `soma-guest` is physically inside the virtual machine.
- `soma-guest` depends on the host-side VMM for its authenticated control transport.

Physical containment answers where bytes and processes live.
Dependency answers which lower layer or peer a piece requires to work.

```text
HOST MACHINE
|
+-- SOMA VMM process
|   +-- private guest-memory mapping
|   +-- virtual-device implementations
|   `-- vCPU execution through KVM
|
+-- host isolation
|   +-- VMM jail
|   +-- cgroup
|   +-- network namespace
|   `-- private storage resources
|
`-- SANDBOX VIRTUAL MACHINE
    +-- guest Linux kernel
    +-- guest user space
    +-- soma-guest
    `-- user workload
```

The sandbox product includes both the isolated VM and the host ownership needed to enforce its lifetime.
Only the lower box labeled sandbox virtual machine is guest-visible.

## The fundamental runtime primitives

A primitive is an irreducible runtime responsibility required for a declared capability or safety property.
A primitive is not necessarily one source file or one Rust crate.

| Primitive | What it provides | What depends on it | Required for every production sandbox? |
|---|---|---|---|
| Physical virtualization support | Hardware execution mode for guests | KVM | Yes |
| Host Linux kernel | Processes, memory, files, namespaces, networking | KVM, VMM jail, brokers | Yes |
| KVM | VM and vCPU execution interface | SOMA VMM | Yes |
| SOMA VMM | One owner for one Machine | Complete sandbox lifecycle | Yes |
| vCPU | Guest instruction execution | Guest Linux and workload | Yes |
| Private guest memory | Isolated guest RAM and machine state | Kernel, processes, snapshot restore | Yes |
| Guest Linux kernel | Process, memory, filesystem, and device management | Guest user space | Yes |
| Immutable root filesystem | Operating-system and workload files | Guest user space | Yes |
| Private writable filesystem | Per-Instance mutable state | Workspace and processes | Yes |
| Authenticated control channel | Host-to-guest lifecycle and commands | `soma-guest` | Yes |
| Fresh entropy | Safe restored randomness and keys | Identity repair and guest programs | Yes |
| `soma-guest` | Repair, readiness, execution, output, shutdown | Product operations | Yes |
| Fresh Instance identity | Separates one lifetime from every clone | Authentication, network, receipts | Yes |
| Cleanup ownership | Revokes and releases every resource | Safe reuse and fleet stability | Yes |
| Virtual network | Guest packet transport | Egress and ingress | No |
| Persistent volume | Data surviving Instance destruction | Stateful workloads | No |
| PTY | Interactive terminal behavior | Interactive agents and shells | No |
| Checkpoint | Later continuation of runtime state | Pause and resume products | No |

## What virtio is

Virtio is the standard connection between guest Linux drivers and virtual devices implemented by the VMM.
It is not the VMM, KVM, the sandbox, or physical hardware.

```text
guest application
      |
guest Linux subsystem
      |
guest virtio driver
      |
===================== virtual-machine seam =====================
      |
SOMA virtual-device implementation
      |
host resource
```

The current machine design uses these device roles:

| Device role | Guest sees | Host provides | Classification |
|---|---|---|---|
| Root block | Read-only disk | Immutable Generation root | Required machine primitive |
| Overlay block | Writable disk | Private Instance disk head | Required product primitive |
| Vsock control | Host communication socket | Authenticated SOMA control transport | Required control primitive |
| Entropy | Randomness device | Fresh host entropy | Required security primitive |
| Network | Ethernet adapter | TAP and isolated host network | Optional capability |

Additional virtio devices are not automatically desirable.
Every device adds guest-facing parsing, state, snapshot compatibility, attack surface, testing, and restore work.
SOMA adds a device only when a required capability cannot be supplied through the existing minimal surface.

## Follow a filesystem operation

```text
user program opens /workspace/result.json
      |
guest Linux filesystem
      |
OverlayFS chooses private writable upper storage
      |
guest virtio block driver
      |
virtio block device in SOMA VMM
      |
private Instance disk head on the Host
```

The immutable lower filesystem can be shared safely because the guest cannot modify it.
Each Instance receives its own writable upper filesystem so two sandboxes never share mutable files accidentally.

## Follow a command

```text
Claude Code, Codex, OSA, Hermes, CLI, or SDK
      |
portable SOMA interface
      |
Host runtime and SOMA VMM
      |
virtio vsock control transport
      |
authenticated soma-guest
      |
new guest process
      |
stdout, stderr, exit status, or signal result
```

This path works without a network device.
Vsock is the private host-to-guest control path, while networking is the guest-to-network packet path.

## Follow a network packet

```text
user workload opens a socket
      |
guest Linux network stack
      |
guest virtio network driver
      |
virtio network device in SOMA VMM
      |
Host TAP device
      |
per-Instance network namespace
      |
routing, DNS, firewall, proxy, and metadata protection
      |
effective egress policy
      |
approved internet or private-network destination
```

The virtual network device is optional.
When networking is enabled, isolation and policy enforcement are mandatory.
Egress is an optional capability built on the network chain.
Safe egress enforcement is a required security primitive whenever egress is enabled.

Ingress follows the reverse direction but begins at an explicitly published and authorized Host endpoint.
Attaching a network device does not automatically authorize public ingress.

## Where the Generation connects

```text
Template source
      |
Template compiler
      |
Template Lock
      |
Generation builder and certification
      |
immutable Generation
      +-- guest kernel
      +-- immutable root filesystem
      +-- soma-guest
      +-- machine and device contracts
      +-- compatibility evidence
      `-- optional snapshot state
              |
              | consumed by
              v
           SOMA VMM
              |
              v
         fresh sandbox VM
```

The Template is a preparation recipe.
The Template Lock is the exact resolved recipe.
The Generation is prepared immutable machine material.
The Instance is the fresh running sandbox lifetime.

Neither the Template compiler nor the registry runs inside the guest.
The VMM never runs a package manager or resolves a mutable image tag during Launch.

## Host modules surrounding one sandbox

```text
                           Host runtime
                                |
        +-----------------------+-----------------------+
        |                       |                       |
        v                       v                       v
prepared allocator       network broker         storage broker
        |                       |                       |
sterile worker           namespace and TAP       private disk head
        |                       |                       |
        +-----------------------+-----------------------+
                                |
                                v
                         SOMA VMM process
                                |
                                v
                         one sandbox VM
```

The allocator, network broker, storage broker, policy compiler, launch-input broker, and Generation store are part of the SOMA runtime system.
They are not processes inside the sandbox.
They reduce the VMM's privileges and keep expensive preparation outside Launch.

## Required core, optional capability, and optimization

These terms describe different reasons a piece exists.

### Required core

Without a required core piece, the production sandbox cannot exist or cannot satisfy its baseline isolation contract.

Examples include KVM, the VMM, vCPU, private memory, guest kernel, root storage, writable storage, control, entropy, identity repair, authentication, and cleanup.

### Optional capability

The sandbox can exist safely without an optional capability.
Selecting the capability activates additional required primitives and policy.

Examples include networking, egress, ingress, persistent volumes, PTY, checkpointing, browser support, and future GPU support.

### Optimization

An optimization changes how efficiently SOMA creates the same sandbox without changing its product identity.

Examples include snapshot-backed memory, prepared workers, preallocated network bundles, Generation caching, copy-on-write disk heads, and bounded admission pools.

An optimization must never reuse tenant identity, writable state, network authority, or authenticated session secrets.

## Capability activation rule

```text
capability disabled
      |
      `-- related device and policy may be absent

capability enabled
      |
      +-- required device or transport
      +-- isolation mechanism
      +-- policy enforcement
      +-- lifecycle ownership
      +-- cleanup
      `-- evidence
```

Networking is optional.
If networking is enabled, network isolation, policy, cleanup, and evidence are mandatory.

Egress is optional.
If egress is enabled, destination enforcement, metadata protection, protocol semantics, cleanup, and evidence are mandatory.

A persistent volume is optional.
If one is attached, ownership, authorization, mount policy, detach behavior, crash recovery, and deletion semantics are mandatory.

## What is a module

A module is an implementation responsibility hidden behind a small interface.
A primitive describes required runtime behavior, while a module describes how the code owns behavior.
One module may implement several closely related primitives, and one primitive may require cooperation between modules.

```text
runtime responsibility                 likely owning module

KVM access and machine mechanics       soma-kvm
one Machine lifecycle                  soma-vmm
guest protocol                         soma-guest
guest PID 1 behavior                   soma-guest-agent
OCI and Generation preparation         soma-generation
portable sandbox operations            soma facade
local Backend composition              soma-local
human command interface                soma-cli
agent MCP interface                    soma-mcp
```

Template modules are a different use of the word.
A Template module is a reusable user-facing recipe contribution such as an agent, tool, workspace convention, or policy request.
Template modules compile into a Generation and do not become privileged VMM modules.

## What the tickets mean

Tickets are implementation and verification slices.
They are not separate sandbox products and they do not automatically correspond one-to-one with runtime primitives or Rust crates.

```text
one SOMA sandbox architecture
      |
      +-- machine tickets
      +-- device tickets
      +-- guest tickets
      +-- storage tickets
      +-- network tickets
      +-- security tickets
      +-- Template and Generation tickets
      +-- lifecycle tickets
      +-- performance tickets
      `-- evidence tickets
```

All those tickets converge on one result: a fresh isolated command-ready SOMA sandbox.

## Test your understanding

Use these questions when reviewing a new SOMA feature:

1. Is this piece physically inside the guest, on the Host, or in the preparation and control plane?
2. Which lower piece does it depend on?
3. Which higher piece depends on it?
4. Is it required for every sandbox, required only when a capability is enabled, or only an optimization?
5. Which deep module owns its behavior?
6. What authority or mutable state does it hold?
7. How is it cleaned up after partial failure or client disappearance?
8. Does it add work to the measured Launch path?
9. What evidence proves that it worked and remained isolated?

If those questions do not have precise answers, the feature is not designed completely.
