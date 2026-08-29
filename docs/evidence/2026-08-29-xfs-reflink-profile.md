# XFS reflink clone-latency matrix and the prepared-head decision - 2026-08-29

## Evidence boundary

This result proves that, on a real Ubuntu 24.04 x86_64 host, `crates/soma-storage` can prove an XFS `reflink=1` mount with a tiny `FICLONE`, reject a `reflink=0` mount without any copy fallback, build byte-identical sterile ext4 templates from one pinned `mke2fs` recipe, create private heads with `FICLONE` under a capability directory descriptor and hand back only an open descriptor, prove that every reported head extent is shared, prove that two clones diverge without touching the template or each other, prove that exhausting a clone reports `ENOSPC` while the template digest survives, and keep a directory reconcilable through 32-way concurrent create and cleanup.
It then measures the complete cost of one on-demand head across 69 matrix cells with 200 raw samples each and zero failures, and derives the decision that decision-map ticket #11 requires.

It does not prove production-host latency.
The filesystem is a loop device over a sparse image file on the host's ext4 NVMe root, so every `fsync` crosses two filesystems and the loop driver, and the host was a busy development machine running twelve unrelated containers.
It proves nothing about a raw partition, a certified host class, quota exhaustion, unsupported mount options, or a crash during clone or deletion.
The numbers decide between on-demand and prepared heads; they are not a per-Instance disk-cost claim for any production profile.

## Decision

On-demand cloning is not admitted.
The disk share of the fast-path budget is fresh resource activation below 1.00 ms at p99.
The best 100-way `FICLONE` cell (`100m-sterile-warm-c100-none-ficlone`) has a complete-clone p99 of 9.9 ms and an `ioctl`-only p99 of 7.0 ms; the worst (`4g-frag-warm-c100-none-ficlone`) has 1,868 ms.
No cell at any concurrency fits the budget: the best single-clone cell (`1g-sterile-warm-c1-none-ficlone`) has a complete-clone p99 of 1.25 ms because the durable file `fsync` alone is 0.6 ms at p50 and 1.1 ms at p99.
Prepared sterile heads are therefore mandatory: the host allocator must create, sync, and verify heads outside Launch from sterile templates, keep one prepared pool per size class, and keep head destruction off the request path as well, because a concurrent unlink of 100 heads raised the 100-way complete-clone p99 from 21.5 ms to 57.1 ms.
The mechanism the allocator will call is exactly the measured `clone::clone_head` path; only its placement changes.

## Identities

- SOMA Git revision measured: `3ff64e6db819b1a19aba38c21f5d0b2defbec256`, which is the branch after rebasing onto `origin/main` at `9f3a656`; the `crates/soma-storage` tree object at that revision is `5d886fa2d460906d2651c58d1320570f94f96c7e`, which identifies the measured code independently of later history rewrites, and the branch was rebased again onto `origin/main` at `3d848cd` before review with that tree unchanged.
- Host: Ubuntu 24.04.4 LTS, `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC` x86_64.
- CPU: Intel Core Ultra 9 275HX, 24 logical CPUs, microcode `0x11b`; 62 GiB RAM.
- Host storage: SK hynix HFS001TEJ9X115N NVMe, root filesystem ext4, which has no reflink support and no `xfsprogs`.
- Rust toolchain: `1.98.0 (88d9e12ae 2026-08-18)`, release profile; Docker 29.3.0.
- Measurement image: `scripts/xfs-reflink-bench.Dockerfile` from `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`, built as `sha256:55a9ce9bd26ef87ad6bad440c5ef7618e2198e86936838367607d96174dc8659`, with `xfsprogs 6.6.0-1ubuntu2.1`, `e2fsprogs 1.47.0-2.4~exp1ubuntu4.1`, `util-linux 2.39.3-9ubuntu6.5`, and `coreutils 9.4-3ubuntu6.2`.
- Head filesystem: `/dev/loop29` with `--direct-io=on` and a 512-byte logical sector over the 20 GiB sparse file `soma-xfs-reflink.img`, formatted `mkfs.xfs -m reflink=1` with `agcount=4`, `bsize=4096`, `crc=1`, `finobt=1`, `sparse=1`, `rmapbt=1`, `reflink=1`, `bigtime=1`, `inobtcount=1`, `nrext64=0`, and a 16,384-block internal log; mounted `rw,noatime` with superblock options `rw,inode64,logbufs=8,logbsize=32k,noquota`; 20,953,145,344 free bytes at the start of the matrix.
- Negative filesystems: a 320 MiB `reflink=1` image for the `ENOSPC` proof and a 320 MiB `reflink=0` image for the rejection proof, attached the same way.
- Run window: host script started 2026-08-29T12:19:01Z, matrix identity record at Unix time 1788005946, container finished 2026-08-29T12:22:26Z.

## Invocation

```sh
SOMA_XFS_SCRATCH=/path/to/scratch scripts/xfs-reflink-bench.sh all
```

The host script builds `soma-storage-bench` and the test executables with `cargo build --locked --release`, creates the three sparse images, builds the pinned image, and runs `scripts/xfs-reflink-container.sh` as root inside `docker run --rm --privileged` with the repository mounted read-only.
The container attaches and formats the loop devices, runs the ignored live tests with `SOMA_XFS_REFLINK_DIR`, `SOMA_XFS_TEMPLATE_DIR`, `SOMA_XFS_TINY_DIR`, and `SOMA_XFS_NOREFLINK_DIR` set, unmounts and remounts the head filesystem and compares the template digests, and then runs:

```sh
soma-storage-bench --dir /mnt/soma/reflink --out /scratch/raw/xfs-reflink-samples.jsonl
```

with the default 200 samples per cell.
Without the environment variables every live test fails with `prerequisite missing`; none of them passes silently.

## Measured boundary

