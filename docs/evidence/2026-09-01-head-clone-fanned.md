# Removing both serialized objects from the head clone - 2026-09-01

[Why the writable head clone swings](2026-08-31-head-clone-serialization.md) proved that the
private overlay head clone is serialized rather than slow, that a cohort of one hundred pays one
hundred times one clone, and that there are two serialized objects which behave as an AND gate:
the head directory and the refcount records of the template's extents. It changed no production
code. This record is the fix, measured end to end through the launcher rather than through the
probe.

## What was built

Two modules in `crates/soma-storage`, and eight lines of wiring in the KVM backend's boot path.

`shard` spreads head creation over `h00` upwards inside the head root. A launch is usually its
own process, so a counter starting at zero in every process would put every launch on `h00`;
the starting point is therefore derived once per process from its identity and the clock, and
the counter only spreads launches within a process.

`fan` hands a launch one of several independent physical copies of the overlay template.
Independent means physically independent: the earlier record showed that giving every thread its
own source inode made contention six times worse, because the copies still shared one set of
extents. A fan is therefore written by moving bytes through user space, never with
`copy_file_range`, which XFS may serve by sharing extents, and every copy is proved before it is
published: the same length as the template, the same SHA-256, and zero shared extents of its own.

Warming a fan is a prepare-time operation, not a launch-time one. `fan_warm` creates the shards
and writes the copies; a head root that was never warmed still launches, because the launch path
falls back to cloning the template itself and pays exactly what it paid before. Nothing on the
launch path writes, hashes, or copies.

## The alternative that was weighed first

The obvious competitor is to take the clone off the launch path entirely by handing out heads
cloned in advance, which the read-only arm's 0.3 ms segment shows the ceiling of. It was not
taken, for four reasons, and the fourth is the decisive one.

It does not remove the serialization; it moves it in time. Refilling a pool is N clones from one
template into one directory, which is the same two exclusive sections at the same price. A pool
only helps while it is deeper than the burst, and the shape SOMA is compared on is a cold burst
of one hundred, so the pool has to cover a whole burst and refill between bursts.

It gives up a property the current design has. A head today exists on the filesystem for no time
at all: it is created and unlinked inside one function, and the durable machine host record could
observe an empty head directory after every run. A prepared head has to be named on disk, owned,
reconciled after a crash, and collected. `soma-storage` already has `lease`, `release`, and
`reconcile` designed for exactly that, so this is buildable, but it is a lifecycle with failure
modes rather than a syscall change.

It is not simpler. It needs a refill policy, a depth, an owner, and a reconciler, against two
self-contained modules and one prepare step.

And it needs this work anyway. A pool that refills at burst rate refills through the same
serialized sections, so the fan is a precondition for a fast pool rather than an alternative to
it. Prepared heads remain the right next step if the residual clone cost still matters, and they
will be cheaper to build on a sharded, fanned storage layer than on today's one.

## Evidence boundary

One host, eval-1, the same one the earlier record used: Ubuntu 24.04, kernel 6.8.0-138, 80
cores, `/srv` on XFS with `reflink=1`, 32 allocation groups, over an LVM volume on a four spindle
md RAID10 of 10K SAS disks. The backing store is rotational. Every figure is busybox at one vCPU,
1024 MiB of memory and a 2048 MiB overlay, restoring a 1 GiB memory snapshot, which is the
production shape the earlier record measured.

It proves nothing about a flash backed host, another filesystem, a workload that writes to its
overlay, or any concurrency other than one hundred.

## The three arms

Each arm has its own head root, so no arm inherits another's directories, and the arms differ
only in the shard count and whether a warmed fan is present.

| arm | head directories | template copies |
| --- | ---: | ---: |
| `base` | 1 | 1, no fan |
| `shard` | 16 | 1, no fan |
| `fan` | 16 | 4 |

Cohorts were taken one at a time round robin across the three arms, so host drift hits all three
equally, with each round gated on a direct `/proc/stat` busy sample taken while this run was
idle. The one minute load average is recorded beside every cohort but gates nothing, because a
cohort of one hundred sandboxes raises it by itself.

## PLACEHOLDER RESULTS

## The mechanism, directly observed

The four copies are one 2 GiB extent each at four disjoint physical block ranges, and none of
them carries the shared flag:

```text
PLACEHOLDER FIEMAP
```

One launch traced with `strace` opens a shard of the head root and one copy of the fan, rather
than the head root itself and the template:

```text
PLACEHOLDER STRACE
```

## What this does not fix

PLACEHOLDER GAP

## Raw

[`raw/2026-09-01-head-clone-fanned/`](raw/2026-09-01-head-clone-fanned/), with `harness/`
holding every script used.
