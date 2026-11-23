# Why the writable head clone swings, and by how much - 2026-08-31

The merged binary device set record found that the private overlay head clone is the only
unstable segment of a writable launch at concurrency, with cohort medians spread 3.2x. This
record identifies what the segment is waiting on, measures the spread properly, and tests the
mechanism by manipulating it.

## Evidence boundary

Everything here is one host, eval-1: Ubuntu 24.04, kernel 6.8.0-138, 80 cores, 156 GiB of
memory, `/srv` on XFS with `reflink=1`, `rmapbt=1`, 32 allocation groups and a 785 MiB internal
log, over an LVM volume on a four spindle md RAID10 of 10K SAS disks. **The backing store is
rotational, not flash.** Every figure is busybox at one vCPU and 1024 MiB, the shape the engine
is compared against.

The host is shared with other agents. Every cohort in this record was preceded by a direct
`/proc/stat` busy sample taken while this run was idle, the runs refused to start a round above
12 percent busy, and every raw record carries the busy figure and the load average it was taken
under. The one minute load average is not usable as a gate here: a cohort of one hundred
sandboxes raises it by itself, so a run gated on it stalls on its own last sample. No cohort in
the headline table was discarded, and the highest busy figure recorded on either side of
any of the eighty cohorts was 1.99 percent.

Earlier measurements taken this evening while three other agents were building on this host are
not in this record. They agreed with what follows, but they are not evidence.

It proves nothing about a flash backed host, about another filesystem, about a workload that
writes to its overlay, or about any concurrency other than the ones stated.

## What the segment is

`admitted -> machine_launched` is the private overlay head clone in
`crates/soma-local/src/backend/kvm/boot.rs`: an exclusive create under the head directory
descriptor, a `FICLONE` of the Generation's sterile overlay template, a `FIEMAP` walk proving
every extent is shared, and an unlink. The template is one 2 GiB file holding **a single
extent**, fully allocated, so template fragmentation cannot be involved.

## The observation, re-measured on an idle host

Forty cohorts of each arm, alternating, one hundred sandboxes each, eight thousand launches,
all successful, none discarded.

| arm | cohort median range | spread | median | launch p50 | p95 | p99 | max |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| writable | 27.4 to **270.8** ms | **9.88x** | 41.4 ms | 41.2 | 139.5 | **276.6** | 292.6 |
| read-only | 23.7 to 38.4 ms | 1.62x | 30.3 ms | 30.3 | 48.0 | 53.8 | 59.8 |

The clone segment alone, over the same launches:

| arm | cohort p50 range | launch p50 | p95 | p99 | max |
| --- | --- | ---: | ---: | ---: | ---: |
| writable | 0.47 to **234.59** ms | 7.0 | 102.6 | **237.6** | 250.5 |
| read-only | 0.24 to 0.27 ms | 0.3 | 0.4 | 1.4 | 10.4 |

**133 ms was not the tail.** A cohort median of 270.8 ms and a launch p99 of 276.6 ms were
reached inside forty cohorts, and the spread of cohort medians is 9.88x rather than 3.2x. Six
cohorts per arm understated both. The distribution is bimodal rather than heavy tailed: thirty
of forty writable cohorts sit between 27.4 and 44.1 ms, and ten sit between 53.4 and 270.8 ms.

The read-only arm's clone segment is 0.24 to 0.27 ms in **every one of forty cohorts**,
including the cohorts run immediately after the worst writable ones. Question four is answered
directly: the read-only path is not affected, and nothing observed here moves it.

## What the threads are waiting on

`crates/soma-storage/examples/head_probe` performs the same four syscalls a launch performs,
one hundred threads released through a barrier, with each phase timed on its own. Kernel stacks
were dumped whenever most of its threads were blocked. The picture is the same on a loaded host
and on an idle one:

```
COUNT 99
[<0>] xfs_ilock2_io_mmap+0xe6/0x390 [xfs]
[<0>] xfs_reflink_remap_prep+0x55/0x2a0 [xfs]
[<0>] xfs_file_remap_range+0x89/0x360 [xfs]
[<0>] vfs_clone_file_range+0x110/0x360
[<0>] ioctl_file_clone+0x52/0xc0
```

and the one thread that is not queued:

```
COUNT 1
[<0>] xfs_buf_lock ... xfs_read_agf ... xfs_refcount_finish_one
[<0>] xfs_defer_finish_noroll ... xfs_trans_commit
[<0>] xfs_reflink_remap_extent+0x325/0x5f0 [xfs]
```

Ninety nine threads are queued and one is updating the allocation group's refcount btree. The
clone is not slow. It is **serialized**, and a cohort pays one hundred times one clone.

`xs_sleep_logspace` in `/proc/fs/xfs/stat` is **zero since boot**, so log grant waits are
excluded. Device read counters across one hundred and fifty back to back probe cohorts are
**zero**, so the serialized section is not reading from the spindles in steady state.

## The contended object is the extent, not the inode

Twenty cohorts of each arm, interleaved. All three arms clone into one head directory; only the
source changes.

| source | inodes | physical extents | clone p50 | clone p99 |
| --- | ---: | ---: | ---: | ---: |
| one shared template | 1 | 1 | 1132.8 us | 2554.2 |
| one private reflink per thread | 100 | 1 | **7244.2 us** | 10894.1 |
| four independent copies | 4 | 4 | **97.3 us** | 1088.4 |

