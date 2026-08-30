# Review of the Isorun creation-latency evidence

- Date: 2026-08-30
- Reviewed commit: `d27e7bd`
- Reviewed files: `COMPETITORS.md` and `docs/evidence/2026-08-30-isorun-create-latency.md`
- Outcome: Valuable experiment and useful competitive signal, but not yet reproducible evidence

## What is worth preserving

The experiment is correctly focused on a previously unknown quantity.
It separates Isorun's returned `create_ms` field from client wall-clock latency and from ComputeSDK create-through-command TTI.
It exercises sequential, concurrency-10, and two concurrency-100 Node cohorts instead of publishing only the fastest observation.
Both concurrency-100 cohorts returned a p50 of 73 ms, while the sequential Node cohort returned a p50 of 22 ms.
Every recorded main-cohort command reportedly succeeded with exit code zero, and the document explicitly says the result proves nothing about SOMA.

Those are strong research choices.
The observed burst sensitivity is directly relevant to SOMA's pool sizing, admission, replenishment, and benchmark design.

## Required corrections

### 1. Retain the credential-free harness

The account credential must remain outside the repository.
The harness itself must not.

Add a parameterized harness that reads the credential from an environment variable and records:

- Exact request body and selected service region.
- Monotonic request-start and response-finish timestamps.
- The complete create response with secrets removed.
- Command result and exit status.
- Destruction request and verified destruction result.
- Request-send offset from the cohort barrier.
- Retry count, timeout, and every error.
- One record for every attempted sample, including failures.

The harness must not print or persist the credential.

### 2. Retain redacted raw samples

Commit the JSONL or equivalent raw records from which every table cell is calculated.
Remove account identifiers, request authorization, and any returned secret while preserving timing, cohort, result, and cleanup fields.

The report generator must recompute the table from those records.
A reviewer should be able to reproduce min, p50, p95, p99, max, success, and cleanup counts without access to Isorun.

### 3. Correct the evidence classification

`create_ms` is provider instrumentation independently collected by the experimenter.
It is not an independently timed server interval because the implementation and timer endpoints are controlled by Isorun.

Use this classification:

> Independently collected vendor-reported telemetry

Keep harness wall-clock time as the independently measured client interval.

### 4. Do not equate `create_ms` with SOMA authenticated Ready

The reviewed material does not establish whether `create_ms` includes:

- Admission queueing.
- Worker allocation.
- Memory restore.
- Guest identity and entropy repair.
- Network repair.
- Guest authentication.
- Successful command execution.

Describe it as an unknown vendor-defined creation stage.
Compare it with SOMA only as competitive context, never as an equivalent lifecycle milestone.

### 5. Mark causal explanations as hypotheses

The tiny-image bimodality may be consistent with warm-pool behavior, but it does not prove a hit or miss.
The 4,808 ms Deno request may be consistent with excluded image preparation, but one request does not prove that the image was uncached or identify what work occurred.

Both statements need explicit inference language.
Do not state another provider's internal mechanism as fact without source or instrumentation evidence.

### 6. Define every cohort and billing scope

The 250-row main table and the additional Deno and Node probes need separate cohort identities.
Record their attempted, successful, failed, and cleaned counts independently.

The reported 0.13-cent total and 0.0152-cent calculation also need distinct scopes.
Name which samples each amount covers and retain the redacted billing observation used to verify it.

### 7. Complete experiment metadata

Record:

- Exact UTC start and finish.
- Measuring host operating system and architecture.
- Approximate network location without exposing private host identity.
- Harness commit and command.
- Python version.
- Request timeout and retry policy.
- Concurrency implementation and barrier definition.
- Service endpoint and region.
- Every excluded probe and why it is excluded.

## Correct claim after repair

The strongest supported wording should remain narrow:

> In two 100-request Node 22 cohorts collected from one host, account, service region, and day, Isorun returned a p50 `create_ms` of 73 ms.
> A ten-request sequential cohort returned a p50 of 22 ms.
> `create_ms` is vendor-reported telemetry with undocumented timer endpoints and is not equivalent to SOMA authenticated Ready or ComputeSDK TTI.

That claim remains useful without overstating the evidence.

## Architecture consequence for SOMA

The experiment does not prove why Isorun degraded under burst.
It does prove that SOMA must treat concurrency as an independent benchmark dimension rather than extrapolating from a sequential result.

The SOMA implementation consequence is:

```text
prepare before demand
        |
admit against exact capacity
        |
claim one sterile worker atomically
        |
transfer fresh authority
        |
resume, repair, authenticate, execute
        |
measure every stage and cleanup
        |
replenish outside the request path
```

The request path must not perform OCI acquisition, Generation construction, kernel cold boot, namespace construction, TAP creation, storage-head cloning, VMM process creation, or full snapshot restoration when the declared preparation class is a prepared worker.

## Acceptance checklist

- [ ] Credential-free harness committed.
- [ ] Redacted raw records committed.
- [ ] Report regenerated entirely from retained records.
- [ ] Every attempted sample and cleanup result included.
- [ ] Evidence reclassified as independently collected vendor telemetry.
- [ ] Direct comparison with SOMA Ready removed.
- [ ] Warm-pool and image-preparation explanations labeled as hypotheses.
- [ ] Cohort and billing scopes made explicit.
- [ ] Host, time, timeout, retry, and scheduling metadata completed.
- [ ] Architecture lesson linked to the competitive adoption audit.

## Review judgment

Keep the experiment.
Harden its evidence.
Use its burst observation to improve SOMA's prepared-worker design, but do not use undocumented provider telemetry as proof of an equivalent SOMA stage.
