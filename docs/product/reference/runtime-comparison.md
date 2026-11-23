# SOMA compared with Firecracker and other runtimes

SOMA packages a Rust KVM monitor with an OCI-to-Generation workflow, guest commands and files, terminals, and an HTTP lifecycle API.
A desktop workload adds a graphical session, computer-use controls, and a browser viewer on the same SOMA instance model.
These pieces give an operator a starting point for code sandboxes, desktops, and services.

## Executable size

These are the uncompressed Linux x86_64 VMM executables from the named builds.
MB is decimal, where 1 MB is 1,000,000 bytes.

| VMM build | Size on disk |
| --- | ---: |
| SOMA alpha.4 draft `soma-vmm` | **1.93 MB** |
| Firecracker v1.18.0 | 3.92 MB |
| Cloud Hypervisor v55.0 static build | 7.71 MB |

The measured SOMA VMM is roughly half the size of this Firecracker executable and one quarter the size of this Cloud Hypervisor executable.
Build flags, static linking, and supported features affect executable size.
Host services, guest memory, kernels, and workload images have separate footprints. [Exact bytes, release sources, and checksums](../../engineering/evidence/2026-11-22-runtime-footprint.md#peer-vmm-executable-sizes)

## What each project provides

| Project | What it provides | Start here |
| --- | --- | --- |
| **SOMA** | KVM microVMs, OCI-derived Generations, authenticated guest command and file operations, terminals, and a desktop workload with computer-use controls | [HTTP API](api-surface.md), [desktop workload](../../../workloads/desktop/README.md) |
| **Firecracker** | KVM microVM monitor with machine configuration, a jailer, virtual devices, and snapshot save and restore | [Getting started](https://github.com/firecracker-microvm/firecracker/blob/v1.18.0/docs/getting-started.md), [snapshot support](https://github.com/firecracker-microvm/firecracker/blob/v1.18.0/docs/snapshotting/snapshot-support.md) |
| **Cloud Hypervisor** | Cloud VMM with CPU and device hotplug, memory resizing, and a broader virtual hardware surface | [Project documentation](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v55.0/README.md) |
| **gVisor** | An application kernel and OCI runtime for container isolation, with Systrap and KVM platforms | [Architecture and platforms](https://gvisor.dev/docs/architecture_guide/platforms/) |
| **Kata Containers** | A Kubernetes and containerd runtime that packages a guest agent and can use VMMs such as Firecracker, Cloud Hypervisor, and QEMU | [Kata architecture](https://katacontainers.io/software/) |

Firecracker and SOMA both provide microVM lifecycle and snapshot mechanisms.
SOMA's workload-facing interfaces include authenticated execution and file operations in the guest.
Compare those with the workload stack you would assemble around Firecracker.
A platform using either engine still owns features such as tenant placement, release routing, and managed database recovery.

## Performance and memory

Firecracker publishes under 5 MiB of overhead per VM and a boot figure under 125 ms. [Firecracker project figures](https://firecracker-microvm.github.io/)
An earlier SOMA KVM cold-boot fixture measured about 3.6 MiB of non-guest resident memory. [SOMA cold-boot sample](../../engineering/evidence/2026-08-29-x86_64-pvh-kernel-boot.md#host-side-resident-overhead-for-one-1-vcpu-guest-with-256-mib-ram)
The measurement methods differ, so these figures leave the relative runtime memory overhead open.

Two prepared-worker SOMA runs at 100 concurrent requests completed all 200 commands and cleanups.
Median create-through-`node -v` time was 60.01 ms and 63.78 ms, with p99 of 69.85 ms and 71.04 ms.
Those are historical loopback runs distributed over three KVM hosts. [Burst methodology and samples](../../engineering/evidence/2026-09-01-computesdk-http-east-burst.md)

A head-to-head test needs the same hardware, guest workload, instance shape, concurrency, and cache state.
The [benchmark contract](../../engineering/benchmark-contract.md) specifies the timing and evidence to retain.
