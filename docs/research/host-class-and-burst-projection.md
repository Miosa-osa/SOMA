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

**The stage medians of a hundred-way cohort on eval-1.**
`machine_launched` 44.4 ms, Ready a further 53.4 ms, the command a further 74.5 ms, summing to 172.3 ms.
eval-1 is a dual Xeon Gold 6138: 40 cores, 80 threads, 2.0 GHz, a 2017 part.

## What follows arithmetically

Machine construction is the segment a prepared worker removes from the request path, and it is 44.4 ms of the 172.3 ms.
A prepared-worker path that removes all of it leaves about 128 ms on eval-1.

Isorun's observed Node cohorts returned 22 ms sequentially and 73 ms at concurrency 100, recorded in [the Isorun evidence review](../reviews/2026-08-30-isorun-evidence-review.md) as vendor telemetry rather than independently measured server timing.

So the conclusion that matters is arithmetic on measured values rather than a projection:

**eval-1 cannot produce a leading burst figure even with a perfect prepared-worker path.**
128 ms against 73 ms is not a close result, and no further software work on the machine-construction segment can close it, because that segment is already gone in the 128 ms.

This is a statement about the host, not about SOMA.
It means a burst campaign run on eval-1 measures a 2017 processor, and reporting its figure would understate the engine while appearing to measure it.

## Projection, not measurement

The following applies the measured per-core ratio and a simple model in which a cohort's wall clock is the total CPU work divided by the usable parallelism.
It ignores memory bandwidth, cross-core contention, and lock behaviour, all of which are real and none of which are modelled.
**These figures exist to choose a host class, not to be reported.**

| Host class | Threads | Sequential, projected | Burst c=100, projected |
|---|---:|---|---|
| eval-1, Xeon Gold 6138 x2 | 80 | 62.6 ms (measured) | 165 ms (measured) |
| Ryzen 9 9950X | 32 | about 21 ms | about 70 to 95 ms |
| EPYC 9654 class | 192 | about 27 ms | about 30 to 50 ms |
| Isorun, for comparison | unknown | 22 ms | 73 ms |

Two consequences follow, and they point at different parts.

**Sequential is won by core speed.**
One sandbox uses one core, so the fastest desktop part is the right host for a sequential figure, and a 9950X class part is projected to reach the sequential number Isorun reports.

**Burst is won by core count as well as core speed.**
A hundred sandboxes on 32 threads is three times oversubscribed, which is worse than eval-1's 1.25 even though every core is far faster.
A high-core-count server part is what a burst campaign needs, and on that class the projection clears the comparison before prepared workers are counted at all.

## Consequences

- A burst campaign must name its host class in the retained artifact, beside the cohort and the boundary.
- SOMA must not publish a burst figure measured on eval-1 as a competitive result. eval-1 remains a correct host for correctness, contention behaviour, capacity, and leakage work.
- Choosing the campaign host is a prerequisite for the campaign, not a detail of it, and the choice is a high-core-count current server part.
- The prepared-worker path remains the right work regardless: it is the only lever that removes the 44.4 ms, and it is worth the same proportion on any host.