`total` is the complete cost of one head as the allocator would pay it: `openat` with `O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW` under the head directory descriptor, the `FICLONE` `ioctl`, `fsync` of the head, `fsync` of the directory, `fstat` of template and head, and a `FIEMAP` walk with `FIEMAP_FLAG_SYNC` that requires every reported extent to carry `FIEMAP_EXTENT_SHARED`.
`FICLONE` is the `ioctl` alone.
For the `cp` comparison the `FICLONE` column is the complete `cp --reflink=always` child process from spawn to exit, `create` is zero, and the same syncs and verification follow.
Every duration is a monotonic wall-clock measurement in nanoseconds inside the thread that performed the work.

A cell with concurrency `c` runs `200 / c` bursts; every burst releases `c` threads through one barrier and each thread creates one head, so a 100-way cell is two bursts of 100 simultaneous clones from one template inode.
Heads are unlinked between bursts outside the timer.
Cold cells run `sync` and write `3` to `/proc/sys/vm/drop_caches` before every burst.
The free-space cells first `fallocate` filler files until 10 percent of the filesystem is free and remove them afterwards.
The cleanup cells precreate 100 additional 1 GiB heads before each burst and release 100 unlinking threads through the same barrier; each unlink plus directory `fsync` is recorded as its own sample.
Percentiles are nearest-rank over the raw samples, so p99 of 200 samples is the 198th smallest.

## Live conformance results

```text
test clone_shares_every_extent_and_isolation_holds_across_two_clones ... ok
test concurrent_create_and_cleanup_leave_a_clean_directory ... ok
test profile_probe_accepts_the_reflink_mount ... ok
test profile_probe_rejects_the_reflink_disabled_mount_and_so_does_clone ... ok
test writing_through_a_clone_reports_enospc_and_leaves_the_template_intact ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.07s

test template::tests::two_templates_from_one_recipe_are_byte_identical ... ok
```

After `umount` and `mount` of the head filesystem the two live templates read back with the same SHA-256 values they had before, `5700671a...ea5054` for `live-iso-v1.ext4` and `f8fa57e7...50fe43` for `live-burst-v1.ext4`.
The `reflink=0` mount was rejected by `StorageProfile::probe` with `ReflinkUnsupported`, the probe files were unlinked, `clone_head` on that mount returned `ReflinkUnsupported`, and the failed destination did not exist afterwards.
The isolation proof wrote different patterns at the first, an early metadata, the middle, and the last 4 KiB region of two clones, forced allocation with `fdatasync`, read the template and each peer back unchanged, and observed the written extents lose `FIEMAP_EXTENT_SHARED`.

## Templates

Every template is `mke2fs -F -q -t ext4 -b 4096 -I 256 -i 16384 -m 0 -r 1 -e remount-ro -O none,has_journal,ext_attr,resize_inode,dir_index,filetype,extent,flex_bg,sparse_super,large_file,huge_file,dir_nlink,extra_isize,metadata_csum,64bit -E hash_seed=...,lazy_itable_init=0,lazy_journal_init=0,root_owner=0:0,nodiscard` with a private `mke2fs.conf`, a derived `-U`, `E2FSPROGS_FAKE_TIME=1787961600`, `LC_ALL=C`, and a closed `PATH`, then `e2fsck -fn`.
`prealloc` adds one `fallocate` over the whole file and `frag` adds one 4 KiB `fallocate` every 128 KiB; unwritten extents read as zero, so the three templates of one size carry the same filesystem structure and differ only through the UUID, hash seed, and label that each class name derives.
Creation time is outside every timed sample.

| Class | Bytes | Extents | Shared | Unwritten | Mapped bytes | Creation ms | SHA-256 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `bench-100m-sterile` | 104,857,600 | 10 | 0 | 5 | 5,988,352 | 54 | `5b9036f1eb4c4dd6da27b26e971641076bdb63276937f42bf7ed3738867a6418` |
| `bench-100m-prealloc` | 104,857,600 | 11 | 0 | 6 | 104,857,600 | 64 | `73757fbc3a63c10980ca63be311e7e61a67cbddd2431375e813230084726d405` |
| `bench-100m-frag` | 104,857,600 | 764 | 0 | 759 | 9,076,736 | 65 | `8f4d2043312ed7f6a17b1676d3b0dbac9fb695f994e3f4edbd1afc27e4c845d9` |
| `bench-1g-sterile` | 1,073,741,824 | 14 | 0 | 3 | 50,999,296 | 685 | `d4708abd06e9459902ce98cab65b336ab24e773668d17b3223d5fed8d68ff297` |
| `bench-1g-prealloc` | 1,073,741,824 | 23 | 0 | 12 | 1,073,741,824 | 527 | `5c85da85fd295660c7a81b668c19611a5d7a66b9cafc0de8ad4fe49cb575957c` |
| `bench-1g-frag` | 1,073,741,824 | 7,813 | 0 | 7,802 | 82,944,000 | 496 | `e15338425d31d8214d63add3e44826e3a550dc0a7d7bab8580df0884f796a921` |
| `bench-4g-sterile` | 4,294,967,296 | 19 | 0 | 4 | 136,486,912 | 1617 | `f98cb4239a2d4d8934faa96bef17d0ce81acca5e8e1c306ec596237d4d9d6e52` |
| `bench-4g-prealloc` | 4,294,967,296 | 33 | 0 | 18 | 4,294,967,296 | 1787 | `02bbaf259627ecc98025349876e3f8cb05785f725f47e9f44b14ffe975956376` |
| `bench-4g-frag` | 4,294,967,296 | 31,738 | 0 | 31,723 | 266,407,936 | 2026 | `a57e84a49f1295d7feb7b555be5f6b0d3b38fbddf0883c1f83a474510bc6b324` |

## Results

Every cell has n = 200 successful samples and 0 failures. Values are microseconds; `total` is the complete clone cost and `FICLONE` is the `ioctl` alone, or the `cp` child process in the comparison table.

### In-process FICLONE, 100 MiB template, no extra pressure

