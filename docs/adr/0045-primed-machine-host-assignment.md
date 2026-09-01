# ADR 0045: Prime one identity-free machine and private disk per hosted slot

## Status

Accepted.

## Context

ADR 0044 moved process creation before request traffic but deliberately left VM restore on the request path.
Exact ComputeSDK-shaped qualification showed that process preparation alone produced variable 62 to 71 ms medians under a synchronized one-hundred-sandbox burst.
The request still paid for snapshot restore, KVM and device construction, and private overlay creation before it could assign fresh Instance authority.

A stopped restored VM and a reflinked private disk contain no tenant identity, readiness secret, guest CID, network lease, secret value, or customer write.
Those resources can therefore be prepared before a request without giving a future Instance authority early.

## Decision

Each configured API worker starts one dedicated machine-host child before the listener accepts traffic.
The parent transfers independently opened, previously verified Generation descriptors to the child.
The child reconstructs the admitted Generation, restores one stopped identity-free VM, creates one unlinked private overlay head, and acknowledges preparation over its private channel.

The parent publishes a child into the available pool only after that acknowledgement.
A matching Launch atomically removes one child from the pool and sends the complete Instance assignment.
The assignment supplies the public Instance identity, operation identity, guest CID, launch page, readiness challenge, network authority, secrets, and the already-private disk descriptor.
The child then resumes the vCPU, authenticates guest repair, binds the Instance socket, and reports Ready.

The prepared key includes the Generation, memory size, vCPU count, overlay capacity, and declared device set.
A request that does not match the key cannot consume the prepared machine.
Pool depletion or a nonmatching request uses the ordinary on-demand path and reports `on_demand` rather than pretending preparation occurred.
The refill worker begins after the timed first command should have completed and restores the configured pool depth asynchronously.

The public API may reuse one HTTP/1.1 connection for a bounded maximum of sixteen requests.
This lets the create, first command, and excluded cleanup use one correctly framed connection while preventing one peer from monopolizing a worker indefinitely.

## Security invariants

- No Instance identity or tenant data exists in an available worker.
- Every worker owns exactly one KVM VM and can serve at most one Instance before exiting.
- Verified artifact paths are never reopened by the child.
- Every private disk is a unique reflinked and immediately unlinked file held only by descriptor.
- A failed preparation kills and waits for its child before the API can continue startup.
- A failed assignment consumes the worker and cannot return partially assigned authority to the pool.
- Successful children are transferred to the process reaper, and cleanup must leave no zombie or Instance socket.

## Consequences

API readiness now means the configured fast-path capacity actually exists, not merely that empty processes were spawned.
Startup and refill perform more work and retain one stopped VM plus one private disk descriptor per available slot.
Operators must size the pool against memory commitment, KVM limits, file descriptors, storage metadata, and admitted burst capacity.
Shared production-host traffic can still inflate latency after resume, so qualification must record host load and must not present a contended cohort as engine-only performance.

The final east-host qualification binary had SHA-256 `fbeb7229640c56876799196752daf2ed787e2ca545b38c4fa9aa5105324bff90`.
Its first two identical final cohorts completed 200 of 200 commands and cleanups.
They measured 60.01 and 63.78 ms median, 69.27 and 70.75 ms p95, and 69.85 and 71.04 ms p99 through the exact create-through-`node -v` timing boundary.
A third cohort retained 100 percent success but was invalid as clean performance evidence because unrelated encrypted-disk and control-plane load on host10 raised all forty of its prepared samples to approximately 1.16 seconds.
