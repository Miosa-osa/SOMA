# The declared device set, re-measured on the merged binary - 2026-08-31

Every earlier figure for the read-only core was taken on its own branch, at 128 MiB, before three
branches were merged. This record re-measures both arms on **one binary** built from the merge
(`8ed0579`), from Generations prepared and captured by that same binary, at the shape the engine is
compared against: one vCPU, 1024 MiB, busybox.

The read-only arm is not a flag. Its captured snapshot contains **no `overlay.raw` at all**
(`memory.raw` and `state.somasnap` only), so the absent overlay device is a property of the
artifact rather than of the launch.

## Sequential, thirty samples each, after five warming launches

| Arm | ok | TTI p50 | TTI p95 | `machine_launched` | `ready` |
| --- | :--: | ---: | ---: | ---: | ---: |
| writable | 30/30 | 29.40 ms | 30.66 | 0.72 | 23.02 |
| read-only | 30/30 | **26.94 ms** | 29.49 | 0.45 | 20.35 |

At concurrency one the head clone is already nearly free, so removing it wins about 2.5 ms.

## Concurrency 100, six cohorts per arm, six hundred sandboxes each

Six hundred launches per arm, all successful. Cohorts one to three were interleaved between the
arms; four to six ran back to back within each arm, so neither arm holds a scheduling advantage.

| Arm | TTI p50 of each cohort | median | spread | `machine_launched` |
| --- | --- | ---: | ---: | --- |
| writable | 42.1, 45.9, 53.4, 67.5, 120.7, 133.0 | 60.5 ms | **3.2x** | 9.0 to 97.6 ms |
| read-only | 29.5, 35.2, 35.4, 35.7, 36.2, 37.7 | **35.5 ms** | **1.3x** | **0.3 ms, all six** |

Read-only is about 1.7 times faster at the median, and that is the smaller half of the result. The
writable arm's private head clone ranges over an order of magnitude between cohorts, from 9.0 to
97.6 ms, and **is the whole of the difference**: the two arms' `ready` segments track their
`machine_launched` values almost exactly. The read-only arm's clone is 0.3 ms in every one of six
cohorts, to one decimal place.

So the claim worth carrying is not only that removing the clone is faster. It is that **the clone
is the only unstable segment on the launch path at concurrency, and removing it removes the
instability with it.**

## A correction, recorded because it nearly shipped

A single cohort of each arm was measured first and read the other way: writable 28.1 ms against
read-only 34.8 ms. That pair is in the samples and is not wrong; it caught the writable arm on its
best cohort, which the six-cohort distribution above shows is its 42.1 ms tail rather than its
typical behaviour. One hundred-way cohort per arm is not a comparison, exactly as this repository's
own performance findings already warned, and the reversal was only caught by repeating.

The degradation was also checked for a harness fault before it was believed: the head directory
held **zero** files afterwards and `/srv` was at twelve percent, so no head leaked and no volume
filled. It is variance, not accumulation - cohorts four to six fall back to 45.9, 67.5 and 53.4
after the 133.0 peak.

## What this does not say

The comparison is busybox at one shape on one host. It does not measure a workload that writes,
which is the case a writable overlay exists to serve; a Generation that declares no writable
storage is a different product configuration, not a faster setting for the same one.
