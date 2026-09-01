# Where the durable path's second and a half goes - 2026-09-01

Measured on eval-1 at `ac330a1` (baseline) and `813df1c` (changed), KVM backend, busybox at one
vCPU, 1024 MiB of memory and 2048 MiB of storage, against a Generation compiled and captured by
the same binary at that shape. Every sample is retained under
[`raw/2026-09-01-durable-path-attribution/`](raw/2026-09-01-durable-path-attribution/); the host
is described in [`host.txt`](raw/2026-09-01-durable-path-attribution/host.txt).

The question was gap 1 of [the handover](../HANDOVER.md): the ComputeSDK contract benchmark passes
100 of 100 at concurrency 100 and takes 1.31 to 1.60 s to first command, against 20.5 ms for
`soma run`.

## The answer in one line

**Four durable state writes stand between a caller and its first command, and on this host each
one is a synchronous commit to a RAID10 of rotational disks.** Move only the state root to tmpfs,
change nothing else, and the same binary on the same host goes from 1318.7-1997.7 ms to
307.3-322.7 ms at the same hundred-way cohort.

## What each measured stage is

The stage names the benchmark prints are milestone deltas in a receipt, and each one is produced by
an exact piece of code. Read from the source rather than guessed:

| Stage | Milestones | What runs between them |
| --- | --- | --- |
| launch: workload resolution | `accepted` to `workload_resolved` | `KvmBackend::resolve`, which reads the prepared entry |
| launch: admission | `workload_resolved` to `admitted` | `Engine::admit_launch`, which is exactly one `FileStateStore::create` |
| launch: machine creation | `admitted` to `machine_launched` | The host's own launch stamp, clamped to this process's clock |
| launch: readiness | `machine_launched` to `ready` | Spawning the host process, restoring the machine, reaching an authenticated session |
| destroy: cleanup | `cleanup_started` to `cleanup_finished` | `host::cleanup`: the round trip, `cleanup_resident`, and the close |

Two facts follow from that table and are the reason the receipt alone was misleading. The
`admitted` stamp is taken by the Backend at the top of `launch`, **after** the durable write, so
`launch: admission` is the write and nothing else. And the two durable writes the exec process
makes, and the one the launch process makes after `ready`, are all outside every named stage, which
is why they were showing up as "harness: process and transport overhead".

## Where the time actually goes

Recomputed from the retained samples, which already carry each process's wall duration beside its
receipt, so this needed no new instrumentation. Medians of one hundred successful samples per
cohort ([`attribution-baseline.txt`](raw/2026-09-01-durable-path-attribution/attribution-baseline.txt)):

| | run 1 | run 2 | run 3 |
| --- | ---: | ---: | ---: |
| **time to first command** | **1793.46 ms** | **1319.21 ms** | **1999.59 ms** |
| launch process | 833.14 | 758.49 | 1467.44 |
| &nbsp;&nbsp;launch: admission - **one durable write** | 337.26 | 329.30 | 1047.47 |
| &nbsp;&nbsp;launch: readiness - the machine | 79.62 | 29.48 | 30.75 |
| &nbsp;&nbsp;outside the facade - **one durable write** plus process start | 394.44 | 400.82 | 411.88 |
| exec process | 985.15 | 583.34 | 563.84 |
| &nbsp;&nbsp;exec: command execution - the guest | 11.86 | 12.37 | 14.69 |
| &nbsp;&nbsp;outside the facade - **two durable writes** plus process start | 977.38 | 569.54 | 548.04 |
| gap between the two processes | 0.57 | 0.72 | 0.68 |
| destroy: cleanup - the release | 905.22 | 861.72 | 1001.48 |

The machine is 29 to 80 ms of it and the guest command is 12 to 15 ms. Everything else is durable
state and process start.

## The substitution that proves the mechanism

The same release binary, the same host, the same overlay heads on the same XFS, the same
concurrency, with one thing changed: the durable state root moved from `/srv` to `/dev/shm`. Three
cohorts of one hundred, each 100 of 100 successful and 100 of 100 cleanup-complete
([`tmpfs-state/`](raw/2026-09-01-durable-path-attribution/tmpfs-state/),
[`attribution-tmpfs.txt`](raw/2026-09-01-durable-path-attribution/attribution-tmpfs.txt)):

| | state root on `/srv` | state root on tmpfs |
| --- | ---: | ---: |
| time to first command, p50 | 1318.7, 1787.2, 1997.7 ms | **322.7, 320.5, 307.3 ms** |
| launch: admission, p50 | 329.30, 337.26, 1047.47 ms | **0.17, 0.20, 0.19 ms** |
| launch: readiness | 29.48 to 79.62 ms | 18.71 to 25.18 ms |
| exec: command execution | 11.86 to 14.69 ms | 10.23 to 11.08 ms |

Host busy fraction sampled beside each cohort was 0.22 to 0.30 for the `/srv` arm and 0.235 to
0.238 for the tmpfs arm, so the two arms saw comparable neighbours; the samples are in each arm's
`host.txt`. This is not a supported configuration and is not proposed as one. It is the control
that says which component the time belongs to.

## Why the writes cost what they do

`/srv` is an XFS filesystem on an LVM volume on a RAID10 of **rotational** disks
([`host.txt`](raw/2026-09-01-durable-path-attribution/host.txt)). A durable write in this store
commits three times: the record's data, the directory after the record is linked into it, and the
directory again after the temporary name is removed. Three barriers against a disk revolution is
the 54 ms a single uncontended `launch: admission` costs on this host, and four such writes before
the first command is the second and a half.

