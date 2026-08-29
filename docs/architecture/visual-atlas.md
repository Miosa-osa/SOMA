# SOMA visual atlas

The canonical explanation of physical containment, dependency direction, required primitives, optional capabilities, and optimizations is [What makes one SOMA sandbox](sandbox-stack.md).

This atlas shows the machine as nested shapes and directory trees.
It uses plain text so the architecture remains readable in a terminal, source viewer, screen reader, or printed page.

## Find the picture you need

- [The whole system](#1-the-whole-system-in-one-picture)
- [The Linux KVM stack](#2-the-linux-kvm-production-stack)
- [The minimum pieces of a sandbox](#3-the-minimum-pieces-of-one-kvm-sandbox)
- [The guest filesystem and its first files](#4-where-the-first-file-and-first-directory-come-from)
- [Where Node, Python, or another runtime comes from](#5-where-node-python-or-another-runtime-comes-from)
- [Template, Generation, and Instance](#6-template-generation-and-instance-as-physical-shapes)
- [What a vCPU is](#10-what-a-vcpu-actually-is)
- [How a Host is divided](#11-one-large-host-divided-into-sandboxes)
- [The incremental capacity ladder](#15-capacity-ladder-from-one-sandbox-to-a-fleet)
- [How 200 sandboxes fit on 80 threads](#how-200-sandboxes-can-fit-on-80-hardware-threads)
- [Density mechanisms and tradeoffs](#density-mechanisms-and-their-tradeoffs)
- [Unsafe density shortcuts](#what-should-not-be-used-casually)
- [The worked 256 GiB Host example](#a-realistic-200-sandbox-shape-on-the-256-gib-example-host)
- [Whether one Host can create 100,000 sandboxes](#16-can-one-host-create-100000-sandboxes)
- [How a fleet reaches 100,000](#how-soma-reaches-100000-active-sandboxes)

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
  | USER-SELECTED PROGRAM                                     |
  | executable and dependencies come from the workload       |
  +-----------------------------------------------------------+
  | GUEST USER SPACE                                          |
  | workload runtime + application files + libraries          |
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
|   |   `-- <runtime>        only when selected workload provides one
|   `-- src/app/             example workload files when provided
`-- var/                     writable or overlay-backed runtime data
```

`/` is the root directory of the guest filesystem.
It is not the repository root, host root, or OCI registry layout.
The exact directory contents come from the selected OCI workload plus the small SOMA guest requirements added during Generation construction.

The first user-space process is PID 1.
For the SOMA machine profile, PID 1 must establish required mounts and start or supervise `soma-guest` before arbitrary workload execution is accepted.

## 5. Where Node, Python, or another runtime comes from

SOMA does not require Node to execute an agent command.
Node is one optional Workload runtime selected by the user.

### Three different things called an agent

```text
EXTERNAL AGENT
Claude Code, Codex, OSA, Hermes, or an application
runs outside the sandbox
        |
        | Launch and Execute request
        v
SOMA GUEST AGENT
soma-guest
runs inside every SOMA Machine
authenticates, repairs, executes, and reports
        |
        | exact executable path and argument vector
        v
USER WORKLOAD PROGRAM
JavaScript agent, Python agent, shell command, Rust binary, or other program
comes from the selected workload
```

The external agent does not need to be installed inside the sandbox just to control it.
The SOMA Guest agent is infrastructure and is unrelated to Node.js.
The user's workload program determines whether Node, Python, Java, a shell, or no language runtime is needed.

### The user selects the workload before Launch

```text
Template or preparation input
        |
        +-- image: node:22 ----------> includes Node.js
        |
        +-- image: python:3.13 ------> includes Python
        |
        +-- image: ubuntu:24.04 -----> includes Ubuntu user space
        |                              does not imply Node.js
        |
        `-- custom OCI image --------> includes exactly what its builder added
```

An OCI image is a stack of filesystem layers plus configuration metadata.
The selected image supplies the workload's files, executables, dynamic loader, shared libraries, certificates, and default environment.
SOMA resolves the mutable image tag to an exact platform-specific digest before constructing a Generation.

### Node 22 example

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

Node exists at `/usr/local/bin/node` in this example because that path came from the selected `node:22` filesystem.
SOMA did not install Node during Launch.

### The same machine without Node

```text
custom OCI image
|
| contains /opt/agent/my-agent as a compatible static executable
| contains no Node.js, Python, or shell
v
custom Generation
|
| Launch
v
custom Instance
|
| Execute ["/opt/agent/my-agent", "--task", "inspect"]
v
bounded output + execution Receipt
```

A native Rust or Go agent can execute directly when its binary and required files are present and compatible with the guest architecture.
There is no reason to install Node for that workload.

### What Execute actually does

```text
external caller
|
| Execute {
|   program: "/usr/local/bin/node",
|   args: ["--version"]
| }
v
soma-guest receives authenticated bounded request
|
| verifies lifecycle, operation identity, path, arguments, output, and time bounds
v
guest Linux attempts direct process execution
|
+-- executable and dependencies valid --> process starts
|
`-- executable or dependency invalid ---> typed execution failure
```

The command is a direct executable path plus an argument array.
SOMA does not assume a shell, package manager, Node, Python, or application framework.

### What happens when the image is wrong

| Requested command | Selected workload contents | Result |
|---|---|---|
| `/usr/local/bin/node --version` | `node:22` contains that executable and its libraries | Node starts |
| `/usr/local/bin/node --version` | plain Ubuntu has no Node at that path | executable-not-found failure |
| `/bin/sh -c ...` | image contains no shell | executable-not-found failure |
| `/opt/agent/run` | file exists but is not executable | permission failure |
| `/opt/agent/run` | binary targets the wrong CPU architecture | executable-format failure |
| `/opt/agent/run` | dynamic loader or required library is absent | loader or dependency failure |
| `/opt/agent/run` | compatible static binary is present | starts without Node or Python |

Readiness proves that the SOMA Guest agent can execute the fixed readiness command.
It does not prove that every later user-selected executable exists.
A Template or Generation certification profile may add workload-specific probes, but those probes must be explicit.

Ubuntu, Kali, Python, Node, and custom agent images follow the same preparation shape.
Their user-space files differ, while the Machine contract and SOMA lifecycle remain stable.

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

## 15. Capacity ladder from one sandbox to a fleet

This lesson changes one variable at a time so the reason for each limit remains visible.
The numbers are explanatory arithmetic, not measured SOMA performance or capacity claims.

### Keep one sandbox shape fixed

Every step uses this illustrative Machine shape:

```text
ONE SANDBOX
  1 vCPU
  512 MiB guest RAM
  64 MiB placeholder for VMM and Host memory overhead
  4 GiB logical private writable disk
  one shared immutable Generation
  one private network identity
```

The memory admission cost used in the examples is therefore:

```text
512 MiB guest RAM + 64 MiB overhead = 576 MiB per resident Instance
```

Real SOMA Host profiles must replace the placeholder overhead with retained measurements.
The actual overhead changes with the kernel, device model, page tables, queue sizes, networking, observability, and workload.

### Stage 1: one sandbox on one Host

```text
HOST
4 hardware threads
8 GiB RAM
|
`-- Instance A
    +-- 1 vCPU
    +-- 512 MiB guest RAM
    +-- private writable disk
    `-- private identity and network
```

Nothing needs to be oversubscribed.
There is also almost no density benefit from sharing because only one Instance uses the Generation.

At this stage, the important lesson is ownership:

```text
physical CPU thread       Host owns it
virtual CPU state         Instance A owns it
guest RAM mapping         Instance A owns it
immutable Generation      Host cache owns it
private dirty pages       Instance A owns them
private writable blocks   Instance A owns them
```

The first failure can still happen before capacity is exhausted.
An incompatible Generation, unavailable KVM, failed network attachment, invalid snapshot, or failed guest authentication must reject Launch even though CPU and RAM are available.

### Stage 2: four sandboxes share one Generation

```text
HOST
|
+-- selected workload Generation stored once
|   +-- example: Node 22 only when the user selected node:22
|   +-- kernel
|   +-- immutable root filesystem
|   `-- prepared snapshot pages
|
+-- Instance A: private vCPU, dirty pages, disk, identity
+-- Instance B: private vCPU, dirty pages, disk, identity
+-- Instance C: private vCPU, dirty pages, disk, identity
`-- Instance D: private vCPU, dirty pages, disk, identity
```

This is where immutable sharing first matters.
Four Instances can map the same verified read-only files and clean snapshot pages instead of storing four independent copies.

What remains private:

- vCPU register and interrupt state
- every modified memory page
- every writable filesystem block
- network identity and policy state
- guest session keys and Instance identity

If one guest modifies a shared clean page, copy-on-write gives that guest a private physical page.
The other three guests continue seeing the original immutable page.

### Stage 3: sixteen sandboxes introduce scheduling

Assume an 8-thread, 16 GiB Host reserves 2 threads and 3 GiB for Host work.

```text
admissible CPU       6 thread units
admissible memory    13 GiB = 13,312 MiB
memory bound         floor(13,312 / 576) = 23 Instances
```

Sixteen resident Instances fit the illustrative memory budget.
They require CPU overcommit:

```text
16 vCPUs / 6 thread units = 2.67:1 CPU overcommit
```

If only four vCPUs are runnable, they can run immediately.
If all sixteen become runnable, the scheduler time-slices them across six thread units.

```text
QUIET MOMENT                         FULL WAKE-UP

4 runnable vCPUs                     16 runnable vCPUs
6 thread units                       6 thread units
no CPU queue                         about 2.67 contenders per unit
low scheduling delay                 queueing and context switching rise
```

Isolation still holds during the full wake-up.
Performance changes because execution time is scarce, not because memory boundaries disappeared.

### Stage 4: sixty-four sandboxes change the likely bottleneck

Assume a 16-thread, 32 GiB Host reserves 2 threads and 4 GiB.

```text
admissible CPU       14 thread units
admissible memory    28 GiB = 28,672 MiB
memory bound         floor(28,672 / 576) = 49 Instances
```

The CPU could admit 56 vCPUs under a 4:1 overcommit policy.
Memory stops this exact 512 MiB shape at 49 before CPU reaches 56.
Sixty-four resident Machines do not fit the guaranteed-memory policy.

There are only four honest choices:

```text
1. admit 49 and reject the rest
2. reduce the requested guest-memory shape
3. add physical RAM or another Host
4. offer an explicit elastic-memory class with weaker guarantees
```

Pretending shared clean pages will always remain shared is not a fifth choice.
A workload update, garbage collection cycle, file cache growth, or simultaneous compilation can dirty memory and destroy the optimistic sharing ratio.

### Stage 5: sixty-four sandboxes on a balanced Host

Increase the Host to 40 threads and 128 GiB RAM.
Reserve 4 threads and 12 GiB.

```text
admissible CPU       36 thread units
admissible memory    116 GiB = 118,784 MiB
memory bound         floor(118,784 / 576) = 206 Instances

64 vCPUs / 36 thread units = 1.78:1 CPU overcommit
```

Now sixty-four resident sandboxes fit comfortably in CPU and memory.
The next limit may move somewhere less obvious:

- Launch page faults may saturate storage reads.
- First writes may cause an OverlayFS copy-up burst.
- Sixty-four TAP interfaces may trigger network setup or policy latency.
- Agent workloads may exhaust outbound connection tracking or bandwidth.
- Sixty-four VMM processes multiply file descriptors and host threads.
- Simultaneous completion may produce a cleanup storm.

This is why adding RAM does not guarantee that every other subsystem scales with it.

### Stage 6: two hundred sandboxes on the 80-thread Host

Use the earlier 80-thread, 256 GiB Host with 72 thread units and 232 GiB admitted after reserves.

```text
memory required      200 * 576 MiB = 112.5 GiB
memory remaining     232 GiB - 112.5 GiB = 119.5 GiB
CPU overcommit       200 / 72 = 2.78:1
```

For this 512 MiB shape, CPU scheduling behavior is likely to become important before guaranteed RAM.
That answer reverses for a 2 GiB guest shape:

```text
2 GiB guest + 64 MiB overhead = 2,112 MiB per Instance
232 GiB / 2,112 MiB = 112 Instances by memory
```

The same Host can therefore admit about 200 of the illustrative 512 MiB shape but only about 112 of the illustrative 2 GiB shape before other gates.
Hardware specifications do not produce one universal sandbox count.
Machine shape and workload behavior are part of the capacity answer.

### Stage 7: a larger Host introduces topology

Doubling cores and RAM does not always double useful capacity.
A large server may have multiple CPU sockets or NUMA nodes.

```text
LARGE DUAL-NUMA HOST

NUMA node 0                            NUMA node 1
CPU threads 0..79                      CPU threads 80..159
local RAM bank A                       local RAM bank B
       |                                      |
       +------------- interconnect -----------+
```

A vCPU running on node 0 reaches node 0 memory faster than remote memory on node 1.
If a VMM's vCPU threads move between nodes while its guest memory remains elsewhere, latency and interconnect traffic rise.

A topology-aware allocator should place each small Machine's vCPUs, memory, and device work on one NUMA node when practical.
It should treat sibling simultaneous-multithreading threads as shared core capacity rather than pretending every hardware thread has the power of an independent physical core.

The edge case appears when each node has enough total free resources but neither node has the requested resources together.

```text
node 0: 2 free CPU units, 20 GiB free RAM
node 1: 12 free CPU units, 1 GiB free RAM
request: 8 vCPUs, 8 GiB RAM

Host totals say yes: 14 CPU units and 21 GiB RAM are free
single-node placement says no: neither node satisfies both dimensions
```

This is resource fragmentation.
Adding the free numbers across a Host can produce an answer that no valid placement can realize.

### Stage 8: three hundred sandboxes on a 160-thread Host

Assume a 160-thread, 512 GiB Host reserves 16 threads and 48 GiB.
That leaves 144 thread units and 464 GiB for admitted Instances.

```text
memory required      300 * 576 MiB = 168.75 GiB
memory remaining     464 GiB - 168.75 GiB = 295.25 GiB
CPU overcommit       300 / 144 = 2.08:1
```

The simple CPU and memory arithmetic fits.
The larger Host now requires topology-aware placement across its NUMA nodes, and 300 simultaneous network attachments or snapshot restores must be measured as separate gates.

### Stage 9: five hundred sandboxes on the same Host

```text
memory required      500 * 576 MiB = 281.25 GiB
memory remaining     464 GiB - 281.25 GiB = 182.75 GiB
CPU overcommit       500 / 144 = 3.47:1
```

Five hundred mostly waiting agents may fit the arithmetic.
Five hundred CPU-heavy build agents will create a persistent run queue and will not behave like five hundred dedicated CPUs.

At this rung, aggregate limits deserve equal attention:

- Memory bandwidth can saturate even when RAM capacity remains free.
- Shared last-level CPU caches can become contested.
- NIC receive and transmit queues can become hot.
- Connection tracking and ephemeral ports can constrain network-heavy workloads.
- One Generation restore can create a synchronized page-fault wave.
- Five hundred VMM processes and vCPU threads increase scheduler and kernel-object work.

### Stage 10: eight hundred sandboxes approach the cliff

```text
memory required      800 * 576 MiB = 450 GiB
memory remaining     464 GiB - 450 GiB = 14 GiB
CPU overcommit       800 / 144 = 5.56:1
```

The arithmetic still says 800 is below the illustrative 824-Instance memory bound.
The operational answer should probably be no for a guaranteed class because only 14 GiB remains for estimation error, growth, bursts, fragmentation, and emergency cleanup.

This is the difference between a mathematical maximum and a safe operating maximum.

```text
mathematical maximum
  first point at which a simplified division no longer fits

safe operating maximum
  last load-tested point that preserves latency, isolation, cleanup,
  failure headroom, topology, and every resource reserve
```

Adding Instance 801 may succeed technically while making the entire Host less reliable.
SOMA should reject before reaching that cliff according to the certified Host profile.

### Stage 11: one thousand no longer fits this Host shape

```text
memory required      1,000 * 576 MiB = 562.5 GiB
admissible memory    464 GiB
memory deficit       98.5 GiB
```

Snapshot sharing may lower the initial resident set, but it cannot satisfy a guaranteed 512 MiB promise for 1,000 Instances on this Host.
The honest next step is to add Hosts, reduce the Machine shape, or define an explicitly elastic class.

### Incremental Host ladder

The table uses the fixed 1-vCPU, 512 MiB guest, 64 MiB placeholder-overhead shape.
Every row remains subject to storage, network, process, burst, and cleanup gates.

| Host class | Total threads | Total RAM | Illustrative reserve | Strict CPU bound | Memory bound | Example overcommitted admission |
|---|---:|---:|---:|---:|---:|---:|
| Tiny | 4 | 8 GiB | 1 thread, 2 GiB | 3 | 10 | 3 strict or 6 at 2:1 |
| Small | 8 | 16 GiB | 2 threads, 3 GiB | 6 | 23 | 16 at 2.67:1 |
| Medium | 16 | 32 GiB | 2 threads, 4 GiB | 14 | 49 | 42 at 3:1 |
| Large | 40 | 128 GiB | 4 threads, 12 GiB | 36 | 206 | 144 at 4:1 |
| Dense | 80 | 256 GiB | 8 threads, 24 GiB | 72 | 412 | 200 at 2.78:1 |
| Very large | 160 | 512 GiB | 16 threads, 48 GiB | 144 | 824 | 576 at 4:1 |

The example admission column is not a recommendation.
It shows how CPU overcommit changes the arithmetic while the memory bound remains separate.
Only workload-specific load testing can certify an overcommit ratio.

### One large Host as the count rises

This view keeps the illustrative 160-thread, 512 GiB Host fixed.

| Resident count | CPU ratio against 144 units | Memory used | Arithmetic result | New concern |
|---:|---:|---:|---|---|
| 100 | 0.69:1 | 56.25 GiB | comfortable in CPU and RAM | baseline device and network cost |
| 200 | 1.39:1 | 112.5 GiB | light CPU overcommit | synchronized Launch behavior |
| 300 | 2.08:1 | 168.75 GiB | plausible for bursty agents | NUMA placement and NIC queues |
| 400 | 2.78:1 | 225 GiB | workload-sensitive | cache and scheduler contention |
| 500 | 3.47:1 | 281.25 GiB | plausible only with evidence | memory bandwidth and aggregate I/O |
| 600 | 4.17:1 | 337.5 GiB | high overcommit | wake-up and cleanup storms |
| 700 | 4.86:1 | 393.75 GiB | narrow safety margin | dirty-memory and reserve pressure |
| 800 | 5.56:1 | 450 GiB | below arithmetic memory ceiling | only 14 GiB memory headroom remains |
| 824 | 5.72:1 | 463.5 GiB | mathematical memory edge | no credible operational headroom |
| 1,000 | 6.94:1 | 562.5 GiB | impossible for guaranteed shape | exceeds admitted RAM by 98.5 GiB |

The rows do not certify any count.
They teach why each increase changes more than one operational risk even when the Machine shape stays constant.

### Fleet ladder after one Host stops being enough

Assume load testing eventually certifies 200 active Instances per Host for one exact workload class.
Also assume the fleet reserves 20 percent of Host capacity for failures, draining, upgrades, and bursts.

```text
usable active capacity per Host
200 * 0.80 = 160 Instances
```

| Active target | Bare minimum at 200 per Host | Hosts with 20 percent spare policy | What becomes newly important |
|---:|---:|---:|---|
| 1,000 | 5 | 7 | placement retries and one-Host failure |
| 2,500 | 13 | 16 | rack and power-domain awareness |
| 5,000 | 25 | 32 | cell-local admission and Generation distribution |
| 10,000 | 50 | 63 | control-plane partitioning and bounded fan-out |
| 25,000 | 125 | 157 | failure-domain balancing and rolling upgrades |
| 50,000 | 250 | 313 | multiple cells and aggregate artifact delivery |
| 100,000 | 500 | 625 | regional cells, quotas, reconciliation, and disaster capacity |

The spare-policy column uses `ceiling(active target / 160)`.
It does not mean every Host should normally run at its certified maximum.

At fleet scale, immutable sharing still happens primarily within each Host's local cache.
A Generation must also be distributed across Hosts, and a cold fleet-wide request can overwhelm the registry or artifact service even when every Host has free CPU and RAM.

```text
one cached Host launch
  reads local immutable artifacts

one thousand cold Hosts launching together
  may all request the same Generation from shared distribution services
  creates a control-plane and artifact-delivery fan-out problem
```

Cells bound that fan-out and contain failures.

```text
GLOBAL CONTROL PLANE
|
+-- Cell A
|   +-- bounded Host set
|   +-- local admission
|   +-- local Generation distribution
|   `-- local failure containment
|
+-- Cell B
|   `-- same independent responsibilities
|
`-- Cell C and later cells
```

The global layer chooses a healthy cell.
The cell chooses a compatible Host.
The Host atomically reserves resources and launches one Instance.
No global scheduler should synchronously manipulate 100,000 individual KVM file descriptors.

### What happens when one more sandbox arrives

Admission must be atomic across every dimension.

```text
Launch request
      |
      v
check compatible Generation -------- no --> reject
      |
     yes
      v
check CPU policy -------------------- no --> reject: CPU capacity
      |
     yes
      v
check RAM and NUMA placement -------- no --> reject: memory or fragmentation
      |
     yes
      v
check private storage reserve ------- no --> reject: storage capacity
      |
     yes
      v
check network and kernel objects ---- no --> reject: network or Host objects
      |
     yes
      v
reserve everything together
      |
      v
Launch
```

SOMA must not reserve CPU, fail to reserve networking, and leave the CPU reservation leaked.
A rejected Launch returns capacity evidence and rolls back every partial reservation.

### Why a Host breaks above its safe point

Different limits fail differently.

| Limit crossed | First visible symptom | What happens next | Required response |
|---|---|---|---|
| Runnable vCPU capacity | run queues and scheduling delay rise | command p99 grows before failures appear | stop admission or move work |
| Guaranteed RAM | reservation cannot be satisfied | Launch must fail before VM creation | reject or choose another Host |
| Elastic RAM | reclaim and dirty pressure rise | stalls, OOM kills, or Host instability | throttle, evict, or fail closed |
| Snapshot faults | Launch latency rises together | storage queue and page-fault workers saturate | cap concurrent restore |
| Writable storage | free blocks or quota reserve falls | writes fail and cleanup may need space | reject before emergency reserve |
| File descriptors | socket, TAP, event, or image opens fail | partially created Machines become likely | retain FD reserve and reject early |
| Process or thread limit | VMM or vCPU thread creation fails | Launch cannot establish ownership | reject and roll back |
| Network addresses | no private identity is available | guest cannot receive valid network state | reject before resume |
| Connection tracking | new flows drop or time out | network-heavy agents fail unevenly | shard, raise proven limit, or throttle |
| Network bandwidth | latency and packet loss rise | unrelated tenants become noisy neighbors | shape and admit by bandwidth class |
| Cleanup capacity | dead Machines accumulate temporarily | resources remain unavailable longer | reserve cleanup workers and backpressure |
| NUMA locality | remote-memory access increases | tail latency rises despite free totals | topology-aware placement |

The best admission signal appears before user-visible failure.
For CPU, that may be runnable pressure and throttling rather than 100 percent utilization alone.
For memory, it is committed capacity, private dirty growth, reclaim pressure, and NUMA fit.
For storage, it is reserved headroom, write amplification, IOPS, and queue depth.
For networking, it is address inventory, flow state, packets, bandwidth, and policy-programming latency.

### Three workload patterns produce three different answers

```text
PATTERN A: API-WAITING AGENTS
CPU active 10 percent of the time
small dirty-memory working set
many network waits
high CPU overcommit can work
network limits may appear first

PATTERN B: BUILD AGENTS
CPU active most of the time
large dirty-memory working set
heavy filesystem writes
low CPU overcommit is safer
CPU, RAM, and storage contend together

PATTERN C: IDLE INTERACTIVE SESSIONS
almost no CPU while idle
moderate resident memory
occasional synchronized wake-up
high resident density can work
burst admission must cover wake-up storms
```

This is why SOMA needs workload classes backed by evidence rather than one global overcommit number.
An admission profile that is safe for API-waiting agents may collapse under build agents on the same hardware.

### The proof required before moving up one rung

Do not jump from 16 to 200 because the arithmetic fits.
Increase load in steps and retain the evidence.

```text
16 -> 24 -> 32 -> 48 -> 64 -> 96 -> 128 -> 160 -> 200

At every step measure:
  Launch p50, p95, and p99
  first-command p50, p95, and p99
  runnable vCPU pressure and throttling
  resident, shared, and private dirty memory
  major and minor page faults
  storage queue depth, latency, and free reserve
  packets, bandwidth, drops, and connection tracking
  VMM processes, threads, and file descriptors
  cleanup time and leaked-resource count
  success rate and exact rejection reason
```

Stop increasing when the next rung violates the latency, isolation, cleanup, or safety objective.
The last passing rung is evidence for that exact Host profile, Generation, Machine shape, workload class, concurrency pattern, and SOMA version.
It is not proof for a different workload or Host.

The kernel mechanisms underneath this model are documented by the [KVM API](https://docs.kernel.org/virt/kvm/api.html), [cgroup v2 CPU and memory controls](https://docs.kernel.org/admin-guide/cgroup-v2.html), [Linux scheduler design](https://docs.kernel.org/scheduler/sched-design-CFS.html), and [OverlayFS upper and lower layers](https://docs.kernel.org/filesystems/overlayfs.html).

## 16. Can one Host create 100,000 sandboxes?

The answer changes depending on what `100,000` counts.

```text
100,000 CREATED OVER TIME
create -> execute -> destroy -> reuse capacity -> repeat
Possible on one Host if there is no deadline and cleanup remains complete.

100,000 QUEUED REQUESTS
requests wait outside the Host admission boundary
Possible for a control plane, but they are not running sandboxes.

100,000 RESIDENT INSTANCES
all Machines retain memory, process, kernel, storage, and network state
Not realistic on an 80-thread Host with 25 GiB or 256 GiB RAM.

100,000 ACTIVE INSTANCES
all workloads demand CPU, memory bandwidth, storage, or network together
Requires a fleet, not one Host of this size.
```

### Why 25 GiB cannot hold 100,000 microVMs

Before reserving anything for the Host:

```text
25 GiB / 100,000 = about 262 KiB of physical RAM per Instance
```

That budget would need to contain guest memory, page tables, VMM state, virtual-device state, guest-kernel state, private dirty pages, and host accounting.
It is not sufficient for a useful Linux microVM.

Assume an illustrative 25 GiB Host reserves 5 GiB for the host and has 20 GiB left for sandboxes.
The following table demonstrates the arithmetic but is not a measured SOMA capacity claim.

| Machine shape | Illustrative measured overhead placeholder | Memory bound | Strict 1:1 CPU bound | Approximate safe upper bound before other gates |
|---|---:|---:|---:|---:|
| 1 vCPU, 1 GiB guest RAM | 64 MiB | 18 | 72 | 18 |
| 1 vCPU, 512 MiB guest RAM | 64 MiB | 35 | 72 | 35 |
| 1 vCPU, 256 MiB guest RAM | 64 MiB | 64 | 72 | 64 |
| 1 vCPU, 128 MiB guest RAM | 64 MiB | 106 | 72 | 72 |

The 64 MiB overhead value is deliberately labeled as a placeholder.
The real value must be measured on the certified SOMA Host with the exact kernel, device model, Generation, workload, and VMM build.

At 1 GiB per guest, memory stops admission near 18 Instances in this illustrative 25 GiB scenario.
At 128 MiB per guest, strict 1:1 CPU allocation stops admission at 72 even though the memory arithmetic reaches 106.
An operator may configure CPU overcommit for idle or I/O-bound workloads, but a simultaneous burst then queues runnable vCPUs and loses predictable latency.

### The same calculation for 256 GiB

Assume the earlier illustrative reserve leaves 232 GiB for sandboxes and 72 strict CPU units.

| Machine shape | Illustrative measured overhead placeholder | Memory bound | Strict 1:1 CPU bound | Approximate safe upper bound before other gates |
|---|---:|---:|---:|---:|
| 1 vCPU, 1 GiB guest RAM | 64 MiB | 218 | 72 | 72 |
| 1 vCPU, 512 MiB guest RAM | 64 MiB | 412 | 72 | 72 |
| 1 vCPU, 256 MiB guest RAM | 64 MiB | 742 | 72 | 72 |
| 1 vCPU, 128 MiB guest RAM | 64 MiB | 1,237 | 72 | 72 |

These are strict continuously-busy CPU bounds, not limits on mostly idle resident Machines.
With a validated 4:1 CPU overcommit policy, CPU admission could become 288 single-vCPU Instances, but only if memory and every other gate also admit 288.
That policy does not manufacture additional CPU capacity.
If all 288 become runnable together, they compete for 72 thread units.

### How 200 sandboxes can fit on 80 hardware threads

Isolation and simultaneous execution are different properties.
KVM can maintain 200 isolated Machines even though the processor can run instructions for only about 80 vCPU threads at one instant.

```text
200 SANDBOXES
200 virtual CPUs
       |
       | host scheduling and time slicing
       v
80 hardware threads
```

Using the illustrative reserve of 72 schedulable thread units:

```text
200 admitted vCPUs / 72 thread units = about 2.78 vCPUs per thread unit
```

The host scheduler rapidly switches between runnable vCPU threads.
When one guest waits for a network response, disk operation, timer, or host command, another guest can use that hardware thread.

```text
ONE HARDWARE THREAD OVER TIME

time  -------------------------------------------------------------->

      Instance A    Instance B    host work    Instance C    Instance A
      vCPU 0        vCPU 0                     vCPU 0        vCPU 0
      [ running ]   [ running ]                [ running ]   [ running ]

other Instances are runnable, sleeping, waiting for I/O, or paused
```

This works best for bursty agent workloads.
Agents commonly wait for model responses, remote APIs, filesystem operations, subprocesses, or user input.
It works poorly for 200 continuous CPU-bound compilers, encoders, or numerical jobs because the run queue remains full.

SOMA can preserve isolation while oversubscribing CPU because each guest retains private vCPU state and KVM memory boundaries while only execution time is multiplexed.

### Density mechanisms and their tradeoffs

| Mechanism | What is shared or delayed | Why it increases density | Required safety rule |
|---|---|---|---|
| CPU time slicing | Hardware execution time | More vCPUs exist than hardware threads | Enforce quotas, weights, and starvation bounds |
| CPU overcommit | Admission against expected utilization | Idle guests do not waste dedicated cores | Publish the ratio and reject overload beyond the profile |
| Immutable root sharing | Read-only OCI-derived filesystem blocks | Node, Ubuntu, and libraries are stored once | Guests must never mutate shared backing |
| Snapshot memory mapping | Immutable prepared memory pages | Launch avoids copying all guest RAM | Map privately and fail closed on incompatible state |
| Copy-on-write memory | A page is copied only after one guest writes | Identical clean pages consume one physical backing | Every modification becomes Instance-private |
| Lazy page faults | Snapshot pages are loaded when touched | Unused guest address space need not be resident immediately | Bound fault storms and retain enough backing capacity |
| Sparse writable disks | Physical blocks are allocated when written | A 10 GiB logical disk need not consume 10 GiB immediately | Enforce logical and physical quotas plus emergency reserve |
| Shared page cache | Host caches immutable files and snapshot pages | Repeated launches avoid storage reads | Cache content must remain immutable and identity-verified |
| I/O multiplexing | Device and network service time | Waiting guests allow other guests to progress | Apply per-Instance fairness and throughput limits |
| Prepared workers | Invariant VMM setup happens before Launch | Reduces latency without reusing tenant state | Assignment must be single-use with fresh authority |

These mechanisms share immutable bytes, hardware time, caches, or unused capacity.
They do not share mutable guest memory, credentials, writable filesystems, Instance identity, or authenticated control authority.

### What should not be used casually

```text
unsafe or misleading shortcut             reason
--------------------------------------------------------------------------
shared writable root across tenants        one guest can affect another
reused guest identity or session secret    breaks Instance isolation
unbounded CPU overcommit                    destroys latency under a burst
unbounded sparse-disk promises              host can run out after admission
host swap as normal capacity                severe and unpredictable latency
cross-tenant anonymous-page merging         creates side-channel concerns
counting virtual RAM as guaranteed RAM      ignores the dirty-page worst case
```

SOMA should prefer explicit immutable sharing over transparent cross-tenant page-merging mechanisms.
For example, mapping one verified snapshot file privately is understandable and auditable.
Scanning unrelated guest memory and merging coincidentally equal anonymous pages creates a more complicated security boundary and should not be a default density mechanism.

### A realistic 200-sandbox shape on the 256 GiB example Host

The following is an explanatory configuration, not measured evidence:

```text
Host admissible pool
  72 CPU thread units
  232 GiB RAM

200 admitted Machines
  1 vCPU each
  1 GiB guest RAM each
  illustrative 64 MiB non-guest overhead each

CPU
  200 / 72 = 2.78:1 overcommit

Memory
  200 * (1 GiB + 64 MiB) = 212.5 GiB
  232 GiB - 212.5 GiB = 19.5 GiB remaining admission headroom
```

This shape fits the illustrative CPU and memory policies.
It still requires measured proof that storage, network, VMM processes, file descriptors, page-fault rate, Launch concurrency, and cleanup concurrency remain within their certified limits.

On a 25 GiB Host, 200 Machines cannot each receive guaranteed 1 GiB memory.
After a 5 GiB host reserve, 200 Machines divide 20 GiB into only about 102 MiB each before VMM overhead.
That smaller Host would need a much smaller guest profile, elastic memory with explicitly weaker guarantees, fewer resident Machines, or more Hosts.

### Why shared snapshots do not remove the RAM limit

Private copy-on-write memory can let many Instances initially share immutable snapshot pages.
It reduces physical memory used at Launch, but any Instance may dirty pages later.

SOMA therefore needs two explicit policies:

```text
GUARANTEED MEMORY ADMISSION
reserve enough capacity for the promised worst case
lower density, predictable behavior, no memory surprise

ELASTIC MEMORY ADMISSION
admit against measured resident and dirty-page behavior
higher density, but requires pressure limits, eviction policy, and weaker guarantees
```

SOMA must never advertise guaranteed 1 GiB Machines while admitting them as if every Machine will permanently use only its initial shared pages.

### What actually limits the Host first

```text
workload type                    likely first constraint
----------------------------------------------------------------
CPU-heavy agents                hardware threads and run queues
large language runtimes         resident RAM and private dirty pages
build workloads                 storage IOPS, writes, and memory
network crawlers                bandwidth, packets, ports, and conntrack
very tiny idle guests           VMM processes, threads, FDs, and kernel objects
large simultaneous Launch burst page faults, storage reads, and repair channels
large simultaneous cleanup      filesystem, networking, and process teardown
```

The production allocator must report which capacity gate rejected Launch.
That evidence lets an operator decide whether to add RAM, add hosts, reduce Machine shapes, adjust a proven overcommit policy, or move the workload to another pool.

### How SOMA reaches 100,000 active sandboxes

One control plane divides the target across many independently bounded Hosts.

```text
                         100,000 active Instances
                                   |
                              control plane
                                   |
             +---------------------+---------------------+
             |                     |                     |
           cell A                cell B                cell C ...
             |                     |                     |
        many Hosts            many Hosts            many Hosts
             |                     |                     |
       bounded Instances      bounded Instances      bounded Instances
```

If a certified Host safely supports 200 active Instances for a particular workload and availability policy, 100,000 Instances require at least 500 such Hosts before spare capacity, failure domains, upgrades, and regional redundancy are added.
The number `200` in that example must come from retained load-test evidence rather than from hardware specifications alone.

## 17. The sentence to keep in your head

SOMA prepares a sealed Generation, realizes it as a fresh isolated Instance through a Backend, and gives an agent a bounded authenticated way to execute inside it.
