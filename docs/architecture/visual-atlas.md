# SOMA visual atlas

This atlas shows the machine as nested shapes and directory trees.
It uses plain text so the architecture remains readable in a terminal, source viewer, screen reader, or printed page.

## 1. The whole system in one picture

```text
+-----------------------------------------------------------------------+
| CALLER                                                                |
| Human CLI | Claude Code | Codex | OSA | Hermes | application          |
+-----------------------------------+-----------------------------------+
                                    | Launch / Execute / Stop
                                    v
+-----------------------------------------------------------------------+
| SOMA PRODUCT INTERFACE                                                |
| crates/soma | soma-cli | soma-mcp                                     |
+-----------------------------------+-----------------------------------+
                                    | select a truthful Backend
              +---------------------+----------------------+
              |                     |                      |
              v                     v                      v
        +-----------+         +-----------+          +-----------+
        | Linux KVM |         | Apple VM  |          | Docker    |
        | production|         | local dev |          | local dev |
        +-----+-----+         +-----+-----+          +-----+-----+
              |                     |                      |
              v                     v                      v
        hardware VM           hardware VM             container
        sandbox               sandbox                 sandbox
```

The word sandbox names the isolated product environment.
The boxes underneath show what actually supplies that isolation on each Backend.

## 2. The Linux KVM production stack

Read this diagram from the bottom upward to understand foundations.
Read it from the top downward to follow a command.

```text
                         AGENT COMMAND
                              |
                              v
  +-----------------------------------------------------------+
  | GUEST WORKLOAD                                            |
  | node /usr/src/app/agent.js                                |
  +-----------------------------------------------------------+
  | GUEST USER SPACE                                          |
  | Node 22 + application files + libraries                   |
  | soma-guest + PID 1 + essential system files               |
  +-----------------------------------------------------------+
  | GUEST LINUX KERNEL                                        |
  | schedules processes, manages guest memory and devices     |
  +-----------------------------------------------------------+
  | VIRTUAL MACHINE                                           |
  | vCPUs | private RAM | block | network | vsock | entropy    |
  | This running Machine and Instance lifetime is the sandbox |
  +-----------------------------------------------------------+
  | soma-vmm                                                  |
  | one host process constructs and owns the Machine          |
  +-----------------------------------------------------------+
  | soma-kvm                                                  |
  | checked Rust interface to Linux KVM                       |
  +-----------------------------------------------------------+
  | HOST LINUX KVM                                            |
  | kernel API for creating VMs, vCPUs, and guest memory      |
  +-----------------------------------------------------------+
  | HOST LINUX KERNEL                                         |
  | processes | cgroups | namespaces | TAP | files | security |
  +-----------------------------------------------------------+
  | PHYSICAL OR NESTED HOST                                   |
  | CPU virtualization | RAM | storage | network interface    |
  +-----------------------------------------------------------+
```

`soma-kvm` is not a separate VM inside `soma-vmm`.
It is library code linked into the `soma-vmm` process.
Linux KVM is not a SOMA process either.
It is a host-kernel interface used by that process.

## 3. The minimum pieces of one KVM sandbox

The smallest useful sandbox is more than a Rust struct and less than a general-purpose PC.

```text
ONE INSTANCE
|
+-- identity
|   +-- unique Instance ID
|   +-- fresh guest session secret
|   +-- fresh entropy and network identity
|
+-- execution state
|   +-- one or more virtual CPUs
|   +-- private guest-memory mapping
|   +-- restored or directly initialized CPU and device state
|
+-- boot material
|   +-- Linux kernel
|   +-- initramfs
|   +-- kernel command line
|   +-- fixed virtual-machine layout
|
+-- storage
|   +-- immutable root filesystem from the Generation
|   +-- private writable filesystem for this Instance
|
+-- devices
|   +-- root block device
|   +-- writable block device
|   +-- network device
|   +-- guest-control device
|   +-- entropy device
|
+-- host resources
|   +-- one soma-vmm process
|   +-- cgroup and process-isolation state
|   +-- TAP and network-policy state
|   +-- owned files and descriptors
|
+-- guest control
    +-- soma-guest process
    +-- authenticated host-to-guest channel
    +-- Repair, Execute, Shutdown, and evidence protocol
```

Removing any required branch either prevents the guest from starting or removes an isolation, lifecycle, or readiness guarantee.

