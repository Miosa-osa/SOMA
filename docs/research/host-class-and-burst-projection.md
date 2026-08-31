# Host class and the burst result

## Decision

The measurement host is part of the result.
SOMA must state the host class its benchmark campaign runs on, and must not present a burst figure taken on a host class that cannot produce a competitive one.

This document records what was measured about host class, what follows arithmetically, and which parts are projection rather than measurement.
**Only the first section is measured. Everything after it is a model and may not be quoted as a result.**

## Measured

Two things are measured and retained.

**The per-core ratio between two hosts.**
The same code, restoring the same Generation, reached Ready in about 12 ms on a Core Ultra 9 275HX and about 29 ms on a Xeon Gold 6138, a ratio of roughly 2.4.
The retained comparison is [the current-authority capture and restore record](../evidence/2026-08-30-x86_64-snapshot-restore-current-authority.md).

**Five hundred-way cohorts and twenty-five sequential samples on eval-1**, retained in [the eval-1 burst and sequential record](../evidence/2026-08-31-eval1-burst-and-sequential.md).
Sequential p50 is 65.5 ms over 25 samples.
At concurrency 100 the cohort p50 ranges from 166.0 to 233.5 ms with a cohort median of 181.4 ms, and five hundred of five hundred sandboxes succeeded.
Stage medians across those cohorts are machine construction 48.0 ms, Ready a further 57.7 ms, and the command a further 79.0 ms.
eval-1 is a dual Xeon Gold 6138: 40 cores, 80 threads, 2.0 GHz, a 2017 part.

The cohort spread is about 40 percent of the median, so a single hundred-way cohort is not a reliable point estimate on this host and none of these figures may be quoted alone.

## What follows arithmetically

Machine construction is the segment a prepared worker removes from the request path, and it is 48.0 ms of the median cohort's 181.4 ms.
A prepared-worker path that removes all of it leaves about 133 ms on eval-1.

Isorun's observed Node cohorts returned 22 ms sequentially and 73 ms at concurrency 100, recorded in [the Isorun evidence review](../reviews/2026-08-30-isorun-evidence-review.md) as vendor telemetry rather than independently measured server timing.

So the conclusion that matters is arithmetic on measured values rather than a projection:

**eval-1 cannot produce a leading burst figure even with a perfect prepared-worker path.**
133 ms against 73 ms is not a close result, and no further software work on the machine-construction segment can close it, because that segment is already gone in the 133 ms.
The sequential comparison is worse rather than better: 65.5 ms against 22 ms is a factor of three, and sequential latency is the measure least helped by adding cores.

This is a statement about the host, not about SOMA.
It means a burst campaign run on eval-1 measures a 2017 processor, and reporting its figure would understate the engine while appearing to measure it.

## Projection, not measurement

The following applies the measured per-core ratio and a simple model in which a cohort's wall clock is the total CPU work divided by the usable parallelism.
It ignores memory bandwidth, cross-core contention, and lock behaviour, all of which are real and none of which are modelled.
**These figures exist to choose a host class, not to be reported.**

| Host class | Threads | Sequential, projected | Burst c=100, projected |
|---|---:|---|---|
| eval-1, Xeon Gold 6138 x2 | 80 | 65.5 ms (measured) | 166 to 234 ms (measured) |
| Ryzen 9 9950X | 32 | about 22 to 27 ms | about 80 to 110 ms |
| EPYC 9654 class | 192 | about 28 to 33 ms | about 35 to 60 ms |
| Isorun, for comparison | unknown | 22 ms | 73 ms |

Two consequences follow, and they point at different parts.

**Sequential is won by core speed.**
One sandbox uses one core, so the fastest desktop part is the right host for a sequential figure.
A 9950X class part is projected to reach roughly the sequential number Isorun reports rather than to beat it, so sequential is a contest SOMA can enter on the right host and not one it currently wins.

**Burst is won by core count as well as core speed.**
A hundred sandboxes on 32 threads is three times oversubscribed, which is worse than eval-1's 1.25 even though every core is far faster.
A high-core-count server part is what a burst campaign needs, and on that class the projection clears the comparison before prepared workers are counted at all.

## Consequences

- A burst campaign must name its host class in the retained artifact, beside the cohort and the boundary.
- SOMA must not publish a burst figure measured on eval-1 as a competitive result. eval-1 remains a correct host for correctness, contention behaviour, capacity, and leakage work.
- Choosing the campaign host is a prerequisite for the campaign, not a detail of it, and the choice is a high-core-count current server part.
- The prepared-worker path remains the right work regardless: it is the only lever that removes the 48.0 ms, and it is worth the same proportion on any host.
