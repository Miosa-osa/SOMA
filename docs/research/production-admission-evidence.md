# SOMA production admission evidence v1

## Decision

A host profile is production-admitted only by a signed immutable validation report whose raw artifacts satisfy every mandatory gate.
No skipped test, mock, cross-build, development backend, average-only timing, or vendor claim counts as passing evidence.

## Gates

Correctness covers OCI import, deterministic Generation reproduction, cold boot, snapshot capture and restore, authenticated repair, Ubuntu and Node commands, stop, destroy, and retry semantics.
Isolation covers private memory, immutable root, private overlay, namespaces, cgroups, seccomp, device-parser fuzzing, network policy, metadata denial, authority exclusion, and cross-Instance attempts.
Failure covers corruption, incompatibility, resource exhaustion, timeout, vCPU stall, process and broker crash, hostd restart, partial transfer, and cleanup reconciliation.
Concurrency covers sequential soak plus bursts of 10 and 100 with unique identity, bounded admission, no starvation, complete cleanup, and post-run host baselines.

Performance records raw monotonic timestamps for every lifecycle milestone, cold and warm cache state, preparation class, successes and failures, and p50, p95, p99, minimum, maximum, and cohort size.
Required headline boundaries are complete server-side create and first bounded command, not process start or vCPU resume.
The exact ComputeSDK benchmark is retained separately with its source revision and network placement.

## Provenance and admission

Reports bind SOMA revision, source status, binaries, toolchains, GenerationId, artifact digests, host distribution and kernel, KVM API and capabilities, CPU and microcode, memory, NUMA, storage and mounts, network profile, cgroup mode, mitigations, firmware, ambient load, and benchmark harness revision.
Admission policy checks signed report schema, freshness, exact profile identity, mandatory results, regression thresholds, and revocation state.
Any kernel, microcode, VMM, machine contract, device contract, compiler, filesystem, network, or jail change invalidates affected evidence until recertified.

Modules are `evidence/schema`, `evidence/collector`, `evidence/harness`, `evidence/report`, `evidence/sign`, `admission/policy`, and `admission/revoke`.
The existing validation template is the operator runbook; this document defines when its output is sufficient to make a public claim.