## 4. Where the first file and first directory come from

A Linux machine does not begin with Node.
The host loads the guest kernel separately, and the kernel creates the first user-space process from the initramfs or root filesystem.

```text
HOST-SIDE GENERATION ARTIFACTS
generation/
|-- manifest                 identifies and binds every artifact
|-- kernel                   guest Linux kernel image
|-- initramfs                earliest temporary root filesystem
|-- root.erofs               immutable workload root filesystem
|-- overlay.ext4             blank private writable filesystem template
|-- snapshot.memory          optional prepared guest memory
`-- snapshot.machine         optional vCPU and virtual-device state

GUEST VIEW AFTER START
/
|-- init                     first user-space program, or link to PID 1
|-- bin/                     essential commands when the profile includes them
|-- dev/                     virtual devices exposed by the kernel
|-- etc/                     machine and workload configuration
|-- proc/                    process and kernel view mounted at runtime
|-- run/                     volatile runtime state
|-- sys/                     device and kernel view mounted at runtime
|-- tmp/                     temporary writable state
|-- usr/
|   |-- bin/
|   |   `-- node             present in a Node workload Generation
|   `-- src/app/             example agent application
`-- var/                     writable or overlay-backed runtime data
```

`/` is the root directory of the guest filesystem.
It is not the repository root, host root, or OCI registry layout.
The exact directory contents come from the selected OCI workload plus the small SOMA guest requirements added during Generation construction.

The first user-space process is PID 1.
For the SOMA machine profile, PID 1 must establish required mounts and start or supervise `soma-guest` before arbitrary workload execution is accepted.

## 5. How Node 22 is added

Node is workload content, not virtualization machinery.

```text
node:22 OCI image
|
| layers contain /usr/local/bin/node, libraries, certificates, and metadata
v
soma-generation
|
| validates OCI identity
| normalizes filesystem semantics
| adds or verifies the SOMA guest contract
| builds immutable filesystem artifacts
| optionally captures prepared machine state
v
Node 22 Generation
|
| Launch adds fresh private state
v
Node 22 Instance
|
| Execute ["/usr/local/bin/node", "--version"]
v
bounded output + execution Receipt
```

Ubuntu, Kali, Python, and a custom agent image follow the same preparation shape.
Their user-space files differ, but the Machine contract and lifecycle remain stable.

## 6. Template, Generation, and Instance as physical shapes

```text
TEMPLATE                         GENERATION                    INSTANCE
editable recipe                  sealed reusable box           opened private machine

+------------------+             +------------------+           +------------------+
| image: node:22   |   compile   | exact OCI digest |  Launch   | unique identity  |
| cpu: 1           | ----------> | kernel + rootfs  | --------> | private memory   |
| memory: 1 GiB    |             | guest + snapshot|           | private disk     |
| network: egress  |             | compatibility   |           | private network  |
| ttl: 10 minutes  |             | content ID      |           | running workload |
+------------------+             +------------------+           +------------------+
       recipe                     never runs itself              runs once
```

Editing a Template creates another Template revision.
Building different immutable content creates another Generation.
Launching the same Generation twice creates two different Instances.

## 7. What is shared and what must be private

```text
                         ONE GENERATION
                immutable kernel, rootfs, snapshot
                              /   |   \
                             /    |    \
                            v     v     v
                     Instance A Instance B Instance C
                     private RAM private RAM private RAM
                     private disk private disk private disk
                     identity A   identity B   identity C
                     network A    network B    network C
```

Sharing immutable bytes makes launch fast.
Sharing mutable identity, writable state, credentials, or guest authority would break isolation.

## 8. Source tree as an assembly line

```text
crates/
|
+-- soma-generation/  OCI input -----------------> Generation
|
+-- soma/             public lifecycle contract -+
|                                                  |
+-- soma-cli/         human caller ----------------+--> Launch request
|                                                  |
+-- soma-mcp/         agent caller ----------------+
|
+-- soma-local/       Backend selection -----------> KVM, Apple, Docker, or remote
|
+-- soma-vmm/         owns one KVM Machine --------> running Instance
|       |
|       `-- uses soma-kvm/ ------------------------> Linux KVM
|
+-- soma-guest/       runs inside the guest -------> Ready and Execute evidence
|
`-- soma-macos/       Apple development adapter ---> Apple Linux VM
```

