# SOMA prepared worker protocol v1

## Decision

`soma-hostd` maintains bounded pools of sterile, single-use worker processes and resource bundles keyed by exact HostProfile, GenerationId, CPU and memory class, overlay class, and network profile.
Pooling is capacity policy outside `soma-vmm` and never changes Launch semantics or benchmark labeling.

## State machine

A worker moves through `Constructing`, `Sterile`, `Claiming`, `Assigned`, `Running`, and terminal `Destroying` or `Dead`.
Only `Sterile` may be claimed.
One compare-and-swap over WorkerId and monotonically increasing lease generation produces exactly one winner for OperationId.
Retries with the same operation return the same result, while changed intent conflicts.

Sterile workers may hold the executable, jail, empty VM object where measured safe, private memory mapping, verified immutable artifacts, event loop, and unassigned descriptor slots.
They contain no InstanceId, launch page, secret, private disk head, TAP lease, assigned guest CID, command, environment, credential, or tenant byte.
An inactive restored device may retain the non-authoritative captured CID required to decode its snapshot state only when the sterile type cannot start the vCPU, publish launch material, or expose the device bus.
ADR 0033 defines that narrow exception and requires consuming assignment before a readiness challenge exists.
Prepared resource bundles obey the same rule.

Assignment transfers fresh disk, network, control, entropy, identity, deadline, and launch-page authority exactly once.
The allocator leaves the per-Instance data path after transfer.
Any ambiguous transfer destroys the worker rather than returning it to the pool.

## Capacity and recovery

Each pool has minimum, target, maximum, replenishment concurrency, claim deadline, construction deadline, and explicit exhausted behavior.
There is no unbounded waiting queue.
Host restart treats every nonterminal ledger entry as suspect and reconciles process pidfds, cgroups, namespaces, disks, networks, and reservations before replenishing.

Modules are `pool/key`, `pool/state`, `pool/claim`, `pool/transfer`, `pool/replenish`, `pool/backpressure`, `pool/ledger`, and `pool/reconcile`.
Tests prove single winner, idempotent replay, changed-intent conflict, no tenant reuse, crash at every transfer step, pool exhaustion, replenishment storms, Generation eviction, and 100-way fairness and latency.
