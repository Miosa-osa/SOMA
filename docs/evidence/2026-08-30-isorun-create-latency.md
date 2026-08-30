# Isorun creation telemetry, independently collected - 2026-08-30

This records what a third-party service reported about its own creation stage, collected by SOMA.
It measures Isorun only.
It proves nothing about SOMA, and it is not an equivalent SOMA lifecycle milestone.

The corrections required of the first version of this document are recorded in [the review of this measurement](../reviews/2026-08-30-isorun-evidence-review.md) and are applied here.

## Evidence class

**Independently collected vendor-reported telemetry.**

`create_ms` is a field Isorun returns in its own create response.
Its timer endpoints are undocumented.
SOMA did not observe the interval it measures.

The harness wall clock is the only independently measured interval in this document, and it includes transport from a measuring host on another continent.

## Why this exists

`COMPETITORS.md` held two Isorun performance statements that cannot be compared with each other: a vendor claim of `create_ms` 9 repeated publicly as "ready in 10ms", and an independent ComputeSDK observation of 63.90 ms median burst time to interactive, which includes provider transport and a first command.

Neither is a creation-stage figure collected under controlled cohorts.
This run collects that field directly, at three concurrency levels, and retains every record.

## What `create_ms` is not known to include

The reviewed material does not establish whether the field covers admission queueing, worker allocation, memory restore, guest identity or entropy repair, network repair, guest authentication, or successful command execution.

SOMA's `Ready` requires all of those and one bounded command through the production executor.
Treat `create_ms` as an unknown vendor-defined creation stage and use it as competitive context only.
Do not present it beside a SOMA `Ready` figure as though the two measure the same interval.

## Method

One sample is `POST /v1/runs`, then `POST /v1/runs/{id}/exec`, then `DELETE /v1/runs/{id}`.
Every sample is destroyed in a `finally` block and its destruction response retained.

Concurrency is a `ThreadPoolExecutor` of the stated width.
There is no explicit start barrier, so the per-sample request-send offset is recorded instead and the observed dispatch window is reported below rather than assumed.

## Experiment metadata

- Collected 2026-08-30, approximately 02:59 to 03:10 UTC.
- Service endpoint `https://run-us.isorun.ai`, region `us`.
- Measuring host: Linux x86_64, Ubuntu 24.04, kernel 7.0.0-30-generic, network location Asia; roughly 145 ms TCP connect and 246 ms TLS establishment to the endpoint.
- Python 3.12 standard library only; request timeout 180 s; sandbox timeout parameter 300 s; **no retries**, so every attempt is one sample.
- Requested shape for every cohort: 1 vCPU, 1024 MiB memory, 4096 MiB disk.
- Harness: `benchmarks/competitive/isorun_create_latency.py`. Table generator: `benchmarks/competitive/regenerate_isorun_tables.py`.
- The credential is read from `ISORUN_API_KEY` and is never printed or persisted.

## Cohorts and raw records

Raw redacted records are retained in [`raw/2026-08-30-isorun`](raw/2026-08-30-isorun).
They contain timing, cohort, result, and cleanup fields only.
Every table cell below is recomputed from those files by the generator, which needs no Isorun account:

```sh
python3 benchmarks/competitive/regenerate_isorun_tables.py
```

| Cohort | concurrency | attempted | succeeded | destroyed | min | p50 | p95 | p99 | max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `node:22, sequential` | 1 | 10 | 10 | 10 | 20 | 22 | 27 | 27 | 27 |
| `node:22, concurrency 10` | 10 | 20 | 20 | 20 | 21 | 26 | 59 | 59 | 59 |
| `node:22, concurrency 100` | 100 | 100 | 100 | 100 | 19 | 73 | 179 | 207 | 260 |
| `node:22, concurrency 100, repeat` | 100 | 100 | 100 | 100 | 23 | 73 | 188 | 209 | 217 |
| `alpine:3.20, sequential` | 1 | 10 | 10 | 10 | 22 | 23 | 44 | 44 | 44 |
| `busybox:stable-musl, sequential` | 1 | 10 | 10 | 10 | 19 | 48 | 53 | 53 | 53 |
| **every cohort** | | 250 | 250 | 250 | 19 | 60 | 180 | 209 | 260 |