| Cell | n | failed | total p50 us | total p95 us | total p99 us | total max us | FICLONE p50 us | FICLONE p99 us |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `100m-sterile-warm-c1-none-ficlone` | 200 | 0 | 674.2 | 971.3 | 1410.1 | 2004.1 | 50.8 | 112.5 |
| `100m-sterile-warm-c10-none-ficlone` | 200 | 0 | 1783.6 | 3088.8 | 3424.8 | 3513.1 | 222.8 | 803.6 |
| `100m-sterile-warm-c100-none-ficlone` | 200 | 0 | 6564.1 | 9925.5 | 9947.3 | 9952.0 | 3753.3 | 6979.5 |
| `100m-sterile-cold-c1-none-ficlone` | 200 | 0 | 2453.7 | 3888.7 | 4217.9 | 4540.3 | 539.7 | 1786.1 |
| `100m-sterile-cold-c10-none-ficlone` | 200 | 0 | 3465.7 | 5080.5 | 6285.0 | 6426.2 | 596.3 | 2184.8 |
| `100m-sterile-cold-c100-none-ficlone` | 200 | 0 | 8935.7 | 12464.3 | 13496.2 | 13508.1 | 3540.8 | 7006.5 |
| `100m-prealloc-warm-c1-none-ficlone` | 200 | 0 | 666.0 | 944.5 | 1501.0 | 7116.8 | 54.2 | 121.0 |
| `100m-prealloc-warm-c10-none-ficlone` | 200 | 0 | 1574.6 | 2833.6 | 3425.3 | 3507.6 | 252.2 | 986.5 |
| `100m-prealloc-warm-c100-none-ficlone` | 200 | 0 | 7425.3 | 12903.9 | 13763.2 | 13830.9 | 5578.3 | 10801.1 |
| `100m-prealloc-cold-c1-none-ficlone` | 200 | 0 | 2237.1 | 4121.3 | 4622.8 | 5052.5 | 411.3 | 1946.9 |
| `100m-prealloc-cold-c10-none-ficlone` | 200 | 0 | 3370.6 | 5130.9 | 5774.6 | 6016.0 | 487.6 | 1716.6 |
| `100m-prealloc-cold-c100-none-ficlone` | 200 | 0 | 7234.0 | 10631.7 | 11753.1 | 11789.2 | 4254.2 | 7170.0 |
| `100m-frag-warm-c1-none-ficlone` | 200 | 0 | 1130.8 | 1538.9 | 1723.9 | 2438.1 | 479.5 | 683.7 |
| `100m-frag-warm-c10-none-ficlone` | 200 | 0 | 3856.3 | 6091.7 | 6561.8 | 6703.8 | 2755.6 | 5309.6 |
| `100m-frag-warm-c100-none-ficlone` | 200 | 0 | 28065.0 | 53634.8 | 55856.6 | 56904.9 | 27063.5 | 54184.3 |
| `100m-frag-cold-c1-none-ficlone` | 200 | 0 | 3046.5 | 4174.0 | 4670.1 | 4803.0 | 1059.6 | 1942.7 |
| `100m-frag-cold-c10-none-ficlone` | 200 | 0 | 5392.6 | 8242.1 | 9162.2 | 9421.2 | 3046.7 | 6100.4 |
| `100m-frag-cold-c100-none-ficlone` | 200 | 0 | 29008.2 | 54766.4 | 57322.3 | 57677.4 | 26958.8 | 52657.2 |

### In-process FICLONE, 1 GiB template, no extra pressure

| Cell | n | failed | total p50 us | total p95 us | total p99 us | total max us | FICLONE p50 us | FICLONE p99 us |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `1g-sterile-warm-c1-none-ficlone` | 200 | 0 | 748.3 | 1093.6 | 1246.4 | 2221.4 | 98.9 | 158.1 |
| `1g-sterile-warm-c10-none-ficlone` | 200 | 0 | 2524.9 | 3950.4 | 5578.3 | 6091.7 | 565.2 | 2237.7 |
| `1g-sterile-warm-c100-none-ficlone` | 200 | 0 | 11678.0 | 20860.8 | 21467.1 | 21836.4 | 9403.6 | 18703.7 |
| `1g-sterile-cold-c1-none-ficlone` | 200 | 0 | 2557.5 | 3743.2 | 4115.6 | 4597.2 | 655.0 | 1463.7 |
| `1g-sterile-cold-c10-none-ficlone` | 200 | 0 | 3689.4 | 6667.5 | 12898.9 | 12963.8 | 1035.9 | 2675.0 |
| `1g-sterile-cold-c100-none-ficlone` | 200 | 0 | 14471.4 | 23927.3 | 25057.2 | 25142.2 | 9095.5 | 20050.1 |
| `1g-prealloc-warm-c1-none-ficlone` | 200 | 0 | 745.2 | 1142.3 | 1384.7 | 2666.1 | 102.1 | 244.6 |
| `1g-prealloc-warm-c10-none-ficlone` | 200 | 0 | 2321.9 | 3167.2 | 3719.1 | 3993.0 | 561.2 | 1873.9 |
| `1g-prealloc-warm-c100-none-ficlone` | 200 | 0 | 12902.4 | 22898.2 | 23270.7 | 23375.8 | 10554.9 | 20287.7 |
| `1g-prealloc-cold-c1-none-ficlone` | 200 | 0 | 2717.1 | 3690.2 | 4464.2 | 4740.5 | 720.6 | 1598.5 |
| `1g-prealloc-cold-c10-none-ficlone` | 200 | 0 | 3561.4 | 4770.6 | 7482.5 | 7723.4 | 823.3 | 1709.4 |
| `1g-prealloc-cold-c100-none-ficlone` | 200 | 0 | 14726.4 | 25704.2 | 26512.2 | 26950.2 | 10444.7 | 21431.5 |
| `1g-frag-warm-c1-none-ficlone` | 200 | 0 | 5681.3 | 6731.1 | 7347.6 | 9191.0 | 4924.8 | 6303.5 |
| `1g-frag-warm-c10-none-ficlone` | 200 | 0 | 27392.8 | 50153.2 | 51062.4 | 52499.9 | 26207.9 | 50193.7 |
| `1g-frag-warm-c100-none-ficlone` | 200 | 0 | 245494.9 | 469917.8 | 490070.4 | 494837.6 | 244114.0 | 488238.3 |
| `1g-frag-cold-c1-none-ficlone` | 200 | 0 | 7409.1 | 8835.1 | 9184.5 | 9488.6 | 5332.3 | 6535.7 |
| `1g-frag-cold-c10-none-ficlone` | 200 | 0 | 28185.6 | 51263.0 | 52727.0 | 60763.6 | 25866.9 | 50622.1 |
| `1g-frag-cold-c100-none-ficlone` | 200 | 0 | 248348.6 | 469776.1 | 489817.8 | 499783.2 | 246292.3 | 485098.9 |

