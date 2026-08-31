# What actually costs time, and what changed - 2026-08-31

One table per question, so a reader can see which dimension moves a sandbox's time to first
command and which does not. Every figure here was measured on eval-1, an Intel Xeon Gold 6138 at
2.00 GHz with 80 logical CPUs and XFS with reflink, unless a row says otherwise. Each links to the
record that retains its samples.

**Read the caveats before quoting anything.** A single hundred-way cohort varies by about forty
percent between repeats, so no single cell below is a point estimate.

## Two workloads, and why both

Every measurement below runs one of two things inside the sandbox, and the difference between them
is the point rather than an accident.

`node:22` is the official Node.js image, about four hundred megabytes, and it is what the public
benchmark this engine is measured against runs. A figure taken with it is comparable to a
competitor's figure.

`busybox` is an unrelated upstream project that packs roughly four hundred Unix utilities into one
executable of a few megabytes; a minimal Linux system is often nothing but that single binary. It
does almost nothing when it starts, which is exactly why it is here: it is the control. Running it
measures what the engine costs, because the workload costs nearly nothing.

Without the control the two costs are one number and cannot be separated. With it they can:
`node --version` spends 27.4 ms and `busybox --help` spends 3.1 ms on the same machine, so about
twenty-four milliseconds of any Node figure is the language runtime starting itself rather than
anything this engine does.

## Where the time goes in one launch

Stage deltas in milliseconds, `node:22` at one vCPU and 1024 MiB, from
[the speed ladder](2026-08-31-speed-ladder.md).

| Stage | What happens in it | c=1 | c=100 |
| --- | --- | ---: | ---: |
| machine launched | The private overlay head is cloned | 3.7 | 47.7 |
| ready | Launch page, vsock, authenticated handshake, repair | 29.6 | 62.6 |
| command | The workload runs | 27.4 | 77.1 |

## What was optimised, and by how much

| Change | Before | After | Mechanism |
| --- | ---: | ---: | --- |
| [Ephemeral head durability](raw/2026-08-31-eval1-head-sync/) | 72.4 ms | **6.8 ms** | The head was being `fsync`ed. It is unlinked before the clone returns and dies with its machine, so durability bought nothing |
| [Prepared machine pool](2026-08-31-prepared-machine-request-path.md) | 3.27 ms | **18.4 µs** | Machine construction moved off the request path. Two to three orders of magnitude on this host |
| Netd activation | 11.4 ms | **0.80 ms** | Four read-only questions per lifecycle were asked by running `nft`; they are now netlink queries |
| Netd release | 59.8 ms | **49.6 ms** | Same change |
| [Readiness proved by the repair report](2026-08-31-eval1-ready-segment-split.md) | 27.59 ms | **22.60 ms** | The receipt binds a Noise transcript that is already fixed when the handshake completes, so running a command afterwards attested nothing the receipt could carry. Confirmed at eight times the memory, so it is a fixed cost removed rather than a proportional one |
| [Declared device set, read-only root](2026-08-31-merged-binary-device-set-c100.md) | 60.5 ms | **35.5 ms** | Time to first command at c=100, median of six cohorts per arm on the merged binary. A Generation that declares no writable storage clones no private head. The clone is also the only unstable segment: it ranges 9.0 to 97.6 ms across cohorts, and removing it takes the arm's spread from **3.2x to 1.3x** |

Time to first command, p50, before and after the head durability change:

| Configuration | Before | After |
| --- | ---: | ---: |
| busybox, c=100 | 107.5 ms | **58.5 ms** |
| node:22, c=100 | 219.8 ms | **157.6 ms** |

## What does not cost what it looks like

| Thing | Measured | So |
| --- | ---: | --- |
| `FICLONE` of a 10 GiB head | 0.1 to 2.2 ms | Reflink is as cheap as the theory says; the `fsync` beside it was the cost |
| Building all five device models | about 0.013 ms | A sandbox with no network is not measurably faster. Removing it is a surface-area decision |
| Entering a network namespace | 0.06 ms | It was assumed expensive and is not |
| Applying an `nft` ruleset | 15.9 ms | Of which the spawn is 0.3 and the parse 1.2; about 14 ms is a kernel RCU grace period that netlink would not remove |
| Less guest memory | 128 MiB is **slower** than 1024 MiB at c=100, 267.3 against 118.5 | A guest too small to cache its own root faults against the immutable image, and a hundred guests doing that collide |
| Pre-faulting the whole restored memory image | Armed to launch page erased **does not move**: 5.117 ms cold against 5.102 ms pre-faulted | Full residency and eighteen times the minor faults change nothing, and the walk itself costs 57 ms. Host demand paging is not the cost |
| Huge pages for the memory image | No change at c=1, **worse** at c=100 | A `MAP_PRIVATE` mapping cannot hold a huge EPT entry, so guest faults do not move (3441 against 3396) while copy-on-write faults get about seventy percent more expensive |

