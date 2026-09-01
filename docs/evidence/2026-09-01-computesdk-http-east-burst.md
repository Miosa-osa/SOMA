# Exact HTTP Burst TTI qualification across east hosts

## Result

The later primed-machine campaign moved one stopped identity-free VM and one unlinked private overlay head into every available machine-host child before API readiness.
It also reused one bounded HTTP/1.1 connection for create, first command, and excluded cleanup.

| Final primed cohort | Placement host03/04/10 | Median | p95 | p99 | Raw maximum | Command and cleanup success |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 40/20/40 | 60.01 ms | 69.27 ms | 69.85 ms | 73.59 ms | 100/100 |
| 2 | 40/20/40 | 63.78 ms | 70.75 ms | 71.04 ms | 76.66 ms | 100/100 |
| 3, shared-host contention | 40/20/40 | 68.17 ms | 1,168.84 ms | 1,173.48 ms | 1,175.58 ms | 100/100 |
| Release `0aff1c5` | 40/20/40 | 61.09 ms | 70.61 ms | 71.45 ms | 79.63 ms | 100/100 |

The first two identical final-binary cohorts and the source-bound release cohort beat the quoted Isorun 64/79/80 ms result on median, p95, and p99.
The third cohort is retained as resilience evidence and excluded from clean performance comparison because unrelated encrypted-disk and control-plane work slowed all forty host10 samples together.
Every final receipt reports `prepared_worker`.
The initial qualification binary SHA-256 is `fbeb7229640c56876799196752daf2ed787e2ca545b38c4fa9aa5105324bff90`.
The final x86_64 release built from pushed commit `0aff1c5` has SHA-256 `e565e3f24905f1b498ad9ff6a42e5e7a280bf228681f40fc38fd3f8f106708a5`.
That release returned every host to 64 prepared children with zero zombies after the cohort.
The raw shards and the contamination analysis are retained under [the primed-machine raw record](raw/2026-09-01-primed-http-east/README.md).

The earlier sterile-process results follow for comparison.

The secure qualification cohorts completed every `node -v` command and every cleanup.
They used the exact ComputeSDK timing boundary and a shared release epoch across all three shards.

| Cohort | Placement host03/04/10 | Median | p95 | p99 | Raw maximum | Command and cleanup success |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Sterile host 1, first binary run | 40/24/36 | 78.51 ms | 87.98 ms | 89.02 ms | 95.47 ms | 100/100 |
| Sterile host 2, best observed | 40/24/36 | 62.32 ms | 75.43 ms | 75.94 ms | 85.77 ms | 100/100 |
| Sterile host 3 | 40/24/36 | 69.34 ms | 77.80 ms | 80.54 ms | 87.24 ms | 100/100 |
| Sterile host 4 | 40/24/36 | 67.42 ms | 76.31 ms | 77.81 ms | 81.79 ms | 100/100 |
| Sterile host 5 | 40/24/36 | 68.32 ms | 76.78 ms | 78.03 ms | 90.63 ms | 100/100 |
| Sterile host 6, release validation | 40/24/36 | 70.96 ms | 83.76 ms | 84.43 ms | 85.50 ms | 100/100 |
| Sterile host 7, uniform performance governor | 40/24/36 | 69.41 ms | 77.50 ms | 79.56 ms | 84.78 ms | 100/100 |
| Sterile host 8, host10 overloaded | 34/18/48 | 70.25 ms | 83.73 ms | 84.29 ms | 88.31 ms | 100/100 |
| Sterile host 9, host04 overloaded | 34/32/34 | 66.43 ms | 85.53 ms | 86.99 ms | 89.69 ms | 100/100 |
| Sterile host 10, best validated tail | 40/20/40 | 69.23 ms | 76.14 ms | 79.21 ms | 81.52 ms | 100/100 |

The best observed secure cohort beats the quoted Isorun result of 64 ms median, 79 ms p95, and 80 ms p99 on all three scored metrics.
The following three consecutive cohorts did not repeat the median win, so 62.32 ms is evidence of demonstrated capability rather than a stable public claim.
The repeated tail measurements remained near or below the quoted comparison, while median variance remains an optimization target.

Sterile-host cohorts 1 through 5 used binary SHA-256 `6322445ccfa6e7d5ff5e12c2544700fed210c63d2c4e8080c88df4e30ab53383` on every host.
Sterile-host cohorts 6 through 10 used the fully validated binary SHA-256 `ebeaadfaee2902547399969b7e0d27cd38a8c3849f59524157bae18dc4b98850` on every host.
Each API prepared 64 sterile child processes before listening.
After every burst and delayed refill, each API again owned exactly 64 sterile children, zero zombie children, and zero leaked Instance sockets.
The sterile children owned no VM, guest memory, Instance identity, or Generation descriptor before claim.