The arrows describe responsibility and data flow, not Rust crate dependency in every case.
The module map remains the authority for allowed code dependencies.

## 9. Build time, launch time, and command time

```text
SLOW OR OCCASIONAL WORK          FAST REPEATED WORK          AGENT WORK

pull OCI image                  reserve prepared worker      Execute command
normalize layers                map private memory           stream bounded output
build filesystems               restore machine state        return Receipt
install guest agent             attach private resources
capture snapshot               repair cloned identity
certify Generation             prove guest Ready

<--------- build time --------><------ Launch time --------><-- command time -->
```

A meaningful 10 ms target applies to a precisely declared Launch boundary on a prepared compatible host.
It cannot honestly include downloading an arbitrary image, constructing its filesystem, installing software, and snapshotting the result.

## 10. What a vCPU actually is

One vCPU is the processor that the guest believes it owns.
To preserve and resume that illusion, the VMM owns its register state, interrupt state, and run loop.

```text
INSIDE INSTANCE A                    ON THE HOST

guest process                        soma-vmm process
     |                                    |
guest Linux scheduler                     +-- vCPU 0 host thread -----+
     |                                                               |
virtual CPU 0 state                                               host Linux
registers, instruction pointer, flags                            scheduler
                                                                     |
                                                                     v
                                                         hardware thread 0..79
```

When the guest has two vCPUs, `soma-vmm` normally owns two vCPU execution threads.
The host scheduler may run them simultaneously on two hardware threads, pause them, or move them between hardware threads.
CPU pinning can restrict that placement when predictable latency matters.

```text
80 HOST HARDWARE THREADS

thread 0  <--- Instance A vCPU 0
thread 1  <--- Instance B vCPU 0
thread 2  <--- Instance C vCPU 0
...
thread 79 <--- Instance Z vCPU 1

Later, after scheduling:

thread 0  <--- Instance D vCPU 0
thread 1  <--- Instance A vCPU 0
thread 2  <--- host networking work
...
thread 79 <--- soma-vmm cleanup work
```

An idle vCPU consumes little CPU time but still has machine state and supporting memory.
An overcommitted host has more runnable guest vCPUs than hardware threads.
That can improve utilization for bursty workloads, but it increases queueing and latency when many sandboxes become busy together.

## 11. One large host divided into sandboxes

Consider this example Host:

```text
+-----------------------------------------------------------------------+
| HOST                                                                  |
| 80 physical cores                                                     |
| 80 hardware threads                                                   |
| 256 GiB RAM                                                           |
| 2 TiB local storage                                                   |
+-----------------------------------------------------------------------+
```

If the machine truly has 80 cores and 80 threads, SOMA treats 80 as the schedulable hardware-thread count.
If simultaneous multithreading instead exposed 160 threads, the schedulable count would be 160, but sibling threads would not provide the same performance as 160 independent physical cores.

SOMA first reserves capacity for the Host itself.
The exact reserve must come from a measured and certified Host profile.
The following numbers are an explanatory example, not a production recommendation or benchmark claim.

```text
EXAMPLE RESERVATION

Host total                    Reserved for host               Admissible pool
80 hardware threads    minus  8 threads                 equals 72 thread units
256 GiB RAM            minus  24 GiB                    equals 232 GiB
2 TiB storage          minus  200 GiB                   equals about 1.8 TiB
```

The reserve pays for the host kernel, `soma-vmm` processes, networking, page cache, filesystem metadata, observability, cleanup, and failure headroom.
It must not be silently allocated to guests.

### Example shape: 1 vCPU, 1 GiB RAM, 10 GiB writable limit

```text
ADMISSIBLE HOST POOL
|
+-- CPU pool:     72 thread units
|   `-- 72 continuously busy 1-vCPU sandboxes at 1:1 allocation
|
+-- memory pool:  232 GiB
|   `-- fewer than 232 active 1-GiB sandboxes after per-VM overhead
|
+-- storage pool: about 1.8 TiB
|   |-- shared Node 22 Generation stored once
|   `-- private writable blocks consumed only as Instances write
|
`-- other gates
    |-- VMM process and file-descriptor limits
    |-- TAP, IP address, route, and firewall capacity
    |-- network bandwidth and packets per second
    |-- snapshot page-fault and storage throughput
    |-- configured burst and cleanup headroom
```

