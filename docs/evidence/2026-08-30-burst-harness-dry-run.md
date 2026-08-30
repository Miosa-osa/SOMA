# Burst harness proof on the Docker Backend - 2026-08-30

## Evidence boundary

This result proves that the burst harness in `benchmarks/local_alpha/burst` can open every slot of a burst at once against the `docker` Backend through the `soma` command line, time each slot from immediately before the create call to immediately after the workload command succeeded inside the sandbox, destroy and verify every sandbox outside that timer, retain all 230 attempted samples across 4 cohorts with their failures, and refuse to publish an incomplete or class-mixed run.
Every sample was declared as the `warm-cache-restore` experiment class with the `warm_host_page_cache` cache state and observed `linux_container` isolation.

These samples are a harness proof on the `docker` Backend with `linux_container` isolation. They are not a SOMA KVM performance result and they are not comparable to any provider benchmark, including the ComputeSDK Burst TTI benchmark.
It does not prove any latency objective.
Read the final section before quoting any number here.

## Identities

- SOMA Git revision: `9cde4d43c29f9d273f78bf2e5d51e27b973e05de`, worktree clean: `True`.
- Measured binary: `soma`, 2,958,872 bytes, SHA-256 `4f9dd296739ddcd4abadcb506e2a2dcc0896daefbb176d96e028addcc84f7386`, built by `cargo build --locked --release -p soma-cli -p soma-mcp`.
- Cargo source SHA-256 `515f671dab136ff72b13cb03fdffc2ad0b261d2857667de46efd36c67d840eeb`; benchmark harness SHA-256 `33451044b04c2bf43c221706d5417cbde6ca9b68ce71ae5cc089c3709aa48005`.
- Host kernel: `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC Fri Aug  7 13:27:52 UTC 2` x86_64.
- CPU: Intel(R) Core(TM) Ultra 9 275HX, 24 logical CPUs, microcode `0x11b`.
- Memory: 65249064 kB total, 56105464 kB available when the run started.
- Storage: mount `/`, filesystem `ext4`, source `/dev/nvme1n1p2`, device `259:3` (SKHynix_HFS001TEJ9X115N), super options `rw`.
- KVM: `/dev/kvm` present `True`, readable by the harness `True`, modules `kvm, kvm_intel`.
- Backend probe: `{"exit_code":0,"report":{"backend":"docker","production_ready":false,"reason":"backend_probe_passed","runtime_ready":true,"runtime_version":"docker-29.3.0","status":"probe_passed","supported_target":true}}`.
- Observed backend: `["docker_container"]`, isolation `[{"state":"observed","value":"linux_container"}]`, preparation `[{"state":"observed","value":"on_demand"}]`.
- Workload identity reported by the Backend: `[{"generation_id":null,"index_digest":null,"manifest_digest":"sha256:cbf412bcf1379481c80f65208703910fe543b3a948ae74a32a10ca3789dc13ab","platform":{"architecture":"amd64","operating_system":"linux","variant":null}},{"generation_id":null,"index_digest":null,"manifest_digest":"sha256:87a4f951f28b85d189df365d24c479d3bdb70be77c1ff5c9029db2ef67e251ac","platform":{"architecture":"amd64","operating_system":"linux","variant":null}}]`.
- Declared experiment class `warm-cache-restore`, cache state `warm_host_page_cache`, network policy `denied`, shape `{"memory_mib":1024,"storage_mib":10240,"vcpus":1}`.
- Prepared before the timer:
  - the OCI image was pushed to a local registry on 127.0.0.1:5000 and pulled into the Docker daemon before the run, so no Docker Hub round trip happens inside the timer.
  - the Docker daemon was already running with this image present and its layers warm in the host page cache.
  - a discarded two-sample warm-up cohort of the same image and command ran immediately before this cohort.
  - the soma release binaries were built and their build manifest written before the run.
- Excluded from the timer:
  - sandbox destruction, which every sample still executes and verifies after the timer stops.
  - the release build and its build manifest.
  - temporary state-root creation and removal.
  - host metadata collection and report generation.
  - every preparation listed above.

## Invocation

Each slot runs exactly these three processes, with fresh identities:

```sh
$SOMA_BIN --format json --backend docker --state-root $STATE_ROOT machine launch --operation-id $OPERATION_ID --instance-id $INSTANCE_ID --vcpus 1 --memory-mib 1024 --storage-mib 10240 --egress denied --dns denied 127.0.0.1:5000/busybox:stable-musl
$SOMA_BIN --format json --backend docker --state-root $STATE_ROOT machine exec --operation-id $OPERATION_ID --instance-id $INSTANCE_ID --timeout-ms 30000 --max-output-bytes 1048576 -- /bin/busybox true
$SOMA_BIN --format json --backend docker --state-root $STATE_ROOT machine destroy --operation-id $OPERATION_ID --instance-id $INSTANCE_ID
```

## Measured boundary

The time-to-first-command clock is `time.perf_counter_ns` in the slot's own thread with the boundary `immediately_before_the_launch_process_capture_to_immediately_after_the_exec_process_exit_and_pipe_drain; includes_two_soma_process_spawns_and_response_reading; excludes_destroy`.
A cohort of N iterations at concurrency C runs N divided by C bursts, and every slot of a burst is released by one barrier and creates its own sandbox; the cohort table names N and C for each cohort.
Wall time covers the whole cohort including the excluded destruction.
Every percentile is nearest rank over successful samples only, so p99 of 100 samples is the 99th ordered value and p99 of 10 samples is the largest.
Stage rows are the facade milestones the receipts carry; the harness overhead row is the measured time to first command minus the launch and exec facade totals, which is the cost of two process spawns and their response reading.

## Cohorts

| Cohort | Image | Command | Concurrency | Iterations | Successful | Wall time (ns) |
|---|---|---|---:|---:|---:|---:|
| 127-0-0-1-5000-busybox-stable-musl-c1-n20 | `127.0.0.1:5000/busybox:stable-musl` | `/bin/busybox true` | 1 | 20 | 20 of 20 | 8,685,359,002 (8.69 s) |
| 127-0-0-1-5000-busybox-stable-musl-c10-n100 | `127.0.0.1:5000/busybox:stable-musl` | `/bin/busybox true` | 10 | 100 | 100 of 100 | 7,149,294,167 (7.15 s) |
| 127-0-0-1-5000-busybox-stable-musl-c100-n100 | `127.0.0.1:5000/busybox:stable-musl` | `/bin/busybox true` | 100 | 100 | 100 of 100 | 6,687,806,628 (6.69 s) |
| 127-0-0-1-5000-node-22-c1-n10 | `127.0.0.1:5000/node:22` | `/usr/local/bin/node -v` | 1 | 10 | 10 of 10 | 7,181,706,244 (7.18 s) |

## Time to first command

| Cohort | Success rate | min | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|---:|
| 127-0-0-1-5000-busybox-stable-musl-c1-n20 | 100.0% | 278,433,751 (278.43 ms) | 328,683,094 (328.68 ms) | 329,021,392 (329.02 ms) | 329,080,884 (329.08 ms) | 329,080,884 (329.08 ms) |
| 127-0-0-1-5000-busybox-stable-musl-c10-n100 | 100.0% | 433,637,334 (433.64 ms) | 484,966,551 (484.97 ms) | 535,164,090 (535.16 ms) | 583,473,019 (583.47 ms) | 583,535,572 (583.54 ms) |
| 127-0-0-1-5000-busybox-stable-musl-c100-n100 | 100.0% | 1,572,183,580 (1572.18 ms) | 3,679,344,959 (3679.34 ms) | 4,319,849,350 (4319.85 ms) | 4,424,458,570 (4424.46 ms) | 4,492,526,594 (4492.53 ms) |
| 127-0-0-1-5000-node-22-c1-n10 | 100.0% | 479,517,613 (479.52 ms) | 529,286,029 (529.29 ms) | 579,343,233 (579.34 ms) | 579,343,233 (579.34 ms) | 579,343,233 (579.34 ms) |

## Stage timings (ns)

### 127-0-0-1-5000-busybox-stable-musl-c1-n20