The earlier secure descriptor-handoff baseline used binary SHA-256 `4068b77afdb664c0bed15a301a197d9e94680b4c5ac116aecc3a3b3fb53022ed`.
Its best warmed template-fan cohort completed 100/100 commands and cleanups at 65.51 ms median, 74.16 ms p95, and 77.98 ms p99.
The shared open-file-description defect described below was fixed before either of these binaries was measured.

The first secure qualification binary SHA-256 was `a8c6d03be5cdad14e7c29022da6e53f405ed4c3d99953efac56544901e502f4f` on every host.
It cryptographically admits each installed Generation before the API accepts traffic, retains the admitted open files, and transfers independent open file descriptions of those retained inodes to each machine-host process.
The child revalidates manifest identity, profile, descriptor order, file kind, and file size without re-hashing the already verified artifact bytes.
The handoff eliminates path substitution without placing multi-gigabyte hashing inside TTI.

Host03 and host04 produced intermittent `backend_unavailable` launch refusals under higher placement weights during that initial campaign.
Host10 completed both a 64-way capacity probe and the final 98-way shard without a failure.
The root cause was not host capacity.
`File::try_clone` and `SCM_RIGHTS` had duplicated shared open file descriptions, so concurrent kernel, initramfs, and snapshot readers raced one mutable file offset.
Opening an independent description of each retained inode per Launch removed the refusals and produced repeated 100/100 higher-placement cohorts.

The following three cohorts are retained as legacy performance evidence only.
They used a size-only installed-artifact shortcut that was removed because same-size corruption could bypass admission.
They must not be used as release or security evidence.

Three consecutive 100-sandbox cohorts completed 300 of 300 `node -v` commands and 300 of 300 cleanups.

| Cohort | Placement host03/04/10 | Median | p95 | p99 | Raw maximum | Command and cleanup success |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 40/24/36 | 41.56 ms | 55.26 ms | 56.66 ms | 59.44 ms | 100/100 |
| 2 | 40/24/36 | 44.86 ms | 53.76 ms | 55.08 ms | 59.53 ms | 100/100 |
| 3 | 40/24/36 | 42.81 ms | 51.75 ms | 53.30 ms | 56.33 ms | 100/100 |

This is qualification evidence through each host's loopback HTTP API.
It is not an external provider result because it excludes MIOSA's edge, authentication, scheduler, load balancer, and GitHub-runner network path.

## Exact measured boundary

The inspected ComputeSDK benchmark revision was `46dea652fcc372e5acea0c9f372613d86b4b6bab`.
Its Burst TTI timer starts immediately before provider create, waits for the sandbox, executes `node -v`, and stops after the successful command.
Destroy happens after the timed region.
The SOMA harness implements the same boundary through `POST /v1/sandboxes`, `POST /v1/sandboxes/{id}/commands`, and an excluded `DELETE /v1/sandboxes/{id}`.
SOMA additionally requires and reports successful cleanup even though cleanup does not change the ComputeSDK timing sample.

The workload was `node:22`, 1 vCPU, 1,024 MiB memory, 4,096 MiB writable storage, and networking disabled.
The command executed inside every guest was `/usr/local/bin/node -v` and returned Node `v22.23.2` with exit code zero.

## Placement and host profile

All three east hosts expose 32 logical CPUs and approximately 249 GiB of RAM on AMD Ryzen 9 9950X processors.
The empirically selected 40/24/36 placement means 1.25, 0.75, and 1.125 sandbox slots per logical CPU on host03, host04, and host10 respectively.
This fixed split is a qualification input, not a production placement algorithm.
Production placement must derive weights from recent admitted capacity and latency rather than host names.

The legacy qualification API binary SHA-256 was `8cab1532a9d35a2aef17f6d5a1ab42a190515d1876c60863033bd93b28f5f9a5` on every host.
It was built from an uncommitted qualification tree based on revision `fe3d01f85ce7d0fbd96d897a8ebbd3566cdb4229`, so the binary digest rather than a clean release commit is the executable identity.
This prevents the cohort from serving as release evidence.
The API used 64 bounded workers per host and listened on loopback port 18787.
Host03 and host04 used Generation `sha256:f736bd4e424e69a97506951f6f12df318272c8f6a4437cadeb672fae5de097db`.
Host10 used locally captured Generation `sha256:1b2cf33bcbc3841a131488b929e8cb2b86a98de13dfb9be1953693b1bbcdede4`.