### In-process FICLONE, 4 GiB template, no extra pressure

| Cell | n | failed | total p50 us | total p95 us | total p99 us | total max us | FICLONE p50 us | FICLONE p99 us |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `4g-sterile-warm-c1-none-ficlone` | 200 | 0 | 716.6 | 1090.6 | 1440.5 | 3524.5 | 135.1 | 275.2 |
| `4g-sterile-warm-c10-none-ficlone` | 200 | 0 | 2483.3 | 3807.2 | 4119.2 | 4805.9 | 916.5 | 2901.7 |
| `4g-sterile-warm-c100-none-ficlone` | 200 | 0 | 17769.3 | 31902.1 | 34609.1 | 35446.9 | 15696.1 | 31784.3 |
| `4g-sterile-cold-c1-none-ficlone` | 200 | 0 | 2873.4 | 4249.6 | 4724.2 | 5332.0 | 967.7 | 2050.3 |
| `4g-sterile-cold-c10-none-ficlone` | 200 | 0 | 4590.9 | 7184.4 | 8334.2 | 8465.7 | 1268.1 | 2923.9 |
| `4g-sterile-cold-c100-none-ficlone` | 200 | 0 | 17353.7 | 28067.9 | 29857.6 | 30244.8 | 13505.5 | 24753.2 |
| `4g-prealloc-warm-c1-none-ficlone` | 200 | 0 | 760.7 | 1198.9 | 1644.2 | 1992.1 | 138.1 | 376.2 |
| `4g-prealloc-warm-c10-none-ficlone` | 200 | 0 | 2788.9 | 4531.5 | 5367.7 | 6233.5 | 939.2 | 4355.6 |
| `4g-prealloc-warm-c100-none-ficlone` | 200 | 0 | 18090.0 | 29771.2 | 31187.7 | 31570.5 | 15492.2 | 29344.8 |
| `4g-prealloc-cold-c1-none-ficlone` | 200 | 0 | 4827.9 | 22711.8 | 26033.5 | 26712.3 | 1082.2 | 2851.6 |
| `4g-prealloc-cold-c10-none-ficlone` | 200 | 0 | 15891.5 | 36488.0 | 38426.0 | 38565.5 | 1704.9 | 18174.8 |
| `4g-prealloc-cold-c100-none-ficlone` | 200 | 0 | 50205.4 | 71559.5 | 71585.1 | 71596.1 | 19379.5 | 55369.1 |
| `4g-frag-warm-c1-none-ficlone` | 200 | 0 | 21811.6 | 23378.2 | 24198.8 | 30270.8 | 17816.6 | 20584.7 |
| `4g-frag-warm-c10-none-ficlone` | 200 | 0 | 115405.1 | 201052.2 | 215003.4 | 236316.0 | 111915.8 | 211179.1 |
| `4g-frag-warm-c100-none-ficlone` | 200 | 0 | 925566.6 | 1779441.8 | 1867843.8 | 1903654.9 | 922223.1 | 1861428.0 |
| `4g-frag-cold-c1-none-ficlone` | 200 | 0 | 27950.8 | 30033.8 | 39220.5 | 63097.8 | 19709.5 | 25444.7 |
| `4g-frag-cold-c10-none-ficlone` | 200 | 0 | 112722.9 | 193410.9 | 205142.3 | 214076.4 | 102937.3 | 197767.8 |
| `4g-frag-cold-c100-none-ficlone` | 200 | 0 | 941944.7 | 1777286.5 | 1865725.3 | 1917622.3 | 936275.8 | 1854700.6 |

### In-process FICLONE, 1 GiB, 100-way, under free-space and cleanup pressure

| Cell | n | failed | total p50 us | total p95 us | total p99 us | total max us | FICLONE p50 us | FICLONE p99 us |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `1g-sterile-warm-c100-freespace-ficlone` | 200 | 0 | 30353.8 | 37045.7 | 37173.7 | 37265.2 | 8312.9 | 13540.2 |
| `1g-sterile-warm-c100-cleanup-ficlone` | 200 | 0 | 39564.4 | 55940.4 | 57060.3 | 57344.7 | 21823.7 | 30475.4 |
| `1g-prealloc-warm-c100-freespace-ficlone` | 200 | 0 | 37823.2 | 39319.2 | 40553.4 | 40560.5 | 7734.4 | 27086.3 |
| `1g-prealloc-warm-c100-cleanup-ficlone` | 200 | 0 | 50279.4 | 64209.7 | 64392.0 | 64565.6 | 32384.7 | 40862.5 |
| `1g-frag-warm-c100-freespace-ficlone` | 200 | 0 | 252972.0 | 491624.2 | 505424.6 | 512444.2 | 249668.5 | 494504.7 |
| `1g-frag-warm-c100-cleanup-ficlone` | 200 | 0 | 256746.4 | 477376.4 | 506721.4 | 512877.3 | 253490.0 | 488947.5 |

