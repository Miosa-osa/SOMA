# SOMA benchmark contract

## Principle

SOMA optimizes end-to-end command readiness rather than one internal restore phase.
Every published result must be reproducible from raw samples.

## Exact public benchmark

The initial external target is the ComputeSDK Burst TTI benchmark.
The authoritative implementation remains the upstream benchmark repository rather than a SOMA reimplementation.

The comparison profile is:

- OCI image `node:22`.
- 100 iterations.
- Concurrency 100 with every slot opened as one burst.
- Timer begins before the provider create call.
- Timer ends after `node -v` succeeds inside the created sandbox.
- Destruction is excluded from TTI but must still succeed and be reported.
- Median, p95, p99, success rate, wall time, and raw samples are retained.

## Internal stage measurements

Production benchmark instrumentation must record monotonic timestamps for every lifecycle milestone.
The Phase 0 Ready value contains ordered milestones without timestamps.
Internal stages explain the public result but never replace it.

At minimum, retain:

- Request acceptance.
- Resource ownership.
- Process creation.
- Artifact verification.
- Memory mapping.
- KVM creation and state restoration.
- vCPU resume.
- Guest-agent authentication.
- Generation acknowledgement.
- Identity and network Repair.
- First command success.
- Cleanup completion.

## Experiment classes

Results from these classes must not be merged:

1. Cold Generation build from an uncached OCI registry.
2. On-demand restore with a cold host page cache.
3. On-demand restore with a warm host page cache.
4. Restore from prepared host resources.
5. Lease of an already-restored paused machine.
6. Lease of a fully running ready machine.

## Required metadata

- SOMA commit and release identity.
- Host kernel and KVM identity.
- CPU model, microcode, NUMA placement, and enabled feature class.
- Physical RAM and storage topology.
- Guest kernel, command line, vCPU count, memory size, disk size, and Generation digest.
- Network path and benchmark-runner location.
- Cache state and preparation performed outside the timer.
- Concurrency, request scheduling, retries, timeouts, and excluded work.
- Every error and cleanup result.

## Initial targets

Targets are not claims.

- Prepared worker acquisition and dispatch p50 below 0.10 ms and p99 below 0.50 ms.
- Private mapping, KVM restoration, and guest-control wake p50 below 1.15 ms and p99 below 3.30 ms.
- Combined authenticated guest repair and Ready p50 below 1.50 ms and p99 below 3.50 ms after guest-control wake.
- The additive server-side create budget totals 3.25 ms p50 and 8.90 ms p99.
- Complete server-side create p50 below 5 ms and p99 below 10 ms.
- First bounded command completion p50 below 10 ms and p99 below 20 ms from accepted Launch.
- Public Burst TTI median below 50 ms and p99 below 90 ms.
- 100 successful launches, commands, and cleanups out of 100.

Release evidence must contain at least 100 complete bursts and 10,000 samples in addition to the exact 100-sample external benchmark cohort.
The larger engineering corpus makes tail analysis less dependent on one observation.

## Anti-gaming rules

- Do not substitute console output, socket acceptance, or agent ping for the required command.
- Do not warm only SOMA while comparing against a cold competitor path.
- Do not omit failed samples.
- Do not hide preclaimed or already-running capacity.
- Do not compare ARM64 and x86_64 results as one execution path.
- Do not claim an internal microbenchmark as provider TTI.