At strict 1:1 CPU allocation, CPU becomes the first obvious limit at 72 busy 1-vCPU sandboxes.
Memory could theoretically admit more mostly idle sandboxes, but the allocator must include per-VM overhead and must never count the entire 232 GiB as guest payload.
Storage is not calculated as `1.8 TiB / 10 GiB` unless every Instance physically preallocates its complete writable limit.
With sparse private overlays, capacity depends on real written blocks plus a reserved safety policy.

### The capacity equation

For one uniform Machine shape, the safe admitted count is bounded by the smallest independent resource limit.

```text
cpu_limit     = floor(admissible_cpu_units / effective_vcpu_units_per_instance)

memory_limit  = floor(admissible_memory_bytes
                      / (guest_memory_bytes + measured_per_vm_overhead))

storage_limit = floor(admissible_private_storage_budget
                      / reserved_private_bytes_per_instance)

network_limit = measured limit for addresses, policy objects, bandwidth, and packets

process_limit = measured limit for VMM processes, threads, descriptors, and kernel objects

safe_count    = minimum(cpu_limit,
                        memory_limit,
                        storage_limit,
                        network_limit,
                        process_limit,
                        operator_safety_limit)
```

The allocator must perform checked arithmetic and atomically reserve every required dimension.
If any dimension cannot be reserved, Launch fails with capacity evidence instead of partially creating a sandbox.

## 12. Shared bytes versus isolated state

Fast density depends on sharing only immutable material.

```text
HOST
|
+-- Generation cache
|   +-- Node 22 root.erofs -------------------- shared read-only --------+
|   +-- prepared snapshot.memory ------------- shared copy-on-write ---+ |
|   `-- kernel and machine metadata ---------- shared read-only ------+| |
|                                                                      || |
+-- Instance A --------------------------------------------------------+| |
|   +-- private memory pages after writes <-----------------------------+ |
|   +-- private writable filesystem                                      |
|   +-- private vCPU state                                                |
|   +-- private network and identity                                      |
|                                                                         |
+-- Instance B -----------------------------------------------------------+
    +-- private memory pages after writes
    +-- private writable filesystem
    +-- private vCPU state
    +-- private network and identity
```

The host may map the same immutable snapshot pages into many VMM processes with private copy-on-write mappings.
The moment one Instance writes a page, that Instance receives a private copy.
This reduces startup copying and physical memory use without allowing one guest to modify another guest's memory.

The same rule applies to storage.
All Instances may read one immutable Generation root, while each Instance receives a separate writable upper filesystem.

## 13. How isolation is assembled

Isolation is not one switch.
It is a stack of independent boundaries.

```text
TENANT WORKLOAD
|
+-- guest privilege boundary
|   `-- workload is constrained inside guest Linux
|
+-- virtual hardware boundary
|   `-- CPU virtualization and KVM separate guest memory and execution
|
+-- VMM device boundary
|   `-- minimal checked virtual devices handle guest-controlled input
|
+-- VMM process boundary
|   `-- one constrained soma-vmm process owns one Machine
|
+-- host resource boundary
|   `-- cgroup limits CPU, memory, process, and I/O consumption
|
+-- host filesystem boundary
|   `-- VMM receives only required files and descriptors
|
+-- network boundary
|   `-- private attachment, routing, ingress, egress, DNS, and metadata policy
|
`-- authority boundary
    `-- fresh authenticated guest session is valid for one Instance only
```

No single layer eliminates every threat.
The Machine boundary isolates guest execution, while the process, resource, network, storage, and authority boundaries constrain what a compromised guest or VMM can reach.

## 14. Why active count and burst count differ

A Host may retain many idle sandboxes but still fail a simultaneous burst if all vCPUs wake, memory pages become private, and networks transmit at once.

```text
IDLE DENSITY                         BURST DENSITY
many vCPUs mostly sleeping          many vCPUs runnable together
many snapshot pages still shared    many pages copied after writes
little network traffic              packet and bandwidth spike
little writable data                storage write and fault spike
                                    cleanup spike after completion
```

SOMA therefore needs separate admission limits for resident Instances, concurrent Launch operations, runnable vCPUs, private dirty memory, network load, and cleanup work.
A raw count of VM objects is not a truthful capacity model.

## 15. The sentence to keep in your head

SOMA prepares a sealed Generation, realizes it as a fresh isolated Instance through a Backend, and gives an agent a bounded authenticated way to execute inside it.
