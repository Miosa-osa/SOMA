# Provider and VMM lessons

## Method

This document records source-backed observations that affect SOMA's design.
It distinguishes public implementation evidence from inference.
A published internal phase time is not treated as end-to-end sandbox readiness unless the measurement includes a successful first command.

## Firecracker

### Fact

Firecracker uses one process per microVM with separate control, VMM, and vCPU thread responsibilities.
Its snapshot restore path supports private file-backed guest-memory mappings.
Its production guidance depends on a hardened host and jailer rather than the VMM alone.

### SOMA decision

Keep one process per machine, a minimal device model, private snapshot-backed memory, thread-specific containment, and a host hardening contract.
Do not equate a minimal VMM with a complete multi-tenant sandbox boundary.

## Tencent CubeSandbox

### Fact

Tencent Cloud names the complete product CubeSandbox and the VMM layer CubeHypervisor.
CubeShim integrates with containerd, Cubelet owns node-local lifecycle, CubeVS handles kernel-level networking, and CubeEgress provides higher-level egress policy.
The layers are named separately instead of calling the whole product a VMM.

### SOMA decision

Keep the VMM, launcher, guest agent, artifact builder, network policy, and orchestration roles explicit.
Do not put fleet scheduling, public sandbox semantics, or L7 egress inside `soma-vmm`.

## E2B

### Fact

E2B's public infrastructure architecture uses one Firecracker process per sandbox and surrounds it with orchestration and environment services.
E2B is a sandbox platform rather than a VMM implementation.

### SOMA decision

Publish exact layer ownership and never market orchestration work as a custom VMM.

## Mitos

### Fact

Mitos demonstrates prepared Firecracker processes, staged snapshots, reflinked disks, preclaimed capacity, guest repair, and an optional Firecracker fork for live copy-on-write memory.
Its fastest published measurements use preparation outside the request path.
Its own documents separate internal activation time from create-through-first-exec time.

### SOMA decision

Implement the simple prepared-artifact and private-mapping path before adopting userfaultfd write-protection complexity.
Measure stock restore, prepared capacity, paused-pool lease, and public end-to-end TTI as separate experiment classes.

## SporeVM

### Fact

SporeVM demonstrates a custom Zig VMM, immutable snapshot fan-out, private memory backing, OCI conversion, and explicit guest readiness.
Its public support matrix says complete saved-state lifecycle support is mature on ARM64 while Linux AMD64 saved state remains incomplete.

### SOMA decision

Treat architecture support as a tested compatibility dimension.
Do not generalize a benchmark or feature from one hypervisor backend or CPU architecture to another.

## Machinen

### Fact

Machinen publishes separate warm-boot, snapshot, restore, fork, and exec-readiness measurements.
The values differ materially depending on the endpoint.

### SOMA decision

Every performance result names its start event, end event, cache state, concurrency, resource shape, architecture, and excluded work.
Console output is not readiness.

## Vibemon

### Fact

Vibemon implements a custom cross-platform Rust VMM and advertises very small clone phases.
Its public interface can skip agent readiness for some paths, and its repository warns that it is not yet a production isolation boundary.

### SOMA decision

Never publish a clone or restore phase as customer-ready latency.
The default launch interface cannot opt out of Repair or authenticated first-command readiness.

## smolvm

### Fact

smolvm's live-fork design uses shared memory state and modified libkrun.
Its documented security tradeoffs include permissive process tracing and a skipped Landlock restriction for some pathless memory-file flows.

### SOMA decision

Reject a latency optimization if it weakens the hostile multi-tenant boundary.
No launch path may silently downgrade containment to gain speed.

## Dragonball

### Fact

Dragonball is a Rust VMM integrated into Kata Containers for lower runtime and IPC overhead.
A 2026 security advisory describes a guest-to-host escape caused by missing length validation in virtio-blk before host `io_uring` operations.

### SOMA decision

Treat every guest-controlled descriptor, address, length, queue index, and feature bit as hostile.
Use checked arithmetic, bounded guest-memory access, fuzz targets, process isolation, and security regression tests around every device path.
Rust narrows memory-safety risk but does not make unsafe DMA and I/O logic correct automatically.

## crosvm

### Fact

crosvm can isolate emulated devices in separate processes and applies process-specific sandbox policies.
That topology improves fault containment at the cost of more processes and launch coordination.

### SOMA decision

Begin with one VMM process and a very small device surface.
Revisit per-device processes only when a threat analysis or measurement justifies the additional launch and operational complexity.

## Kuasar and Quark

### Fact

Kuasar separates a multi-sandbox runtime from several underlying isolation engines.
Quark pairs a specialized QVisor VMM with a QKernel guest and optimizes their joint interface.

### SOMA decision

Keep the external SOMA interface independent from one orchestrator while allowing the managed guest and VMM to co-design a narrow repair and execution channel.

## Cross-provider rules

- The public interface cannot return success at process start, memory map, VM restore, vCPU resume, console output, or agent connect.
- The compatibility identity includes the VMM build, snapshot schema, CPU class, guest kernel, command line, device topology, memory shape, guest agent, root filesystem, and security policy.
- Full artifact hashing occurs at certification and audit time, not during each launch.
- A hot-pool lease and an on-demand snapshot restore are different products and different benchmark classes.
- Cleanup success and identity uniqueness are part of every burst result.
- A benchmark without raw samples, failures, environment, cache state, and exact timing boundaries is a hypothesis rather than evidence.