## What is fixed, and why

The `ready` segment measured 28.6, 28.5, 29.1 and 29.6 ms at concurrency one across four
configurations differing in memory and workload, and is **22.60 ms** since the readiness probe was
removed. It barely moves across configurations because most of it is the cost of giving one
Instance its own cryptographic identity: the launch page, the vsock connection, the Noise
handshake, and the authenticated repair.

The largest single mechanical cost inside it is now known, and it is not any of the three things
that were assumed. Between arming the vCPU and the guest reaching its own code sat roughly 7 ms
that neither the host nor the guest clock could see. It is
[the guest taking one EPT violation per 4 KiB page it first touches](2026-08-31-restore-resume-page-in.md),
resolved by KVM inside the kernel without ever returning to userspace, which is precisely why both
instruments were blind to it: the userspace loop is idle and the guest clock does not advance.
Over the whole `armed` to `ready` window that is **3199 EPT violations, about 16.8 ms of in-kernel
exit handling against 11.7 ms of actual guest execution**.

It is not the first `KVM_RUN` entry, not restored-clock catch-up, and not a halted vCPU waiting on
a restored APIC deadline; all three were measured and excluded, with no gap over 200 µs anywhere in
the first 10 ms and three halts in total. The cost is proportional to the number of distinct guest
pages the resume touches and to nothing else, which is the one lever that would move it.

It could be very nearly removed by capturing the snapshot after a session exists instead of at the
pre-launch repair point. That is forbidden, and the reason is the product rather than the
performance: every Instance restored from that image would share one identity and one key.
[ADR 0030](../adr/0030-pre-launch-snapshot-capture-point.md) and
[ADR 0033](../adr/0033-sterile-restored-machine-authority-boundary.md) hold that line.

A competitor reporting a materially faster time to first command at the same shape is worth one
question before it is worth an optimisation: whether their sandboxes share an identity.

## What is the workload rather than the engine

`node --version` costs 27.4 ms of command time; `busybox --help` costs 3.1 ms on the same machine
shape. About twenty-four milliseconds of any `node:22` figure is a language runtime starting
itself, and no virtual machine monitor removes it.

## Method notes, learned the hard way

Eight separate times a measurement contradicted the mechanism that had been assumed: the cost of
running `nft`, the cost of entering a namespace, which receipt segment the head clone lives in,
whether less memory is faster, whether the head clone was the clone or the sync, whether host
demand paging explained the resume, whether huge pages would help it, and whether retiring the
launch page earlier would help. In every case the assumption was reasonable and wrong, and three of
them were only settled by building the change and measuring it worse than the thing it replaced.

Two changes were implemented, measured, and then reverted on the evidence: retiring the launch page
early made the segment 1.2 ms **slower**, because removing a memory slot disturbs a running guest
more than an idle one, and huge pages were slower at concurrency 100. Neither is in the tree.

Four harness lessons are worth repeating because each produced numbers that looked real. A
configuration must be launched at the shape its Generation was captured with, or every launch is
refused before a machine exists and the harness reports zeroes. A cohort of one sample is not a
distribution: the first `c=1` figures in the ladder are higher than their `c=10` neighbours because
one sample immediately after a warming launch is the worst estimator available.

A prepared entry with no captured snapshot beside it does not fail either. It cold boots, which
looks like a working measurement and reads about fifteen times slower, so a `machine_launched` to
`ready` segment in the hundreds of milliseconds means there is no `snapshot/` directory rather than
a regression: compiling a Generation and capturing one are two commands, and only the first is in
`prepare-generation.sh`.

And one hundred-way cohort per arm is not a comparison. A single pair of cohorts ranked the two
device sets the wrong way round, by catching the writable arm on its best cohort; six cohorts per
arm reversed it. Repeating is what caught it, not inspection.
