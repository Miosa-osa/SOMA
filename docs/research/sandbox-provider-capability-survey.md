# What sandbox providers offer, and the dimensions SOMA has not chosen

A survey of the providers on the ComputeSDK list, recorded on 2026-08-31 from public documentation.
Every figure here is a vendor claim, not an independently measured fact, and several are absent from public documentation entirely.
It exists to name the dimensions a sandbox product must have an answer for, including the ones SOMA has never decided.

The interface those answers hang from is in [the provider contract gap analysis](provider-contract-gap-analysis.md).

## Resource dimensions

| Provider | vCPU | Memory | Disk | Maximum lifetime | Isolation |
| --- | --- | --- | --- | --- | --- |
| E2B | 2 default, 1 to 8 on paid | 1 GB default, 512 MiB to 8192 MiB | not documented | 1 hour free, 24 hours paid | Firecracker |
| Modal | not documented | not documented | not documented | 5 minutes default, 24 hours maximum | gVisor on KVM |
| Vercel Sandbox | not documented on this page | not documented | Drives in beta | not documented on this page | Firecracker |
| Daytona | not retrieved | not retrieved | not retrieved | auto-stop and auto-archive documented | OCI containers, optional Kata or Sysbox |
| Cloudflare | inherits Containers | inherits Containers | inherits Containers | not documented | containers on Workers |
| Runloop | "a range of options" | "a range of options" | not documented | not documented | microVMs |
| Superserve | not documented | not documented | not documented | not documented | Firecracker |
| **SOMA** | **1, by machine contract** | **the Generation's captured size** | **the Generation's overlay** | **one command, then shutdown** | **KVM, own VMM** |

The last row needs stating carefully, because SOMA does accept a shape and then does not treat it the way the table above implies.

`MachineShape` carries `vcpu_count`, `memory_mib`, and `storage_mib`, and the command line accepts all three. What happens to them differs per field:

- The vCPU count is fixed at one by the `x86_64` machine contract. Both the cold-boot and restore paths call `create_vcpu(0)` and nothing else. The receipt is honest about this: it reports the contract's one as an observed value rather than echoing what was asked for.
- The memory size must **exactly equal** the size the Generation's snapshot was captured with. `compatibility::check_header` compares the requested size against the manifest and returns `Incompatibility::MemoryLayout` when they differ, so a caller asking for more memory than the snapshot holds gets a rejected launch rather than a larger sandbox.
- The overlay size comes from the Generation's template, not from the request.

So shape in SOMA is a **Generation build parameter, not a per-Instance one**, and the honest way to say it is that each Generation has exactly one launchable shape.
That is a stronger constraint than a fixed default, because a default can be raised and this cannot: offering a range of memory sizes means a Generation and a captured snapshot per size.

Two consequences follow that are not about performance.
The prepared worker pool is keyed by exact CPU and memory class, so every additional shape is an additional pool, and capacity becomes per shape rather than per host.
The overlay is the private writable disk, so its size is a per-Instance storage commitment: reflink cloning means the space cost is what is written rather than what is allocated, but admission still has to reserve against what could be written.

## Lifecycle and persistence

| Capability | What providers do | SOMA |
| --- | --- | --- |
| Pause and resume | E2B preserves filesystem and memory, about 4 s per GiB to pause and about 1 s to resume, with a filesystem-only mode. Vercel persists by default and auto-saves on stop | **None.** A sandbox runs one command and is destroyed |
| Snapshot from a running sandbox | E2B, Vercel, and Modal all document it, to skip dependency installation on later runs | **None.** Capture happens once per Generation, before any Instance exists |
| Idle timeout | Modal documents `idle_timeout` with activity defined as commands, stdin, or live TCP | **None** |
| Reconnect to a running sandbox | `sandbox.getById()` across all providers | **None.** No Instance outlives its process |

SOMA's snapshot is a build-time artifact shared by every Instance of a Generation.
Every provider surveyed also has a per-sandbox snapshot taken after the user has changed something, which is a different object with a different lifetime and a different privacy class, because it contains tenant state.
SOMA has no design for that object.

## Capabilities SOMA has no answer for

**Container runtimes inside the sandbox.**
Vercel documents system-privileged processes explicitly, naming Docker, VPN clients, and FUSE drivers as supported workloads.
This is what makes a sandbox usable for real build and test work, and it needs egress, a writable layer the container runtime can use, and enough privilege inside the guest.

**Per-agent isolation inside one sandbox.**
Vercel gives each agent its own Linux user and private home directory, with groups for sharing.
That is the multi-agent pattern, and it is a guest-side concern SOMA's agent does not implement.

**Exposed ports and preview URLs.**
Cloudflare generates preview URLs for HTTP services inside the sandbox; the same idea appears across the list.
SOMA has an `ingress` module inside `soma-netd` and no path from a running Instance to a reachable URL.

**Interactive terminals.**
Cloudflare documents WebSocket terminals with configurable dimensions.
SOMA's `Terminal` frame is a command's exit status, not a pseudo-terminal.

**Attached and remote storage.**
Vercel Drives, and mounting S3 or R2 or GCS through FUSE, give sandboxes state that outlives them.
SOMA's overlay is private, unlinked at launch, and destroyed with the Instance by design.

**Managed base images.**
Vercel's default image bundles Node LTS, Python 3.14, coding agents, and utilities. E2B ships `base`, `python`, `node`, and `ai-agent`.
SOMA can compile any OCI image into a Generation and publishes none, so every user starts from an upstream image and a Template they wrote themselves.

## What this changes about priority

Nothing in this survey is a reason to copy a competitor.
It does change which SOMA decisions are missing rather than merely unimplemented.

The shape question is a decision, not work: SOMA must decide whether a caller may request CPU, memory, and disk, because the answer determines pool keying, admission, capacity, and the Template schema, and every part of the system currently assumes one shape.

The per-sandbox snapshot is also a decision: it introduces an artifact holding tenant state, which the current snapshot deliberately does not, and its privacy class has to be settled before it is built.

The rest is ordered work, and the order is the one in the gap analysis: egress first, because a container runtime, a package install, a repository clone, and a model API call are all the same missing thing; then the guest protocol, because the filesystem and terminal surfaces cannot exist above eight frames that do not include them; then a persistent Host Runtime, because reconnect and pause are ownership rather than API.

## What this document is not

- It is not a measurement of any product, including SOMA.
- Absent figures mean the public documentation did not state them, not that no limit exists.
- It is not a commitment to implement anything listed here.
