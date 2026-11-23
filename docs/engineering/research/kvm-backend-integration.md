# SOMA KVM backend integration v1

## Decision

The portable `soma` facade keeps Resolve, Launch, Execute, Inspect, Stop, and Destroy unchanged.
The Linux KVM adapter composes `soma-generation`, `soma-hostd`, `soma-netd`, storage ownership, one `soma-vmm`, authenticated `soma-guest`, and evidence builders behind that contract.
It never exposes KVM sequencing to callers and never falls back to Docker or Apple virtualization.

## End-to-end transaction

Resolve selects an installed certified Generation by immutable OCI and platform identity.
Launch validates request and host capability, reserves durable OperationId ownership, admits capacity, claims one prepared worker and sterile bundle, creates the private overlay and network lease, transfers descriptors, restores the snapshot, injects fresh launch material, resumes, repairs, probes, activates networking, and returns Ready evidence.
Execute sends one bounded direct command through the owned authenticated controller and returns typed output and process outcome.
Inspect reconstructs truth from the original request, live owner, ledger, network, and cleanup state.
Stop requests authenticated shutdown, enforces the deadline, exits and joins vCPUs, destroys the process, and releases owned resources.
Destroy is idempotent and completes or reports each cleanup disposition independently.

Every mutation records ownership before external effect and records completion before publication.
Timeout never means cancellation succeeded.
Retries use caller OperationId and request fingerprint, and a conflicting fingerprint is rejected.

## Modules and implementation slices

The adapter is split into `resolve`, `admit`, `launch_transaction`, `worker_client`, `vmm_client`, `control_owner`, `inspect`, `stop`, `destroy`, `evidence`, and `reconcile`.
Implementation proceeds as vertical slices: Ubuntu cold boot and file read, authenticated command, Generation restore, private disk, isolated network, full lifecycle, prepared worker, then 100-way burst.

Linux end-to-end tests must run real Ubuntu and Node 22 Generations, exercise every public operation and retry, kill every component at every milestone, prove truthful receipts and cleanup, and verify that unsupported capability returns a typed failure without a weaker backend.