### cp --reflink=always subprocess per head, 1 GiB, warm

| Cell | n | failed | total p50 us | total p95 us | total p99 us | total max us | FICLONE p50 us | FICLONE p99 us |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `1g-sterile-warm-c1-none-cp` | 200 | 0 | 3844.2 | 5622.5 | 7169.1 | 12330.7 | 591.3 | 912.1 |
| `1g-sterile-warm-c10-none-cp` | 200 | 0 | 13587.2 | 14224.9 | 14811.7 | 17143.9 | 1187.8 | 2664.2 |
| `1g-sterile-warm-c100-none-cp` | 200 | 0 | 32270.7 | 49278.0 | 49426.0 | 49447.5 | 11936.4 | 31816.8 |
| `1g-prealloc-warm-c1-none-cp` | 200 | 0 | 3856.0 | 5474.4 | 6448.0 | 15521.2 | 571.0 | 838.8 |
| `1g-prealloc-warm-c10-none-cp` | 200 | 0 | 8634.4 | 14509.0 | 20241.0 | 20805.3 | 1149.1 | 2597.7 |
| `1g-prealloc-warm-c100-none-cp` | 200 | 0 | 32601.3 | 34326.1 | 34769.4 | 35159.4 | 11493.1 | 20776.5 |
| `1g-frag-warm-c1-none-cp` | 200 | 0 | 8739.0 | 10293.7 | 12044.1 | 15864.1 | 5097.4 | 5931.2 |
| `1g-frag-warm-c10-none-cp` | 200 | 0 | 33741.3 | 56294.8 | 61965.7 | 67836.9 | 29809.6 | 58716.0 |
| `1g-frag-warm-c100-none-cp` | 200 | 0 | 258420.0 | 481298.2 | 502237.1 | 515461.6 | 254274.9 | 498809.8 |

### Phase breakdown, p50 / p99 in microseconds, every cell

