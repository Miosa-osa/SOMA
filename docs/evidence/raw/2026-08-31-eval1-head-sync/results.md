# What the private overlay head actually cost

Host `eval-1`, 80 threads, XFS with `reflink=1` at `/srv`, prepared Generations under
`/srv/soma/sweep`, heads under a scratch directory on the same filesystem. Measured 2026-08-31.

The receipt's `admitted` to `machine_launched` segment is the private head clone. A speed ladder
had it ranging from 3.7 ms to 199.5 ms for the same operation, which on a filesystem where
`FICLONE` is a constant-time metadata operation is the finding rather than the mean.

## Before

The phase split came from a temporary build that timed each step of `clone_or_copy` separately
and appended one line per launch. Every number is milliseconds.

| phase | node:22, 1024 MiB, c=1 | node:22, c=100 | busybox, 128 MiB, c=1 | busybox, c=100 |
| --- | --- | --- | --- | --- |
| open the head directory | 0.02 | 0.04 | 0.02 | 0.03 |
| create the destination | 0.06 | 0.17 | 0.06 | 0.14 |
| `FICLONE` | 0.17 | 2.24 | 0.10 | 0.19 |
| `fsync` of the head | **23.60** | **57.68** | **21.80** | **34.71** |
| `fsync` of the directory | 0.00 | 5.37 | 0.00 | 5.67 |
| size and extent verification | 0.03 | 4.31 | 0.03 | 0.85 |
| `unlink` of the name | 0.08 | 0.34 | 0.08 | 0.34 |
| whole step | 24.00 | 72.32 | 22.09 | 42.37 |

Medians, 20 launches at c=1 and 100 at c=100. The `FICLONE` itself is between 0.1 ms and 2.2 ms;
the `fsync` of the head it produced is 80 to 98 per cent of the step. The variance has the same
source: the worst single `fsync` observed at c=1 was 14 171 ms, one launch out of twenty waiting
on the XFS log behind unrelated writers.

## After

The head is unlinked before the function returns and is read only through the descriptor handed
to the machine, so neither sync has anything to make durable. Both are skipped for a head created
`Ephemeral`. Extent sharing is still proved, because the `FIEMAP` verification asks the kernel to
flush the inode itself before it maps it.

| phase | node:22, c=1 | node:22, c=100 | busybox, c=1 | busybox, c=100 |
| --- | --- | --- | --- | --- |
| open the head directory | 0.02 | 0.51 | 0.04 | 0.04 |
| create the destination | 0.05 | 1.85 | 0.05 | 0.61 |
| `FICLONE` | 0.17 | 3.68 | 0.09 | 1.01 |
| `fsync` of the head | 0.00 | 0.00 | 0.00 | 0.00 |
| `fsync` of the directory | 0.00 | 0.00 | 0.00 | 0.00 |
| size and extent verification | 0.02 | 0.01 | 0.02 | 0.01 |
| `unlink` of the name | 0.06 | 0.10 | 0.05 | 0.42 |
| whole step | 0.34 | 6.60 | 0.25 | 2.64 |

## Segment and time to first result

Read from the receipts of the same cohorts, in milliseconds.

| cohort | segment p50 before | after | segment p95 before | after | TTI p50 before | after |
| --- | --- | --- | --- | --- | --- | --- |
| node:22 1024 MiB, c=1 | 24.3 | 0.8 | 31.4 | 0.8 | 81.8 | 57.4 |
| node:22 1024 MiB, c=100 | 72.4 | 6.8 | 76.0 | 15.1 | 219.8 | 157.6 |
| busybox 128 MiB, c=1 | 22.5 | 0.7 | 146.9 | 0.8 | 52.9 | 30.9 |
| busybox 128 MiB, c=100 | 42.8 | 2.9 | 50.5 | 10.5 | 107.5 | 58.5 |

Every cohort launched and completed 100 per cent of its sandboxes before and after.