| Host | Host kernel | Microcode | NUMA | Prepared and head storage |
| --- | --- | --- | --- | --- |
| host03 | `6.8.0-136-generic` | `0xb404035` | 1 node | `/dev/nvme1n1p5`, XFS reflink |
| host04 | `6.8.0-136-generic` | `0xb404035` | 1 node | `/dev/nvme0n1p5`, XFS reflink |
| host10 | `6.8.0-137-generic` | `0xb404023` | 1 node | `/dev/mapper/miosa_crypt`, XFS reflink |

All hosts used the in-kernel AMD KVM module, one socket, 16 physical cores, 32 threads, and one NUMA node.
Before cohort 7, host03 and host10 used the AMD P-state `powersave` governor while host04 used `performance`, although all three already used the `performance` energy preference.
Cohorts 7 through 10 normalized every CPU on every host to the `performance` governor and retained that setting as an explicit qualification input.
That change improved the repeated 40/24/36 result from 70.96/83.76/84.43 ms to 69.41/77.50/79.56 ms at median/p95/p99.
Placement probes then showed a sharp host-specific contention knee: host10 degraded at 48 concurrent launches and host04 degraded at 32.
The 40/20/40 split produced the best validated tail, while 34/32/34 produced the best validated median.
The guest kernel was the pinned Linux `6.12.107-soma-v1` artifact and the Generation-bound command line was the profile-v1 fixed line for root, writable overlay, vsock, entropy, and no network device.
The exact command line is generated only by `soma_kvm::generation_command_line` and is bound into each Generation manifest.

The experiment class was warm-host-page-cache on-demand snapshot restore.
The certified Generation, captured snapshot, Node runtime pages, API binary, and filesystem metadata were prepared before the timer.
No running or paused machine was preassigned to a request.
There were no request retries.
Each request used the 120-second ComputeSDK timeout profile, and the live runner used one barrier plus a shared future epoch for release.
The retained v1 shard schema omitted that epoch, so these artifacts do not independently prove cross-host synchronization and are legacy qualification evidence.
The current combiner rejects any future shard whose common release epoch is missing.
API lifecycle state lived in host-local tmpfs and was outside the timed disk-head filesystem.

## Storage fault found during qualification

Host10 initially took approximately one second per create even though its internal KVM ready measurement was approximately 5.45 ms.
System-call tracing showed four approximately 250 ms `copy_file_range` calls copying the 4 GiB private overlay.
The prepared artifact and private head were on ext4, so the generic storage path correctly fell back to a full copy but violated the fast-host profile.
Moving both onto the same encrypted reflink-enabled XFS filesystem reduced the public create response to approximately 6.62 ms in the isolated probe.

The qualification invariant is strict: prepared artifacts and private heads must share one reflink-enabled XFS device.
`scripts/check-fast-storage.sh` provides a non-destructive operator preflight proof for this invariant.
Runtime HostProfile admission does not consume that proof yet, so this script is a qualification gate rather than a production fail-closed control.

## Lifecycle defect found during qualification

The first repeated bursts left successful machine-host children as zombies because their `Child` handles were dropped without a wait owner.
The KVM host path now transfers every successful child to one bounded process reaper and retains cleanup ownership after the handshake.
The ownership decision is recorded in [ADR 0042](../adr/0042-machine-host-child-reaping.md).
After rebuilding all three qualification listeners, the 100-way cohorts left zero children under each API parent.

## Reproduction

Run one shard per host with a common future epoch and then combine the raw shard files.

```sh
python3 -m benchmarks.computesdk_exact \
  --endpoint http://127.0.0.1:18787=40 \
  --tenant qualification \
  --release-at-epoch-ns "$RELEASE_NS" \
  --output /tmp/run-host03.json

python3 -m benchmarks.computesdk_exact.combine \
  /tmp/run-host03.json /tmp/run-host04.json /tmp/run-host10.json \
  > /tmp/run-combined.json
```

The complete shard and combined JSON documents are retained under [raw/2026-09-01-computesdk-http-east](raw/2026-09-01-computesdk-http-east/README.md).

## Remaining proof

The next campaign must run the actual ComputeSDK provider adapter from its Namespace GitHub runner against MIOSA's external endpoint.
That campaign must retain edge, authentication, scheduling, placement, load-balancer, and network latency rather than subtracting them.
Only that result can support an external provider ranking claim.