| Cell | create | FICLONE or cp | file fsync | dir fsync | verify | concurrent unlink |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `100m-sterile-warm-c1-none-ficlone` | 16.1 / 44.8 | 50.8 / 112.5 | 591.5 / 1213.1 | 0.3 / 0.8 | 4.6 / 28.8 | - |
| `100m-sterile-warm-c10-none-ficlone` | 37.3 / 135.5 | 222.8 / 803.6 | 1421.2 / 3125.2 | 0.4 / 1054.0 | 8.8 / 79.9 | - |
| `100m-sterile-warm-c100-none-ficlone` | 557.9 / 1247.9 | 3753.3 / 6979.5 | 2094.2 / 3346.7 | 0.4 / 1779.8 | 216.3 / 1250.3 | - |
| `100m-sterile-cold-c1-none-ficlone` | 298.6 / 1235.7 | 539.7 / 1786.1 | 1358.8 / 2483.2 | 0.8 / 2.3 | 9.7 / 353.8 | - |
| `100m-sterile-cold-c10-none-ficlone` | 595.9 / 2098.2 | 596.3 / 2184.8 | 1812.4 / 3697.6 | 0.5 / 1801.2 | 35.9 / 2639.9 | - |
| `100m-sterile-cold-c100-none-ficlone` | 1590.2 / 4132.7 | 3540.8 / 7006.5 | 2263.4 / 4785.8 | 0.4 / 2727.6 | 228.1 / 1493.4 | - |
| `100m-prealloc-warm-c1-none-ficlone` | 20.5 / 41.7 | 54.2 / 121.0 | 583.7 / 1233.2 | 0.2 / 0.7 | 4.2 / 32.6 | - |
| `100m-prealloc-warm-c10-none-ficlone` | 57.1 / 164.5 | 252.2 / 986.5 | 1156.7 / 2797.5 | 0.4 / 729.8 | 16.8 / 639.9 | - |
| `100m-prealloc-warm-c100-none-ficlone` | 149.2 / 1178.4 | 5578.3 / 10801.1 | 1910.1 / 4340.6 | 0.4 / 2311.1 | 189.8 / 1406.3 | - |
| `100m-prealloc-cold-c1-none-ficlone` | 340.0 / 1297.4 | 411.3 / 1946.9 | 1317.9 / 2488.3 | 0.9 / 2.4 | 10.5 / 183.0 | - |
| `100m-prealloc-cold-c10-none-ficlone` | 584.6 / 2865.0 | 487.6 / 1716.6 | 1609.4 / 3309.9 | 0.5 / 1294.0 | 36.4 / 1661.4 | - |
| `100m-prealloc-cold-c100-none-ficlone` | 1414.2 / 2890.8 | 4254.2 / 7170.0 | 1754.7 / 3764.4 | 0.4 / 2062.8 | 205.1 / 481.5 | - |
| `100m-frag-warm-c1-none-ficlone` | 22.4 / 48.4 | 479.5 / 683.7 | 607.6 / 1226.8 | 0.3 / 0.8 | 4.9 / 50.0 | - |
| `100m-frag-warm-c10-none-ficlone` | 55.1 / 138.1 | 2755.6 / 5309.6 | 741.7 / 1716.6 | 0.3 / 1.0 | 5.4 / 39.3 | - |
| `100m-frag-warm-c100-none-ficlone` | 148.8 / 795.9 | 27063.5 / 54184.3 | 1002.6 / 2443.3 | 0.4 / 1.8 | 8.9 / 297.3 | - |
| `100m-frag-cold-c1-none-ficlone` | 410.8 / 1452.0 | 1059.6 / 1942.7 | 1446.8 / 2289.3 | 0.8 / 1.8 | 9.8 / 285.8 | - |
| `100m-frag-cold-c10-none-ficlone` | 768.7 / 3230.0 | 3046.7 / 6100.4 | 1243.3 / 2955.0 | 0.5 / 978.9 | 7.5 / 69.3 | - |
| `100m-frag-cold-c100-none-ficlone` | 976.0 / 3665.1 | 26958.8 / 52657.2 | 1095.1 / 3247.4 | 0.4 / 1500.2 | 10.1 / 108.0 | - |
| `1g-sterile-warm-c1-none-ficlone` | 22.0 / 49.3 | 98.9 / 158.1 | 604.4 / 1087.3 | 0.2 / 0.7 | 7.4 / 73.8 | - |
| `1g-sterile-warm-c10-none-ficlone` | 39.9 / 149.1 | 565.2 / 2237.7 | 1439.8 / 2816.0 | 0.4 / 823.0 | 78.3 / 2963.2 | - |
| `1g-sterile-warm-c100-none-ficlone` | 154.0 / 809.1 | 9403.6 / 18703.7 | 1960.7 / 3442.5 | 0.4 / 1902.6 | 260.5 / 696.0 | - |
| `1g-sterile-cold-c1-none-ficlone` | 267.6 / 1382.9 | 655.0 / 1463.7 | 1460.3 / 2439.6 | 0.9 / 1.6 | 13.1 / 313.9 | - |
| `1g-sterile-cold-c10-none-ficlone` | 496.4 / 2522.3 | 1035.9 / 2675.0 | 1698.0 / 3696.2 | 0.5 / 1905.0 | 26.8 / 6440.8 | - |
| `1g-sterile-cold-c100-none-ficlone` | 1973.2 / 4021.5 | 9095.5 / 20050.1 | 1784.1 / 3689.2 | 0.4 / 1953.5 | 231.2 / 6171.5 | - |
| `1g-prealloc-warm-c1-none-ficlone` | 21.9 / 42.7 | 102.1 / 244.6 | 600.3 / 1222.9 | 0.2 / 0.7 | 7.6 / 96.3 | - |
| `1g-prealloc-warm-c10-none-ficlone` | 56.3 / 131.7 | 561.2 / 1873.9 | 1384.5 / 2383.1 | 0.4 / 918.9 | 57.6 / 224.4 | - |
| `1g-prealloc-warm-c100-none-ficlone` | 113.6 / 893.0 | 10554.9 / 20287.7 | 1852.2 / 3384.3 | 0.4 / 1699.6 | 190.0 / 674.7 | - |
| `1g-prealloc-cold-c1-none-ficlone` | 274.0 / 1176.2 | 720.6 / 1598.5 | 1489.9 / 2571.6 | 0.9 / 1.8 | 12.8 / 237.5 | - |
| `1g-prealloc-cold-c10-none-ficlone` | 504.9 / 1954.3 | 823.3 / 1709.4 | 1904.7 / 3112.3 | 0.5 / 1011.3 | 101.4 / 3691.1 | - |
| `1g-prealloc-cold-c100-none-ficlone` | 1072.6 / 3037.0 | 10444.7 / 21431.5 | 2049.2 / 3909.5 | 0.4 / 1760.0 | 217.3 / 518.8 | - |
| `1g-frag-warm-c1-none-ficlone` | 21.4 / 54.7 | 4924.8 / 6303.5 | 665.5 / 1535.7 | 0.4 / 1.0 | 9.3 / 27.6 | - |
| `1g-frag-warm-c10-none-ficlone` | 61.1 / 170.8 | 26207.9 / 50193.7 | 731.8 / 1714.1 | 0.4 / 1.1 | 9.7 / 28.3 | - |
| `1g-frag-warm-c100-none-ficlone` | 130.2 / 345.5 | 244114.0 / 488238.3 | 885.1 / 2579.4 | 0.5 / 1.4 | 13.4 / 53.3 | - |
| `1g-frag-cold-c1-none-ficlone` | 284.7 / 1365.0 | 5332.3 / 6535.7 | 1542.4 / 2625.2 | 0.8 / 1.8 | 12.2 / 272.6 | - |
| `1g-frag-cold-c10-none-ficlone` | 791.6 / 2407.4 | 25866.9 / 50622.1 | 1135.3 / 2248.3 | 0.5 / 1.1 | 10.4 / 28.9 | - |
| `1g-frag-cold-c100-none-ficlone` | 1545.5 / 4362.9 | 246292.3 / 485098.9 | 955.8 / 2275.8 | 0.5 / 1.5 | 13.4 / 38.6 | - |
| `4g-sterile-warm-c1-none-ficlone` | 14.0 / 45.4 | 135.1 / 275.2 | 562.7 / 1262.2 | 0.3 / 0.7 | 9.6 / 104.1 | - |
| `4g-sterile-warm-c10-none-ficlone` | 47.9 / 119.7 | 916.5 / 2901.7 | 1056.0 / 2023.7 | 0.3 / 546.7 | 144.3 / 1681.4 | - |
| `4g-sterile-warm-c100-none-ficlone` | 149.5 / 498.6 | 15696.1 / 31784.3 | 1821.7 / 3687.1 | 0.4 / 1.0 | 151.1 / 2369.8 | - |
| `4g-sterile-cold-c1-none-ficlone` | 283.7 / 1294.7 | 967.7 / 2050.3 | 1451.7 / 2512.2 | 0.9 / 1.7 | 15.0 / 139.9 | - |
| `4g-sterile-cold-c10-none-ficlone` | 936.9 / 3142.5 | 1268.1 / 2923.9 | 1880.1 / 3707.7 | 0.6 / 2678.4 | 52.6 / 2005.1 | - |
| `4g-sterile-cold-c100-none-ficlone` | 1853.3 / 4131.3 | 13505.5 / 24753.2 | 1672.2 / 3461.9 | 0.4 / 2785.0 | 164.4 / 4132.7 | - |
| `4g-prealloc-warm-c1-none-ficlone` | 14.9 / 44.6 | 138.1 / 376.2 | 581.0 / 1280.3 | 0.3 / 0.7 | 9.9 / 99.6 | - |
| `4g-prealloc-warm-c10-none-ficlone` | 54.1 / 126.3 | 939.2 / 4355.6 | 1214.1 / 3170.7 | 0.4 / 781.6 | 115.8 / 934.4 | - |
| `4g-prealloc-warm-c100-none-ficlone` | 94.2 / 1933.0 | 15492.2 / 29344.8 | 1853.8 / 2875.0 | 0.4 / 1633.3 | 165.2 / 1270.0 | - |
| `4g-prealloc-cold-c1-none-ficlone` | 474.6 / 17313.8 | 1082.2 / 2851.6 | 2264.9 / 6962.8 | 0.9 / 2.0 | 15.0 / 217.7 | - |
| `4g-prealloc-cold-c10-none-ficlone` | 773.9 / 18935.6 | 1704.9 / 18174.8 | 11802.1 / 17581.4 | 0.6 / 3529.4 | 15.3 / 3789.9 | - |
| `4g-prealloc-cold-c100-none-ficlone` | 1055.8 / 3319.8 | 19379.5 / 55369.1 | 13068.2 / 18008.2 | 0.5 / 5582.0 | 14242.8 / 19003.1 | - |
| `4g-frag-warm-c1-none-ficlone` | 20.2 / 53.7 | 17816.6 / 20584.7 | 3588.6 / 5181.7 | 0.7 / 2.0 | 11.6 / 40.5 | - |
| `4g-frag-warm-c10-none-ficlone` | 56.6 / 211.4 | 111915.8 / 211179.1 | 3767.4 / 6964.5 | 0.8 / 2.1 | 14.2 / 36.3 | - |
| `4g-frag-warm-c100-none-ficlone` | 72.4 / 13713.9 | 922223.1 / 1861428.0 | 5650.9 / 7928.1 | 0.7 / 1.8 | 18.0 / 45.5 | - |
| `4g-frag-cold-c1-none-ficlone` | 583.2 / 1429.2 | 19709.5 / 25444.7 | 7564.4 / 9223.6 | 0.9 / 2.0 | 14.3 / 496.5 | - |
| `4g-frag-cold-c10-none-ficlone` | 1532.6 / 17045.7 | 102937.3 / 197767.8 | 4719.1 / 8258.9 | 0.7 / 1.7 | 13.1 / 34.8 | - |
| `4g-frag-cold-c100-none-ficlone` | 4626.5 / 19970.7 | 936275.8 / 1854700.6 | 5021.1 / 11309.1 | 0.8 / 1.8 | 18.3 / 50.2 | - |
| `1g-sterile-warm-c100-freespace-ficlone` | 742.8 / 3198.7 | 8312.9 / 13540.2 | 17652.7 / 25046.2 | 0.4 / 9750.9 | 3634.6 / 10337.1 | - |
| `1g-sterile-warm-c100-cleanup-ficlone` | 37.3 / 1328.0 | 21823.7 / 30475.4 | 15339.8 / 20697.6 | 0.5 / 3576.4 | 970.1 / 16772.9 | 16818.6 / 18172.6 |
| `1g-sterile-warm-c1-none-cp` | 0.0 / 0.0 | 591.3 / 912.1 | 3188.6 / 6605.8 | 0.7 / 1.2 | 19.8 / 31.1 | - |
| `1g-sterile-warm-c10-none-cp` | 0.0 / 0.0 | 1187.8 / 2664.2 | 11580.7 / 12785.0 | 0.9 / 9600.8 | 26.5 / 1186.4 | - |
| `1g-sterile-warm-c100-none-cp` | 0.0 / 0.0 | 11936.4 / 31816.8 | 17065.7 / 27750.8 | 0.5 / 13190.1 | 1386.4 / 4282.5 | - |
| `1g-prealloc-warm-c100-freespace-ficlone` | 177.7 / 1114.7 | 7734.4 / 27086.3 | 15356.7 / 21763.4 | 0.3 / 9575.3 | 4123.6 / 16286.2 | - |
| `1g-prealloc-warm-c100-cleanup-ficlone` | 70.7 / 1479.8 | 32384.7 / 40862.5 | 16483.8 / 32945.4 | 0.4 / 2.6 | 1281.2 / 3881.3 | 15233.4 / 16251.5 |
| `1g-prealloc-warm-c1-none-cp` | 0.0 / 0.0 | 571.0 / 838.8 | 3236.0 / 5943.1 | 0.7 / 1.2 | 20.9 / 29.9 | - |
| `1g-prealloc-warm-c10-none-cp` | 0.0 / 0.0 | 1149.1 / 2597.7 | 6919.6 / 18911.8 | 0.7 / 3661.1 | 59.3 / 1432.1 | - |
| `1g-prealloc-warm-c100-none-cp` | 0.0 / 0.0 | 11493.1 / 20776.5 | 15419.9 / 21357.0 | 0.4 / 13145.2 | 2184.9 / 6555.1 | - |
| `1g-frag-warm-c100-freespace-ficlone` | 51.6 / 321.8 | 249668.5 / 494504.7 | 3632.2 / 32041.9 | 0.8 / 1.7 | 15.9 / 348.1 | - |
| `1g-frag-warm-c100-cleanup-ficlone` | 36.0 / 1050.9 | 253490.0 / 488947.5 | 3491.5 / 17696.8 | 0.6 / 1.6 | 14.7 / 460.0 | 14299.8 / 18211.2 |
| `1g-frag-warm-c1-none-cp` | 0.0 / 0.0 | 5097.4 / 5931.2 | 3600.1 / 6134.9 | 0.7 / 2.3 | 20.2 / 32.2 | - |
| `1g-frag-warm-c10-none-cp` | 0.0 / 0.0 | 29809.6 / 58716.0 | 3294.1 / 6642.6 | 0.6 / 2.0 | 12.0 / 52.3 | - |
| `1g-frag-warm-c100-none-cp` | 0.0 / 0.0 | 254274.9 / 498809.8 | 3492.2 / 8649.0 | 0.7 / 7.8 | 20.2 / 89.8 | - |

