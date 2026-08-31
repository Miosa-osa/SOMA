# What a restored guest's resume actually spends its time on - 2026-08-31

## Capability status: Live-proved at `03039729`, diagnostic only

This record answers two questions that were open at the same time. Whether demand paging of the
private memory mapping is a meaningful share of the `ready` segment, and what the unattributed
milliseconds between arming the vCPU and the guest doing anything observable actually are.

They turned out to be the same question, and the answer is a specific mechanism that neither of
the instruments in place could see.
**No number here is a latency result and none may be compared with a benchmark figure.**

## Observation identity

| Field | Observed value |
|---|---|
| Host | eval-1, one bare-metal Ubuntu host |
| CPU | Intel Xeon Gold 6138 at 2.00 GHz, 80 logical CPUs |
| Kernel | Linux 6.8.0-138, `fault_around_bytes` 65536, transparent huge pages on `madvise` |
| Memory | 156 GB, page cache warm after a launch outside every cohort |
| Storage | XFS on `/srv` with `reflink=1` |
| SOMA revision | `03039729`, a branch off `cff352f`. Both arms of every comparison are this one binary |
| Generations | `/srv/soma/sweep/{bb-128-1024,bb-1024-10240,node-1024-10240}`, prepared before this branch |
| Path | Prepared restore, one process per sandbox, the development command line |

The Generations predate the wire-contract change on main that removed the readiness probe, and
this branch predates it too, so the two match. A newer binary could not have launched them.

## The instrument this needed

The receipt says the machine became ready. The guest agent's own clock reads zero across the
window between the vCPU being armed and the guest reaching the launch page, so guest-side
instrumentation is blind to it. Two milestones were added inside the vCPU worker, on each side of
the first `KVM_RUN` call, and every return from `KVM_RUN` is now counted by class, with the first
thousand also timed. `SOMA_KVM_TIMELINE` carries the new milestones with no change anywhere else.

The instrument costs nothing that can be measured: `ready` at concurrency one is 29.96 ms for
`node:22` against 30.49 and 29.09 ms on the two uninstrumented passes, and 28.17 ms for `busybox`
against 29.68 and 28.68 ms.

## Where the resume window goes

Twenty one sequential samples per configuration, medians in milliseconds.

| Step | busybox 128 MiB | busybox 1024 MiB | node:22 1024 MiB |
| --- | ---: | ---: | ---: |
| Armed to first `KVM_RUN` entry | -0.011 | -0.015 | -0.010 |
| The first `KVM_RUN` call | 0.552 | 0.574 | 1.900 |
| First return to launch page erased | 6.726 | 5.308 | 3.121 |
| **Armed to launch page erased** | **7.257** | **5.839** | **5.002** |
| Armed to ready | 24.721 | 24.794 | 25.788 |

Arming the vCPU costs nothing: the worker thread is already inside `KVM_RUN` about ten
microseconds before the main thread records the milestone. The boundary between the first call
and everything after it moves a long way between Generations while the block does not, which is
what the launch-path audit saw from outside and could not explain.

## What the vCPU is doing in that window

`perf record -e kvm:kvm_entry,kvm:kvm_exit` on one `node:22` launch, windows taken from the same
launch's own milestones.

| Window | Wall | Guest execution | In-kernel exit handling | Exits | of which EPT violations |
| --- | ---: | ---: | ---: | ---: | ---: |
| The first `KVM_RUN` call | 2.44 | 0.90 | 1.54 | 181 | 168 |
| Armed to launch page erased | 6.75 | 2.66 | 4.09 | 505 | 459 |
| Armed to ready | 28.49 | 11.69 | 16.80 | 3600 | 3199 |

**Eighty nine percent of the exits are EPT violations, and none of them return to userspace.**
KVM resolves them in the kernel and re-enters the guest, so the vCPU worker sees one long
`KVM_RUN` call and the guest sees nothing at all. That is precisely why both existing instruments
were blind: the userspace loop is idle and the guest clock does not advance.

There is no stall anywhere in the first ten milliseconds: no gap between consecutive KVM events
exceeds 200 microseconds, and there are three `HLT` exits. The window is therefore not the first
entry, not restored-clock timer catch-up, and not a vCPU halted on a restored APIC deadline. It
is the guest running normally and taking one EPT violation per four-kilobyte guest page it
touches for the first time.

## What the guest actually touches

One restored `node:22` at 1024 MiB, read from `/proc/<pid>/smaps` while the guest idles after
ready. Three samples, all within eight pages of each other.

| Quantity | node:22 1024 MiB | busybox 1024 MiB |
| --- | ---: | ---: |
| Pages of the image resident | 4308 (16.8 MiB of 1024) | 3655 (14.3 MiB) |
| Of those, private copies | 1434 | 667 |
| Process minor faults, whole run | 1979 | 1145 |
| Process major faults | 0 | 0 |

This confirms the launch-path audit's count independently, on the same host with a different
instrument. The guest touches under two percent of its image, and those pages cluster: 4308 pages
fall in 22 of the image's 512 two-megabyte regions, 196 pages per region on average.

