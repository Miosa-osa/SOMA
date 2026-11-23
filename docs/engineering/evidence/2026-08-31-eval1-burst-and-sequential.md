# Burst and sequential TTI on eval-1 - 2026-08-31

## Capability status: Live-proved at `b65f41f`

Five hundred sandboxes across five hundred-way cohorts, and twenty-five sequential samples, all on one commit and one host.
Every sample is retained.
This record replaces earlier figures that were observed but never written down; where it contradicts them, this record is the one with files behind it.

## Observation identity

| Field | Observed value |
|---|---|
| Host | eval-1, one bare-metal Ubuntu host |
| CPU | Intel Xeon Gold 6138 at 2.00 GHz, 80 logical CPUs |
| Kernel | Linux 6.8.0-138 |
| Memory | 156 GB, about 124 GB free before each cohort |
| Storage | XFS on `/srv` with `reflink=1`, 1.5 TB free |
| SOMA revision | `b65f41f` |
| Generation | `node:22`, prepared and captured on this commit |
| Path | Prepared restore from a snapshot taken at the pre-launch repair point, runtime warmed before capture |
| Boundary | The receipt's `command_finished` milestone, which is where ComputeSDK stops. Destroy runs and is never counted |

The harness releases every slot from one barrier and parses no receipt until the last sandbox has exited, so no interpreter competes with the measurement.
A sample counts only when the guest returned the version it was asked for.

## Sequential

Twenty-five samples, one sandbox at a time.

| Measure | ms |
|---|---:|
| p50 | 65.5 |
| p95 | 73.5 |
| min | 61.1 |
| max | 80.2 |

Twenty-five of twenty-five succeeded.
The distribution is tight, so the p50 is meaningful.

## Burst at concurrency 100

Five cohorts of one hundred.
**Five hundred sandboxes attempted, five hundred succeeded.**

| Cohort p50, sorted | ms |
|---|---:|
| fastest | 166.0 |
| | 170.5 |
| median cohort | 181.4 |
| | 195.9 |
| slowest | 233.5 |

The spread between the fastest and slowest cohort is 67.5 ms, about 40 percent of the median.
**A single hundred-way cohort is therefore not a reliable point estimate on this host, and no one of these numbers may be quoted alone.**
The honest statement is that concurrency 100 on this host and commit produces a p50 between 166 and 234 ms, with a cohort median of 181.4 ms.

Cohort order was not a factor: the two fastest cohorts were the third and fifth run, so nothing accumulated across runs.
The host was checked between runs and held no leaked overlay heads, no stray processes, 124 GB free memory, and 1.5 TB free storage.

## Where the time goes

Stage medians across the five cohorts, as deltas, measured on the same sandboxes as the totals above.

| Stage | median ms | min | max |
|---|---:|---:|---:|
| machine launched | 48.0 | 36.7 | 59.2 |
| ready | 57.7 | 53.0 | 61.6 |
| command finished | 79.0 | 72.9 | 117.6 |

**Machine construction is 48 ms of the median cohort's 181 ms, and none of it depends on the Instance.**
The same segment measured 2.71 ms uncontended on a developer laptop, retained in [the restore stage timeline](2026-08-30-x86_64-restore-stage-timeline.md), so most of the 48 ms is contention for work that did not have to happen on the request path.
This is the measured case for the [prepared worker protocol](../research/prepared-worker-protocol.md), and it is the first time that case rests on a retained artifact rather than an observation.

Removing all of it would leave about 133 ms at the cohort median, which is the number to compare against a competitor rather than 181.

## What this record does not prove

- It is one host and one Generation. Nothing here is a claim about SOMA on any other host class.
- It does not prove prepared workers, capacity admission, jailing, networking, certification, or a persistent Host Runtime. The measured path is the development command line, one process per sandbox.
- It is not the upstream ComputeSDK campaign. It measures the same boundary with SOMA's own harness; passing a local harness is not the same as running the unmodified upstream benchmark against a provider adapter.
- The 133 ms figure above is arithmetic on the measured stage median, not a measurement. No prepared worker exists to have produced it.

## Retained artifacts

All under [`raw/2026-08-31-eval1-burst-b65f41f/`](raw/2026-08-31-eval1-burst-b65f41f/):

- `burst-c100-b65f41f.jsonl` through `...-run5.jsonl`, one line per sandbox, five hundred samples
- the matching `.summary` files, one per cohort, each carrying its own stage medians
- `burst-c100-b65f41f-AGGREGATE.json`, the five-cohort aggregate quoted above
- `seq-n25-b65f41f.jsonl`, the twenty-five sequential samples
- `burst-tti.sh` and `seq.sh`, the exact harnesses, retained so the boundary they measure is inspectable