| Stage | Samples | min | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|---:|
| launch: workload resolution | 20 | 43,978,403 | 47,795,417 | 52,114,086 | 56,072,043 | 56,072,043 |
| launch: admission | 20 | 1,666,527 | 1,842,450 | 2,041,708 | 2,179,654 | 2,179,654 |
| launch: machine creation | 20 | 102,263,274 | 112,301,297 | 119,889,188 | 126,200,997 | 126,200,997 |
| launch: readiness | 20 | 213 | 284 | 701 | 706 | 706 |
| launch: facade total | 20 | 151,746,855 | 161,363,888 | 173,604,111 | 174,084,390 | 174,084,390 |
| exec: command dispatch | 20 | 41 | 67 | 211 | 229 | 229 |
| exec: command execution | 20 | 35,188,983 | 51,618,142 | 59,866,476 | 61,879,775 | 61,879,775 |
| exec: facade total | 20 | 35,189,212 | 51,618,185 | 59,866,669 | 61,879,833 | 61,879,833 |
| destroy: cleanup dispatch | 20 | 41 | 62 | 237 | 248 | 248 |
| destroy: cleanup | 20 | 63,912,388 | 68,045,861 | 70,676,870 | 76,827,140 | 76,827,140 |
| destroy: facade total | 20 | 63,912,435 | 68,046,028 | 70,676,937 | 76,827,195 | 76,827,195 |
| harness: process and transport overhead | 20 | 75,536,523 | 108,435,734 | 123,498,052 | 133,477,609 | 133,477,609 |

### 127-0-0-1-5000-busybox-stable-musl-c10-n100

| Stage | Samples | min | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|---:|
| launch: workload resolution | 100 | 113,720,414 | 123,992,233 | 142,019,685 | 143,525,223 | 145,553,962 |
| launch: admission | 100 | 1,723,353 | 2,914,891 | 3,882,701 | 4,599,941 | 4,923,068 |
| launch: machine creation | 100 | 172,224,839 | 195,526,906 | 240,735,291 | 258,445,767 | 259,525,435 |
| launch: readiness | 100 | 146 | 299 | 549 | 636 | 680 |
| launch: facade total | 100 | 291,836,835 | 319,922,072 | 387,319,260 | 404,569,719 | 406,120,952 |
| exec: command dispatch | 100 | 52 | 83 | 135 | 179 | 181 |
| exec: command execution | 100 | 46,521,585 | 62,479,622 | 70,162,578 | 70,749,184 | 70,760,255 |
| exec: facade total | 100 | 46,521,683 | 62,479,688 | 70,162,663 | 70,749,290 | 70,760,328 |
| destroy: cleanup dispatch | 100 | 58 | 94 | 145 | 171 | 198 |
| destroy: cleanup | 100 | 96,923,514 | 152,033,085 | 178,124,378 | 183,898,400 | 186,470,647 |
| destroy: facade total | 100 | 96,923,573 | 152,033,170 | 178,124,509 | 183,898,478 | 186,470,781 |
| harness: process and transport overhead | 100 | 75,911,884 | 105,703,423 | 122,934,769 | 129,996,923 | 136,240,862 |

### 127-0-0-1-5000-busybox-stable-musl-c100-n100

| Stage | Samples | min | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|---:|
| launch: workload resolution | 100 | 489,896,446 | 1,108,501,413 | 1,176,515,679 | 1,193,795,430 | 1,223,081,808 |
| launch: admission | 100 | 2,029,281 | 4,503,498 | 9,096,334 | 16,705,555 | 17,400,660 |
| launch: machine creation | 100 | 749,356,105 | 1,785,586,304 | 2,342,516,293 | 2,442,008,386 | 2,464,197,252 |
| launch: readiness | 100 | 226 | 501 | 734 | 775 | 789 |
| launch: facade total | 100 | 1,241,336,391 | 2,928,283,932 | 3,505,372,938 | 3,582,256,676 | 3,593,170,120 |
| exec: command dispatch | 100 | 82 | 172 | 261 | 296 | 309 |
| exec: command execution | 100 | 79,360,976 | 227,811,658 | 362,606,251 | 382,994,063 | 392,568,656 |
| exec: facade total | 100 | 79,361,119 | 227,811,852 | 362,606,512 | 382,994,199 | 392,568,886 |
| destroy: cleanup dispatch | 100 | 80 | 162 | 294 | 354 | 684 |
| destroy: cleanup | 100 | 581,792,172 | 1,605,882,140 | 1,942,718,729 | 1,965,547,857 | 1,996,272,228 |
| destroy: facade total | 100 | 581,792,257 | 1,605,882,363 | 1,942,718,813 | 1,965,548,101 | 1,996,272,392 |
| harness: process and transport overhead | 100 | 118,364,467 | 404,686,424 | 605,963,219 | 630,201,339 | 696,263,119 |