Host fault cost, measured directly against the same file: a copy-on-write write fault is 3.5 to
4.1 microseconds and a read fault is 2.7 to 7.3 microseconds while bringing sixteen pages at once,
because `fault_around_bytes` is 65536. Sixteen hundred host faults therefore cannot be more than
about six milliseconds, and the audit's arithmetic on that point was right.

## The decisive experiment

If demand paging of the host mapping were the cost, making the whole image resident before the
vCPU is armed would remove it. `SOMA_KVM_PREFAULT_MEMORY` walks the mapping once, reading one
byte per page. Both arms are the same binary, interleaved, twenty samples each.

| Measure | Cold mapping | Pre-faulted mapping |
| --- | ---: | ---: |
| Pages of the image resident | 4306 | 262144 |
| Of those, clean pages no copy was made of | 2874 | 260702 |
| Private copies the guest made | 1432 | 1439 |
| Process minor faults | 1974 | 35354 |
| The first `KVM_RUN` call, ms | 1.945 | 1.983 |
| Armed to launch page erased, ms | 5.117 | 5.102 |
| Armed to ready, ms | 25.672 | 24.561 |
| EPT violations in the first 30 ms | 3396 | 3465 |
| Accepted to ready, ms | 29.384 | 86.100 |

**The result is negative and it is clean.** The mapping goes from four thousand resident pages to
all two hundred and sixty two thousand, the process takes eighteen times as many minor faults
getting there, and not one part of the resume moves. The EPT violations are unchanged, because
the extended page table is a second page table that KVM populates on the guest's first access
whether or not the host page-table entry is already present. The walk costs 57 ms on the launch
path, which is what an eager copy is worth avoiding.

The mapping's guarantees survive the experiment, which is worth stating because it is the reason
the walk is a read: 260702 of the resident pages are clean pages of the shared page cache, and
the guest still made exactly its own 1439 private copies.

## Huge pages, and why they were never going to work

Measured before this line of enquiry was closed, and retained because the mechanism explains the
result above. The image was copied into a `tmpfs` mounted `huge=always`, confirmed to be backed
entirely by two-megabyte folios, and bind-mounted into a reflinked Generation.

| Measure | XFS image | Huge-page image |
| --- | ---: | ---: |
| `ready` at concurrency 1, node:22, ms | 30.49 and 29.09 | 31.32 and 30.46 |
| `ready` at concurrency 100, node:22, ms | 46.33 and 39.20 | 55.84 and 60.48 |
| EPT violations in the first 30 ms | 3396 | 3441 |
| Host read faults per 12 MiB touched | 192 | 6 |
| Copy-on-write fault cost, ns | 3472 to 4090 | 6324 to 7018 |

Read faults collapse by a factor of thirty two and nothing else does. A private mapping cannot
hold a huge extended-page-table entry, because any guest write must be able to copy one page
rather than five hundred and twelve, so the EPT stays at four-kilobyte granularity and the exit
count does not move. Meanwhile each copy-on-write fault becomes seventy percent more expensive,
because the huge mapping must be split first. At concurrency one hundred it is measurably worse.

## What this leaves

The `ready` segment's largest single mechanical cost is 3199 EPT violations taken by the guest
during its resume, costing about 16.8 ms of in-kernel handling against 11.7 ms of guest execution.
It is proportional to the number of distinct guest pages the resume touches and to nothing else:
not to the size of the image, not to the residency of the host mapping, and not to the host page
size behind it.

Two levers follow from that and neither is in this record's scope. A guest that touches fewer
distinct pages during its resume pays proportionally less. A mapping that could carry huge
extended-page-table entries would collapse the count by orders of magnitude, and the only known
ways to get one give up either the privacy of a guest's writes or the sharing of one image across
a hundred Instances, both of which are load-bearing.

## Retained artifacts

All under [`raw/2026-08-31-restore-resume-page-in/`](raw/2026-08-31-restore-resume-page-in/):

- `resume-windows.txt`, every resume-window table above, sequential and at concurrency 100
- `kvm-exit-anatomy.txt`, the exit breakdown for the cold, pre-faulted and huge-page-backed runs
- `pagein-state.txt`, the resident-page state of both arms, three samples each
- `fault-cost.txt`, the host fault-cost microbenchmark, including what `MAP_POPULATE` costs
- `cohort-summary.txt` and `results-*.jsonl`, five cohort passes at concurrency 1 and 100
- `probe.c`, `poller.c`, `smaps.py`, `spread.py`, `an3.py`, `resume.py`, and the shell harnesses,
  retained so every boundary these numbers measure is inspectable

## What this record does not prove

- It is one host, one commit, and three Generations. Nothing here is a claim about any other host.
- It is not a time-to-first-command result and must not be compared with a competitor figure.
- The `SOMA_KVM_TIMELINE` output it rests on is a diagnostic with no signature, no identity
  binding, and no stable schema.
- `SOMA_KVM_PREFAULT_MEMORY` is a measurement lever with a negative result behind it. It must
  never be set on a host serving requests, and it should be deleted the day nobody intends to
  re-run this comparison.