Isolated from everything else, one durable write against that state root
([`component/state-store-write.txt`](raw/2026-09-01-durable-path-attribution/component/state-store-write.txt)):

| Writers | Median of six rounds, before | Median of six rounds, after |
| --- | ---: | ---: |
| 1 | 28.28 ms | **22.86 ms** |
| 100 | 273.08 ms | 260.35 ms |

The same measurement with the state root on tmpfs is **0.54 to 1.19 ms at a hundred writers**,
which also rules out the shard lock: the store's own serialisation is not what a hundred callers
are waiting on.

## The change made, and the clean negative

One of the three commits protects nothing. A crash between the link and the unlink leaves the
temporary name behind, and `scan_revisions` already sweeps temporary names before it checks that a
revision has exactly one link, which `publication_recovers_when_crash_leaves_the_committed_temp_link`
proved before this change existed. So the name is now retired before a single directory commit
rather than after a second one, the data commit is `fdatasync` rather than `fsync`, and the mode
rewrites that every state operation performed on the state root, the lock directory and the shard
lock now read the mode first (`45eda40`).

It is worth 28.28 to **22.86 ms** on an uncontended durable write, and **it does not move the
hundred-way contract benchmark at all**. Six cohorts per arm, the two arms alternating within each
round ([`after/`](raw/2026-09-01-durable-path-attribution/after/),
[`baseline/`](raw/2026-09-01-durable-path-attribution/baseline/)), all 100 of 100 successful and
100 of 100 cleanup-complete:

| Block | Order | before, p50 | after, p50 |
| --- | --- | ---: | ---: |
| runs 4-6, no settle between cohorts | baseline first | 1263.5, 1686.3, 1387.2 ms | 2220.5, 2082.3, 2253.2 ms |
| runs 7-9, 45 s settle between cohorts | after first | 1360.0, 1328.5, 1415.6 ms | 1231.4, 1483.3, 1456.7 ms |

The first block says the change is catastrophic and the second says it is nothing. What actually
differs between them is which arm ran second: a cohort that starts while the previous cohort's
hundred overlay heads are still being freed pays for them. The arm that ran first won four of the
five pairs regardless of which arm it was. **The controlled block is the second one, and in it the
two arms are indistinguishable.** That is what the component measurement already said: concurrent
commits batch into shared journal commits, so how many commits each writer asks for stops mattering
once a hundred writers are asking together.

`soma run`, the one-shot path, does not touch the state store and did not move: 25 sequential
samples per arm, two alternating rounds
([`soma-run.txt`](raw/2026-09-01-durable-path-attribution/soma-run.txt)):

| Arm | Time to first command, p50 | `ready` segment |
| --- | ---: | ---: |
| before | 30.52, 30.43 ms | 22.99, 22.65 ms |
| after | 30.32, 30.27 ms | 22.92, 22.99 ms |

## Two things found on the way

**A state root three bytes too deep silently failed a hundred launches out of a hundred.** The
first cohort of this work reported `backend_unavailable` a hundred times with no reason retained
anywhere. `sockaddr_un.sun_path` holds 108 bytes; the machine host's socket name is the state root
plus `machines/` plus a 32-character identity plus `.sock`, and the harness's own state directory
put it at 108 plus a terminator. Every `bind` failed with the error a missing directory gives, and
the host exited before it could say anything. The length is a property of the directory rather than
of the request, so it is now answered once before any process is spawned and reported as
`unsupported` (`813df1c`).

**`preparation: on_demand` on this path is correct and is worth about 3 ms.** The prepared machine
pool lives in `KvmBackend`, and a managed launch builds its machine in a **freshly spawned host
process** whose pool is necessarily empty. The pool took machine construction from 3.27 ms to
18.4 us, so wiring it here is worth about 3 ms against a 1.6 s figure. It is not the reason this
path is slow.

## What this record does not say

- Nothing here measures the release. `destroy: cleanup` is 862 to 1001 ms at a hundred and 81 ms at
  one, and it is unaffected by the state root, so it is not durable state; it is the head release,
  and no experiment here separates the munmap from the extent freeing. It is the free side of the
  clone serialisation already root-caused in gap 4 of the handover, stated here as a hypothesis
  rather than a finding.
- The `outside the facade` rows contain a durable write **and** a process start and this record
  does not separate them. On the tmpfs arm, where the writes are free, they are 122 to 160 ms each,
  which bounds process start from above at roughly that.
- No arm ran on a second host, and no arm ran on non-rotational storage. Whether a durable write
  costs microseconds on an NVMe state root is inferred from the tmpfs control, not measured.
- The host carried other agents' work throughout. Every cohort has its own `/proc/stat` busy
  fraction recorded beside it, and the ordering artefact above is what happens when that is
  ignored.

## What would actually close gap 1

In the order the evidence puts them:

1. **The state root's storage.** It is 75 to 80 percent of time to first command on this host and
   nothing in SOMA's code changes that. Whatever else is done, a durable path measured against
   other providers must not be measured with its state root on spinning disks.
2. **The two process spawns.** With the writes free, they are the largest remaining term at roughly
   250 to 320 ms of a 310 ms figure. The benchmark's boundary includes them by design, and a real
   ComputeSDK client holds one process rather than spawning two, so the honest comparable figure is
   the HTTP surface rather than the command line.
3. **The release**, at 862 to 1001 ms, which is not in time to first command but is in wall time and
   in every cleanup guarantee.
4. **The machine**, at 29 to 80 ms, which is already the smallest term and was never the problem.