### 127-0-0-1-5000-node-22-c1-n10

| Stage | Samples | min | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|---:|
| launch: workload resolution | 10 | 141,889,478 | 148,736,158 | 172,296,637 | 172,296,637 | 172,296,637 |
| launch: admission | 10 | 9,861,422 | 10,798,434 | 14,326,853 | 14,326,853 | 14,326,853 |
| launch: machine creation | 10 | 178,067,028 | 185,845,990 | 212,516,428 | 212,516,428 | 212,516,428 |
| launch: readiness | 10 | 187 | 268 | 736 | 736 | 736 |
| launch: facade total | 10 | 336,923,714 | 349,547,613 | 373,335,968 | 373,335,968 | 373,335,968 |
| exec: command dispatch | 10 | 45 | 57 | 237 | 237 | 237 |
| exec: command execution | 10 | 41,384,163 | 49,795,835 | 58,002,976 | 58,002,976 | 58,002,976 |
| exec: facade total | 10 | 41,384,208 | 49,795,897 | 58,003,021 | 58,003,021 | 58,003,021 |
| destroy: cleanup dispatch | 10 | 48 | 54 | 250 | 250 | 250 |
| destroy: cleanup | 10 | 113,375,205 | 116,541,424 | 133,879,267 | 133,879,267 | 133,879,267 |
| destroy: facade total | 10 | 113,375,259 | 116,541,476 | 133,879,315 | 133,879,315 | 133,879,315 |
| harness: process and transport overhead | 10 | 95,061,649 | 118,895,234 | 184,457,712 | 184,457,712 | 184,457,712 |

## Command output

- `127-0-0-1-5000-busybox-stable-musl-c1-n20`: 20 successful commands returned exit status 0 and 0 stdout bytes (empty).
- `127-0-0-1-5000-busybox-stable-musl-c10-n100`: 100 successful commands returned exit status 0 and 0 stdout bytes (empty).
- `127-0-0-1-5000-busybox-stable-musl-c100-n100`: 100 successful commands returned exit status 0 and 0 stdout bytes (empty).
- `127-0-0-1-5000-node-22-c1-n10`: 10 successful commands returned exit status 0 and 9 stdout bytes exactly `v22.23.2\n`.

## Failures

No sample failed; every one of the 230 samples succeeded.

## Raw data

- `127-0-0-1-5000-busybox-stable-musl-c1-n20`: `busybox-c1.jsonl`, 22 lines, 67,189 bytes, SHA-256 `4d7a615ae403ced9cde320a9bfde7030611a917306810d5a6083daa58a64bf83`.
- `127-0-0-1-5000-busybox-stable-musl-c10-n100`: `busybox-c10.jsonl`, 102 lines, 318,673 bytes, SHA-256 `f7a0dc78418a27ca7e78d0b21eb1268db03099249b8d50f43a79021ae8340bdf`.
- `127-0-0-1-5000-busybox-stable-musl-c100-n100`: `busybox-c100.jsonl`, 102 lines, 319,652 bytes, SHA-256 `6cc8adc99011e2c73b176751d82804111e330a98468f3a48b25ea142836aca86`.
- `127-0-0-1-5000-node-22-c1-n10`: `node22-c1.jsonl`, 12 lines, 35,933 bytes, SHA-256 `40929b6432beec7781952849da557a86ea7a6c4d2eb10cb4e86cc126125ab54a`.

## What this does not prove

- This is a harness proof on the `docker` Backend with `linux_container` isolation. It is not a SOMA KVM performance result, it is not a virtual machine measurement, and it is not comparable to any provider benchmark, including the ComputeSDK Burst TTI benchmark.
- The timer includes two `soma` process spawns and their response reading per sample; a provider SDK measurement does not pay that cost, and the harness overhead stage row states how large it is here.
- Destruction is excluded from every time-to-first-command value. It was executed and its cleanup evidence verified for every sample, and it is inside the reported wall time.
- Percentiles are nearest rank over successful samples only. Failures are listed above and are never merged into the distribution.
- Each cohort was produced once, on one host, on 2026-08-30, without quiescing the host; no repetition on a second host or a second day exists.
- The Backend reported no Generation identity, so no Generation digest is bound to these samples.