Giving every thread its own source **inode** makes it six times worse. Giving them four
independent **physical extents** makes it twelve times better. The queue in
`xfs_ilock2_io_mmap` is only where threads pile up while the inode is also shared; remove the
inode and they pile up deeper on the allocation group instead. What is actually serialized is
the update of the single refcount btree record covering the template's one extent.

## Two contended objects, crossed

Twenty five cohorts of each arm, one cohort at a time round robin across the four arms so host
drift hits all four equally. `t1` is one template, `t4` is four independent copies, `d1` is one
head directory, `d16` is sixteen. `t1-d1` is the production shape.

| arm | create | clone | verify | unlink | cohort wall | worst wall | per clone |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| t1-d1 | 1796.5 us | 3060.2 | 12.3 | 1038.3 | 23.47 ms | 27.85 | 234.7 us |
| t4-d1 | 1969.0 | **95.9** | 12.3 | 2027.1 | 21.81 | 24.79 | 218.1 |
| t1-d16 | **50.0** | 4614.0 | 11.2 | **19.4** | 22.08 | 27.55 | 220.8 |
| t4-d16 | **56.3** | 1117.7 | 11.8 | **21.5** | **15.93** | **17.65** | **159.3** |

There are two serialized objects, not one, and they behave as an AND gate.

- Fanning the template out moves the clone from 3060 to 96 us, a factor of 32.
- Fanning the directory out moves create from 1797 to 50 us and unlink from 1038 to 19 us.
- **Neither alone changes throughput.** Per clone cost goes 234.7 to 218.1 or 220.8, about six
  percent. The cost does not disappear; it relocates to whichever object is still shared. In
  `t1-d16` the clone rises to 4614 us precisely because the directory is no longer throttling
  arrivals at the template.
- Only removing **both** helps: 234.7 to 159.3 us per clone, a 32 percent throughput gain, with
  the worst cohort falling from 27.85 to 17.65 ms.

The `FIEMAP` verification is 11 to 12 us in every arm. It is not a cost and is not implicated.

## Queue depth is what makes it unstable

Concurrencies interleaved within each round, eighteen rounds. Each arm's first cohort is a cold
start and is reported separately rather than averaged in: it was 340.9 ms at concurrency ten.

| concurrency | clone segment p50 | range | spread | cohort median spread |
| ---: | ---: | --- | ---: | ---: |
| 10 | 0.77 ms | 0.56 to 1.00 | 1.8x | 1.15x |
| 25 | 0.72 ms | 0.40 to 1.32 | 3.3x | 1.24x |
| 100 | 5.65 ms | 0.54 to 9.39 | **17.4x** | 1.56x |

At ten and at twenty five the clone segment is about one millisecond and steady. Only at one
hundred does it swing, and in the forty cohort run above it reached 234.59 ms. The instability
is a property of the queue depth, not of the clone.

## What raises the service time

Fifteen cohorts of each arm, warm and cold interleaved, cold meaning `sync` then
`drop_caches`.

| arm | warm wall | cold wall | ratio |
| --- | ---: | ---: | ---: |
| t1-d1 | 24.49 ms | 84.56 ms | 3.5x |
| t4-d16 | 17.84 ms | 131.59 ms | 7.4x |

Cold metadata multiplies the serialized section by several times, and every queued launch pays
it. The fanned out shape degrades more in absolute terms only because it has four templates and
sixteen directories to fault back in rather than one and one.

## The honest gap

The serialization is proven and the amplification is proven. What is **not** identified is what
puts a particular cohort into the slow mode on an idle host with no disk reads. One hundred and
fifty back to back probe cohorts spread only 1.98x, from 24.97 to 49.32 ms, while the real
launcher spread 9.88x over forty. The difference between them is the rest of the cohort: one
hundred machines restoring a 1 GiB memory snapshot each, which the probe does not do. The
working hypothesis is that the cohort's own machines perturb the service time of the section
their siblings are queued on, but that is not demonstrated here and should not be reported as
though it were.

## What this means for a fix

This is not inherent to reflinking. It is two per object exclusive sections in the kernel, and
both are removable, but a fix must remove **both** or it buys nothing, and removing only the
directory made the measured tail worse rather than better.

1. Shard the head directory. Small, and sits behind the existing `SOMA_HEAD_DIR` seam.
2. Fan the overlay template out into several independent physical copies per Generation. This
   is a Generation artifact change and costs one template's bytes per copy, 2 GiB each here.

Measured together, in the probe: 234.7 to 159.3 us per clone and worst cohort 27.85 to 17.65
ms. Neither was measured end to end in the launcher, because neither can be reached through an
existing seam, and this record deliberately changed no production code.

The alternative worth weighing against them is to take the clone off the launch path entirely,
by handing out heads cloned in advance. The read-only arm already shows what that ceiling looks
like: a 0.3 ms segment that did not move across forty cohorts.

## Raw

[`raw/2026-08-31-head-clone-serialization/`](raw/2026-08-31-head-clone-serialization/), with `harness/` holding every script
used and `stacks/` holding the kernel stacks quoted above. The probe is
`crates/soma-storage/examples/head_probe`.