Values are `create_ms` in milliseconds, nearest-rank percentiles.
Zero attempts failed and all 250 sandboxes were destroyed.
`node:22` cohorts executed `/usr/local/bin/node --version` and returned `v22.23.2`; the `alpine` and `busybox` cohorts executed `/bin/busybox true`, which succeeds with empty output.

Billing scope: these 250 cohort samples were billed **0.1191 cents** in total, computed from the retained destruction records.

### Separate probes, excluded from the table

Three additional sandboxes were created outside the cohorts and are excluded from every figure above:

- One `node:22` create used to establish the request shape, billed 0.0152 cents for 13.7 s of uptime, which matches the published 0.04 dollars per hour exactly.
- One `denoland/deno:alpine-2.0.5` create, discussed below.
- One `node:22` create immediately after it, for comparison.

Including them, total spend for the session was approximately 0.13 cents.

## Observations

### The published 10 ms figure did not occur in this sample

No cohort sample reported 10 ms or less, and none reported 15 ms or less.
The lowest reported value across 250 samples was 19 ms.
Twenty-six of 250 were at or below 22 ms.

This is a statement about what was collected here, not a claim that the service can never report 10 ms.

### The reported value rose with concurrency in these cohorts

The sequential `node:22` cohort reported a p50 of 22 ms.
Two independent 100-request cohorts of the same image both reported a p50 of 73 ms, with p99 values of 207 ms and 209 ms.

The dispatch window for the recorded 100-request cohort was 261 ms from first to last request send, measured per sample, so the requests overlapped substantially but were not released by a barrier.

This document does not establish why the reported value rose.
Queueing, pool exhaustion, host contention, and instrumentation scope are all untested explanations.

### Hypothesis: the reported value may exclude image preparation

One create from `denoland/deno:alpine-2.0.5` reported `create_ms` 52 while the harness measured 4,808 ms wall clock.
A `node:22` create from the same host immediately afterwards reported 25 ms and completed in 283 ms.

This is **consistent with** the reported field excluding image acquisition and preparation for an image the service had not already prepared.
One request does not prove the image was uncached, nor identify what the additional time was spent on.
Confirming it would require repeated first-use requests across several previously unseen images.

### Hypothesis: the small-image bimodality may reflect pool behavior

`alpine:3.20` reported values clustered near 22 ms and near 44 ms; `busybox:stable-musl` near 20 ms and near 50 ms.
A smaller image did not produce a smaller reported value.

Bimodality of that shape is **consistent with** a warm resource being present for some requests and not others, but this document does not observe the mechanism and does not establish one.

## What this does not prove

- It does not measure SOMA, and no SOMA figure appears here.
- It does not reproduce the ComputeSDK cohort, which uses a different boundary, region, client, and workload.
- `create_ms` is vendor instrumentation with unknown endpoints, so the concurrency figures are a lower bound on what a caller experiences and cannot be equated with any SOMA stage.
- Sequential and small-image cohorts are ten samples each.
- One measuring host, one account, one service region, one day, no retries.
- Nothing here establishes the isolation, durability, or security properties of the service.

## Consequence for SOMA

The useful transferable result is not the competitor's figure.
It is that a creation-stage figure collected at one concurrency did not predict the same figure at another, in two agreeing cohorts.

SOMA must therefore treat concurrency as an independent benchmark dimension and publish every rung, as [the benchmark contract](../benchmark-contract.md) already requires, rather than extrapolating a sequential result.
The architectural consequence is developed in [the competitive module adoption audit](../research/competitive-module-adoption-audit.md).
