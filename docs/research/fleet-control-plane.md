# SOMA fleet control plane v1

## Decision

Fleet scale is a hierarchy of independently bounded cells rather than one global scheduler or one giant warm pool.
The VMM remains node-local and single-VM.
The control plane owns authentication, quotas, admission, placement, Generation distribution, capacity intent, routing, reconciliation, observability, and regional failure handling.

## Topology

A global directory maps tenant and region to a cell without joining the Launch data path.
Each regional cell contains replicated API and operation state, a bounded scheduler shard, Generation cache coordination, capacity controllers, gateways, and thousands rather than hundreds of thousands of hosts.
Each host runs `soma-hostd`, `soma-netd`, the artifact cache, and one single-use `soma-vmm` per active sandbox.

Placement filters by certified HostProfile, Generation availability, CPU and memory class, overlay and network class, isolation requirements, quota, fault domain, and reserved capacity, then scores cache warmth, pool availability, fragmentation, and load.
The scheduler returns one expiring placement lease.
Host admission is authoritative and may reject stale or oversubscribed placement.

## Operations and consistency

Every mutation uses caller OperationId, canonical request fingerprint, tenant, deadline, and idempotency record.
Operation state is `Accepted`, `Placed`, `HostCommitted`, `Ready`, or a typed terminal result with cleanup state.
Late and duplicate messages are safe, and changed intent conflicts.
There is no distributed transaction across cells and no assumption that timeout means absence.

Generation artifacts are content-addressed, signed, replicated ahead of demand, verified on installation, and immutable on hosts.
Cache miss is a separate preparation class and never hidden in a warm benchmark.
Capacity controllers maintain reserved sterile resources from measured arrival rates and tail latency, with explicit overload rejection and tenant fairness.

## Failure containment and observability

Cells have fixed tested envelopes and independent databases, queues, credentials, routing, quotas, deployments, and kill switches.
Regional failover creates new operations only under explicit client retry policy and never moves live VM memory in version 1.
Host heartbeat loss fences new work, reconciles leases, and reports uncertain cleanup until the host or external infrastructure proves destruction.

Metrics use bounded-cardinality cell, host-profile, Generation class, result, milestone, and failure labels rather than tenant or Instance labels.
Traces follow OperationId across API, scheduler, host, VMM, guest repair, command, and cleanup while redacting secrets and output.

Modules are `api`, `identity`, `quota`, `operations`, `cells`, `placement`, `capacity`, `generations`, `host_leases`, `routing`, `reconcile`, `observability`, and `admin`.
Scale gates progress through one host, one 100-way host burst, a multi-host cell, cell failure tests, multi-cell regional load, and only then a synthetic 100,000-concurrent exercise with measured control-plane and host limits.
