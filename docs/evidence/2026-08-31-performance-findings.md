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
| ready | Launch page, vsock, authenticated handshake, repair, readiness probe | 29.6 | 62.6 |
| command | The workload runs | 27.4 | 77.1 |

## What was optimised, and by how much

| Change | Before | After | Mechanism |
| --- | ---: | ---: | --- |
| [Ephemeral head durability](2026-08-31-eval1-head-sync) | 72.4 ms | **6.8 ms** | The head was being `fsync`ed. It is unlinked before the clone returns and dies with its machine, so durability bought nothing |
| [Prepared machine pool](2026-08-31-prepared-machine-request-path.md) | 3.27 ms | **18.4 µs** | Machine construction moved off the request path. Two to three orders of magnitude on this host |
| Netd activation | 11.4 ms | **0.80 ms** | Four read-only questions per lifecycle were asked by running `nft`; they are now netlink queries |
| Netd release | 59.8 ms | **49.6 ms** | Same change |

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

## What is fixed, and why

The `ready` segment measured 28.6, 28.5, 29.1 and 29.6 ms at concurrency one across four
configurations differing in memory and workload. It does not move, because it is the cost of
giving one Instance its own cryptographic identity: the launch page, the vsock connection, the
Noise handshake, the authenticated repair, and the readiness probe.

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

Five separate times a measurement contradicted the mechanism that had been assumed: the cost of
running `nft`, the cost of entering a namespace, which receipt segment the head clone lives in,
whether less memory is faster, and whether the head clone was the clone or the sync. In every case
the assumption was reasonable and wrong.

Two harness lessons are worth repeating because both produced numbers that looked real. A
configuration must be launched at the shape its Generation was captured with, or every launch is
refused before a machine exists and the harness reports zeroes. And a cohort of one sample is not a
distribution: the first `c=1` figures in the ladder are higher than their `c=10` neighbours because
one sample immediately after a warming launch is the worst estimator available.