## Observations

Clones from one template serialize on the template inode.
Inside one 100-way burst of `1g-sterile-warm-c100-none-ficlone` the sorted `FICLONE` durations rise almost linearly from 0.1 ms to 15.6 ms, the largest one equals the burst wall time, and the 100-way `FICLONE` p50 is 9.4 ms against 0.1 ms single-threaded; the same shape holds for every 100-way cell.
XFS takes both inode locks for the remap, so one template can hand out heads only one at a time, and a burst of `c` requests against one class costs about `c` times the single-clone `ioctl`.
Parallel replenishment therefore gains nothing per class; it needs separate template inodes or simply enough prepared heads.

The `ioctl` cost is proportional to the source extent count, about 0.6 us per extent on this host: 1 GiB with 14 extents costs 99 us at p50, with 7,813 extents 4.9 ms, and 4 GiB with 31,738 extents 17.8 ms.
Template size alone barely matters: 100 MiB, 1 GiB, and 4 GiB sterile templates cost 51 us, 99 us, and 135 us at p50 single-threaded, because a sterile template has only 10 to 19 extents.
The sterile recipe with `lazy_itable_init=0` is therefore the right template shape, and a fragmented template must never be certified.

`FICLONE` maps unwritten source extents as holes in the destination, not as shared extents.
Every clone of the `prealloc` and `frag` templates reports exactly the written extents of its sterile sibling, 5, 11, and 15, all shared, and a direct check on the same image with `xfs_io fiemap` showed a source's fallocated extent (`0x801`) become a `hole` in the clone while the written extent was shared (`0x2001`), with the clone's allocated blocks falling from 8,192 to 2,048 sectors and a fully fallocated fragmented file cloning to a file with zero allocated blocks.
A head therefore never inherits capacity reservation from its template; if a class must guarantee its full logical size on the host filesystem, the allocator has to `fallocate` the head itself after cloning, which this profile does not do, and the free-space evidence plus the `ENOSPC` proof remain the only capacity guard.

