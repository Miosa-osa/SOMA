# Isorun create latency, independently measured - 2026-08-30

This is an independent observation of a third-party service, recorded so that SOMA compares itself against a measured boundary rather than a published claim.
It measures Isorun only.
It proves nothing about SOMA.

The evidence and claim-language corrections required before this result is treated as reproducible are recorded in [the review of this measurement](../reviews/2026-08-30-isorun-evidence-review.md).

## Why this exists

`COMPETITORS.md` recorded two Isorun performance statements that cannot be compared with each other:

- A vendor claim of a `create_ms` of 9 in an illustrative API-reference payload, repeated on the public site as sandboxes "ready in 10ms".
- An independent ComputeSDK observation of 63.90 ms median burst time to interactive, which includes provider transport and the first command.

Neither is the server-side create boundary SOMA's own budget is written against.
This run measures that boundary directly.

## Boundary

One sample is one `POST /v1/runs`, one `POST /v1/runs/{id}/exec`, and one `DELETE /v1/runs/{id}`.

Two quantities are retained per sample:

- `create_ms`, the value Isorun itself returns in the create response. This is server-side and therefore independent of the measuring host's network position. It is the quantity compared with SOMA's restore-to-Ready interval.
- Wall-clock time measured by the harness. This includes transport from the measuring host and is retained only for context.

The measuring host is in Asia and the service region is `us`, so wall-clock numbers carry roughly 145 ms of connection latency and must not be compared with any SOMA number or with the ComputeSDK cohort.

## Identities

- Service: `https://run-us.isorun.ai`, region `us`, authenticated with an account API key that is not recorded here.
- Requested shape: 1 vCPU, 1024 MiB memory, 4096 MiB disk, 300 s timeout.
- Workloads: `node:22` executing `/usr/local/bin/node --version`; `alpine:3.20` and `busybox:stable-musl` executing `/bin/busybox true`; `denoland/deno:alpine-2.0.5` for the cold-image probe.
- Harness: a standard-library Python client, nearest-rank percentiles, every sandbox destroyed in a `finally` block.
- Date: 2026-08-30. Samples: 250 successful sandboxes. Total billed cost: 0.13 cents.

## Result

`create_ms` as reported by the service.

| Cohort | n | min | p50 | p95 | p99 | max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `node:22`, sequential | 10 | 20 | 22 | 27 | 27 | 27 |
| `node:22`, concurrency 10 | 20 | 21 | 26 | 59 | 59 | 59 |
| `node:22`, concurrency 100 | 100 | 19 | 73 | 179 | 207 | 260 |
| `node:22`, concurrency 100, repeat | 100 | 23 | 73 | 188 | 209 | 217 |
| `alpine:3.20`, sequential | 10 | 22 | 23 | 44 | 44 | 44 |
| `busybox:stable-musl`, sequential | 10 | 19 | 48 | 53 | 53 | 53 |
| Every sample | 250 | 19 | 60 | 180 | 209 | 260 |

Every cohort returned a successful command with exit code 0, and `node:22` returned `v22.23.2`, which is the same build SOMA's own `node:22` Generation returns.

## Three observations

### The published 10 ms did not occur

No sample reached 10 ms, and none reached 15 ms.
The fastest single create observed across 250 samples was 19 ms.
Only 26 of 250 samples were at or below 22 ms.

A smaller image did not help.
`alpine:3.20` and `busybox:stable-musl` were bimodal at roughly 20 ms or roughly 45 to 53 ms, which is consistent with a warm-pool hit and miss rather than with image size.

### Concurrency degrades the service by roughly three times

Sequential `node:22` creates were 22 ms at p50.
The same image at concurrency 100 was 73 ms at p50 and about 208 ms at p99, reproduced across two independent runs whose p50 agreed exactly.
The requests were a genuine burst: all 100 left the measuring host within a 261 ms window, recorded per sample.

### The reported create time excludes image preparation

A create from `denoland/deno:alpine-2.0.5`, an image the service had not prepared, reported `create_ms` of 52 while the caller waited 4,808 ms.
A subsequent `node:22` create from the same host reported 25 ms and completed in 283 ms.

The reported quantity therefore excludes image acquisition and preparation.
This is the accounting boundary SOMA's [benchmark contract](../benchmark-contract.md) requires to be reported as a separate preparation class rather than folded into a create result.

Billing was accurate: 13.7 s of uptime was billed 0.0152 cents, which is exactly the published 0.04 dollars per hour.

## What this does not prove

- It does not measure SOMA. No SOMA number appears in this document.
- It does not reproduce the ComputeSDK cohort, which uses a different boundary, region, client, and workload.
- `create_ms` is the service's own instrumentation. Whether it includes admission queueing is unknown, so the concurrency figures are a lower bound on what a caller experiences.
- Sample counts are small for the sequential cohorts and the two tiny-image cohorts.
- One measuring host, one region, one account, one day.
- Nothing here establishes the isolation, durability, or security properties of the service.

## How to reproduce

The harness lives outside the repository because it requires a third-party account credential.
It performs create, exec, destroy per sample with a fixed shape, records `create_ms` and the harness wall clock, records the per-sample request-send offset so the burst window can be checked, and destroys every sandbox it creates.
