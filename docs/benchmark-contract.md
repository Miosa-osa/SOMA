# SOMA benchmark contract

## Principle

SOMA optimizes end-to-end command readiness rather than one internal restore phase.
Every published result must be reproducible from raw samples.

## Controlled local-alpha build provenance

Local-alpha measurement must use the two explicit release binaries from a separate controlled build step.
The build step refuses a dirty or non-Git checkout, removes only the prior `target/release/soma` and `target/release/soma-mcp` outputs, and runs exactly `cargo build --locked --release -p soma-cli -p soma-mcp`.
It writes the v2 build manifest with create-exclusive semantics only after Cargo succeeds and recreates both executable outputs.
The manifest destination must be absolute, must have an existing parent directory, and must not already exist.

Create the build evidence before starting measurement:

```sh
python3 -m benchmarks.local_alpha.build_release \
    --build-manifest /absolute/path/soma-local-alpha-build.json
```

Every measured invocation requires that same external manifest through an absolute path:

```sh
python3 -m benchmarks.local_alpha \
    --scenario-id base-cli-one-shot-node-22-1vcpu-1024mib-10240mib-denied \
    --repetitions 100 \
    --build-manifest /absolute/path/soma-local-alpha-build.json \
    --soma-bin /absolute/path/SOMA/target/release/soma \
    --soma-mcp-bin /absolute/path/SOMA/target/release/soma-mcp \
    --apple-runtime /absolute/path/container \
    --result-dir /absolute/new/soma-local-alpha-results \
    --cache-state cached
```

The measured runner loads and validates the manifest and never invokes Cargo or creates build provenance.

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

`python3 -m benchmarks.computesdk_exact` is the retained qualification harness for the exact public HTTP boundary.
It deliberately uses one fresh HTTP connection for create, command, and destroy because the upstream ComputeSDK provider boundary includes those request round trips.
`python3 -m benchmarks.computesdk_exact.combine` recomputes one cohort from synchronized host shards without treating host-local monotonic clocks as globally comparable.
The implementation mirrors ComputeSDK revision `46dea652fcc372e5acea0c9f372613d86b4b6bab` and its five-percent two-sided trim, arithmetic median, and nearest-rank tail percentiles.
The retained east-host qualification is documented in [the exact HTTP Burst TTI evidence](evidence/2026-09-01-computesdk-http-east-burst.md).

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

## Burst harness entry point

`python3 -m benchmarks.local_alpha.burst run` measures one declared cohort of N iterations at concurrency C, opens every slot of each burst through one barrier, starts the timer before the create call, stops it after the workload command succeeded inside the sandbox, and executes and verifies destruction outside that timer.
It requires the same external build manifest as the sequential runner, requires one of the six experiment classes above, requires every class except `cold-generation-build` to declare what was prepared before the timer, and writes every attempted sample, failures included, to one JSONL results file.
`python3 -m benchmarks.local_alpha.burst report` turns one or more results files of the same experiment class into an evidence document, and refuses an incomplete run, a run whose records mix experiment classes, a sample counted successful without a zero-exit workload command, and a warm class with no recorded preparation.

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
