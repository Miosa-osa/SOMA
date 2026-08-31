# The speed ladder: what a configuration actually costs - 2026-08-31

## Capability status: Live-proved at `1cb15d9`

Twenty cohorts across four configurations and five concurrency levels, on one host and one commit.
**Every cohort succeeded completely: 740 of 740 sandboxes launched, ran their command, and were destroyed.**
Every sample is retained.

This record exists to answer one question with measurement rather than argument: which dimension of a
sandbox actually moves its time to first command.

## Observation identity

| Field | Observed value |
| --- | --- |
| Host | eval-1, Intel Xeon Gold 6138 at 2.00 GHz, 80 logical CPUs |
| Storage | XFS with reflink, warm page cache, one warming launch per configuration outside every cohort |
| Boundary | The receipt's `command_finished` milestone, which is where ComputeSDK stops. Destroy runs and is never counted |
| Path | Prepared restore, one process per sandbox, no jail, no network, uncertified Candidate |

Each configuration is a separate Generation with its own capture, because a restore maps exactly
the memory its snapshot was taken with. Memory is a build parameter, not a launch flag.

## The ladder

Time to first command, p50 in milliseconds.

| Configuration | c=1 | c=10 | c=25 | c=50 | c=100 |
| --- | ---: | ---: | ---: | ---: | ---: |
| busybox, 128 MiB, 1 GiB disk | 89.4 | 51.9 | 67.6 | 86.7 | 267.3 |
| busybox, 512 MiB, 10 GiB disk | 55.2 | 53.6 | 70.5 | 87.3 | 186.5 |
| busybox, 1024 MiB, 10 GiB disk | 75.2 | 92.1 | 64.6 | 80.3 | **118.5** |
| node:22, 1024 MiB, 10 GiB disk | **60.7** | 89.4 | 117.9 | 140.5 | 185.6 |

The `c=1` column is one sample per configuration and is not a distribution; it is reported because
it was measured, and no conclusion here rests on it.

## Three findings

**Less memory is worse under load, not better.** At concurrency 100 the 128 MiB configuration is
the slowest of the three busybox rungs at 267.3 ms, and the 1024 MiB configuration is the fastest
at 118.5 ms. The prediction before the run was that memory would barely matter, because a restore
maps its memory object privately with no eager copy. That prediction was wrong in the opposite
direction from the obvious one: a guest with 128 MiB cannot hold its own root in page cache, so it
faults against the immutable EROFS image repeatedly, and at a hundred sandboxes that reading
collides. Shrinking the machine does not make it faster.

**The session is a fixed cost.** The `machine_launched` to `ready` segment measured 28.6, 28.5,
29.1 and 29.6 ms at concurrency one across all four configurations. It does not move with memory
and it does not move with workload. It is the launch page, the vsock connection, the authenticated
handshake, the repair, and the readiness probe, and at concurrency one it is roughly half of a
60.7 ms result.

**Most of the node figure is node.** The command segment is 27.4 ms for `node --version` and 3.1 ms
for `busybox --help` on the same shape. About twenty-four milliseconds of the headline number is a
language runtime starting itself, which no virtual machine monitor can remove.

## Stage medians

Deltas in milliseconds: machine construction, then ready, then the command.

| Configuration | c=1 | c=100 |
| --- | --- | --- |
| busybox 128 MiB | 57.1 / 28.6 / 3.7 | 199.5 / 61.0 / 6.8 |
| busybox 512 MiB | 23.6 / 28.5 / 3.1 | 127.2 / 53.2 / 6.1 |
| busybox 1024 MiB | 36.5 / 29.1 / 9.6 | 54.4 / 58.1 / 6.1 |
| node:22 1024 MiB | 3.7 / 29.6 / 27.4 | 47.7 / 62.6 / 77.1 |

`machine_launched` is the private overlay head clone. It ranges from 3.7 ms to 199.5 ms for the
same operation on a filesystem where a reflink clone should be close to constant time regardless of
file size. That variance, rather than the mean, is the finding, and it is being measured separately.

## What this does not prove

- It is one host, one commit, and one cohort per cell. An earlier campaign measured about forty
  percent variation between repeats of one hundred-way cohort, so no single cell here should be
  treated as a point estimate.
- It is `soma run`, one process per sandbox. It is not the managed lifecycle the benchmark contract
  requires, and it is not the upstream ComputeSDK campaign.
- No configuration here used the network, a jail, a certified Generation, or a prepared worker.

## Retained artifacts

- [`raw/2026-08-31-speed-ladder/sweep-results.jsonl`](raw/2026-08-31-speed-ladder/sweep-results.jsonl), one record per cohort with its stage medians
- [`raw/2026-08-31-speed-ladder/sweep-prepare.sh`](raw/2026-08-31-speed-ladder/sweep-prepare.sh) and [`sweep-measure.sh`](raw/2026-08-31-speed-ladder/sweep-measure.sh), the exact harnesses
