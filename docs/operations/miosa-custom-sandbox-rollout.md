# MIOSA custom sandbox rollout and benchmark plan

## Purpose

This plan describes how MIOSA can evaluate SOMA beside the existing Firecracker service and later offer SOMA-backed custom sandboxes.
It authorizes no deployment and changes no ComputeSDK repository.
SOMA must remain an experimental engine until every gate below has retained evidence.

## What exists today

SOMA can build an OCI-derived Candidate, boot it on Ubuntu 24.04 x86_64 KVM, capture snapshot artifacts, restore a machine, repair a fresh authenticated guest session, execute a command, and clean up in the development backend.
Generation certification and candidate-bound snapshot verification exist as library gates.
The new sterile restore seam can construct a stopped machine without a private disk head or readiness secret and assign those resources later.

The production fast path does not exist yet.
The current `soma-local` Launch still restores on demand, reports `PreparationClass::OnDemand`, uses a link-down network, and runs the VMM inside the caller process rather than through a separate jailed one-process-per-VM service.
`soma-hostd`, `soma-jail`, `soma-netd`, storage leases, and sterile restore are not yet connected through one live transaction.

## Deployment boundary

SOMA remains the provider-independent VMM and launch runtime.
The MIOSA adapter belongs in the MIOSA platform repository and translates MIOSA requests into SOMA's versioned launch contract.
The adapter must never import KVM internals or weaken SOMA admission.
The existing Firecracker path remains available during comparison and rollback.

```text
MIOSA API and scheduler
          |
          v
MIOSA SOMA adapter outside this repository
          |
          v
soma-hostd -> admitted Generation + prepared-worker pool
          |
          +-> soma-netd network bundle
          +-> soma-storage private overlay
          +-> soma-jail -> one soma-vmm -> one SOMA Machine
```

## Required test hardware

Use at least two identical bare-metal Ubuntu 24.04 x86_64 hosts so one host can run SOMA and one can retain the current engine as a control.
A third identical host is preferred for failure, restart, and soak testing without contaminating latency runs.
Nested virtualization may be used for diagnosis but cannot certify production latency.

Each benchmark host must expose KVM, invariant host configuration, cgroup v2, `/dev/net/tun`, isolated high-performance NVMe storage, and an XFS filesystem with reflink enabled for private disk heads.
Record the CPU model, microcode, NUMA topology, memory channels, kernel, mitigations, firmware, power governor, IRQ placement, storage firmware, and network topology.
Do not disable security mitigations inside a result compared with production unless the result is labeled as a separate unsafe experiment.

For a 100-way `1 vCPU + 1 GiB` burst, 256 GiB RAM is the minimum useful class and 512 GiB is the preferred class for clean headroom, cache control, and failure experiments.
Snapshot-backed private mappings share clean pages, but admission must still reserve for plausible dirty memory and must never assume every tenant stays clean.
An 80-thread host can run 100 mostly waiting one-vCPU sandboxes for the benchmark, but CPU oversubscription must be recorded and separately tested at 1.0x, 1.25x, 2.0x, and the admitted production limit.

## Gate 0: freeze identities and measurement boundaries

Pin the SOMA commit, release digest, guest kernel, guest agent, OCI image digest, GenerationId, host profile, MIOSA adapter commit, and benchmark commit.
Keep OCI acquisition, Generation construction, capture, certification, pool replenishment, and cache warming outside warm Launch timing.
Retain failures and cleanup results instead of deleting them from the cohort.

## Gate 1: complete one production-shaped Machine

Connect certified Generation admission to the live restore path through retained file handles.
Run one `soma-vmm` process per Machine inside the measured jail and cgroup.
Attach a unique private reflink overlay, fresh CID, MAC, IP, TAP, entropy, time sample, and authenticated launch page after ownership transfer.
Require authenticated repair and a real no-op command before Ready.
Prove cleanup after success, timeout, guest failure, VMM crash, caller death, and hostd restart.

## Gate 2: connect the prepared-worker fast path

Make `soma-hostd` construct bounded `Sterile` workers asynchronously from one certified Generation.
Use its existing single-winner claim and durable ledger to transfer the disk, network, identity, deadline, control, and launch authorities exactly once.
Destroy every worker after an ambiguous or partial transfer.
Only this connected path may report `PreparationClass::Prepared`.

## Gate 3: qualify isolation and density

Run sequential and concurrent cohorts at 1, 10, 25, 50, 100, 200, and the first admitted capacity rejection.
Prove private memory divergence, private disk divergence, unique identity, network policy, cross-Instance control rejection, bounded host resources, and complete cleanup.
Measure host CPU, run queue, RSS, proportional set size, dirty memory, page faults, KVM exits, file descriptors, disk latency, network latency, and allocator occupancy.
Capacity is the lowest safe limit found across CPU, memory, dirty-memory reserve, storage IOPS, network queues, file descriptors, KVM resources, and cleanup throughput.

## Gate 4: benchmark correctly

First run an in-repository microbenchmark that separates pool claim, descriptor transfer, disk assignment, network assignment, vCPU resume, guest repair, authenticated Ready, first command, and cleanup.
Then use an out-of-tree provider adapter to run the exact upstream ComputeSDK Burst TTI benchmark without modifying ComputeSDK.
Run `node:22`, 100 iterations, concurrency 100, and stop timing only after `node -v` succeeds inside each sandbox.
Destroy stays outside TTI but every destroy result remains part of the acceptance report.

Run the benchmark runner on the same host to measure engine cost and on a separate nearby host to measure the user-visible API path.
Publish both results separately.
For each cohort retain every raw sample, p50, p95, p99, maximum, success rate, failure reason, cleanup result, cache state, pool occupancy, and host telemetry.
Do not compare a ready lease with another provider's create-through-command result.

## Gate 5: compare with Firecracker

Send identical pinned workloads and shapes through the existing Firecracker engine and SOMA on identical hosts.
Compare complete create-through-command latency, failure rate, density, steady-state overhead, cleanup, isolation coverage, and operator recovery.
SOMA advances only if it meets the security and correctness gates and provides a material product advantage.
Latency alone cannot waive an isolation or cleanup failure.

## Gate 6: controlled MIOSA rollout

Add SOMA as an explicit experimental engine for internal tenants on dedicated hosts.
Run shadow admission first, then synthetic traffic, then trusted internal workloads, then a small opt-in customer cohort.
Use separate host pools, quotas, dashboards, alerts, and a one-action rollback to Firecracker.
Do not place Firecracker and SOMA results under one engine name because their preparation and isolation evidence differ.

## Promotion criteria

Promote SOMA beyond experimental only after the complete Linux KVM gate, production confinement, fresh networking, certified Generation admission, prepared-worker transaction, restart reconciliation, 100-way benchmark, density campaign, and multi-day soak all pass on the exact release.
The initial target remains complete server-side create p99 below 10 ms and exact ComputeSDK Burst TTI p99 below 90 ms with 100 percent create, command, and cleanup success.
These are targets until raw retained evidence proves them.

## Immediate implementation order

1. Add Linux KVM tests for the sterile restore and consuming assignment transition.
2. Replace the link-down placeholder with the admitted `soma-netd` network bundle and fresh MAC repair.
3. Connect certified Generation handles and private overlay leases to `soma-hostd` transfer steps.
4. Launch the restored Machine through `soma-jail` as one `soma-vmm` process.
5. Add the prepared-worker benchmark harness and raw artifact schema.
6. Build the MIOSA adapter outside this repository only after the host transaction passes locally.
7. Execute the host qualification and comparison campaign before any customer deployment.