The durable syncs dominate a single warm clone.
For a sterile 1 GiB template the phases at p50 are 22 us `create`, 99 us `FICLONE`, 604 us file `fsync`, 0.2 us directory `fsync`, and 7 us verification, so the complete cost is 0.75 ms at p50 and 1.25 ms at p99; the directory `fsync` is free because the XFS log commit of the file `fsync` already covered the directory entry.
Cold cache adds about 0.25 ms to `create`, 0.5 ms to `FICLONE`, and 0.9 ms to the file `fsync`, so a cold single clone costs 2.6 ms at p50 and 4.1 ms at p99.

Pressure moves the tail further out.
Ten percent free space raised the 100-way sterile 1 GiB complete-clone p99 from 21.5 ms to 37.2 ms, mostly through the file `fsync` (2.0 ms to 17.7 ms at p50).
One hundred concurrent unlinks of 1 GiB heads raised it to 57.1 ms, each unlink plus directory `fsync` itself costing 16.8 ms at p50 and 18.2 ms at p99 under that load, and to 506.7 ms for the fragmented template.

The Miosa-today `cp --reflink=always` subprocess costs 3.8 ms at p50 and 7.2 ms at p99 per sterile 1 GiB head against 0.75 ms and 1.25 ms in-process, and 32.3 ms against 11.7 ms at p50 100-way; the child process alone is 0.59 ms at p50 where the `ioctl` is 0.10 ms, and the file `fsync` that follows it is five times more expensive (3.2 ms against 0.6 ms at p50), which this run measured but did not explain.
In-process `FICLONE` is the mechanism to keep; the subprocess is retired.

## Raw data

- Samples: `xfs-reflink-samples.jsonl`, 14,479 lines, 5,559,345 bytes, SHA-256 `dc29d9efaed64e8f6805fc9294f8f7a401763c903884d8c31b57260b6ccee675`; one `identity` record, nine `template` records, 13,800 `sample` records, 600 `unlink` records, and 69 `summary` records.
- The file is not committed because of its size; it is retained beside `container-all.log`, `live-tests.log`, `xfs-info-reflink.txt`, `mountinfo.txt`, `losetup.txt`, `loop-map.txt`, `template-digests-after-remount.txt`, `container-identity.txt`, `container-image-id.txt`, and the `xfs_io` check log in the run's `raw/` directory, and every table above is reproducible from it with nearest-rank percentiles.
- The Markdown tables were rendered by `soma-storage-bench` itself and regrouped by template size without changing any value.

## Unproven

- Production-host latency: the loop device, the sparse backing file on ext4, the shared development host, and kernel `7.0.0-30-generic` all differ from a certified bare-metal Ubuntu 24.04 host with XFS on a raw NVMe partition; the shape of the result, not its absolute values, is the evidence.
- Quota exhaustion (`EDQUOT`) is classified like `ENOSPC` but was not exercised because the mount has `noquota`.
- Unsupported mount options, a crash during clone or unlink, and a filesystem that reports `FICLONE` success without shared extents were not exercised; the probe would reject the last case through the `FIEMAP` check but no such filesystem was available.
- The fragmentation dimension is synthetic `fallocate` fragmentation of a sterile template; fragmentation from real guest writes is a property of heads, not templates, and never reaches the clone path.
- The result was produced once on one host; it has not been repeated on a second host or a second day.
- `shellcheck` and `typos` were not installed on the development host; the two shell scripts were checked with `koalaman/shellcheck:v0.10.0` in a container instead.

## Next dependency

The host allocator in `soma-hostd` consumes `soma-storage`: it publishes one `ClassCatalog`, probes the head filesystem once, keeps a bounded pool of prepared, synced, verified heads per class whose depth covers the certified burst, replenishes asynchronously from the sterile template with `clone::clone_head`, transfers the open descriptor at Launch, and destroys released heads through `release::release_head` on a background path that `reconcile::reconcile` audits.
