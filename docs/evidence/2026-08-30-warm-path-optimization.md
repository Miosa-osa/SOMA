# x86_64 warm path: the whole optimization, end to end - 2026-08-30

## What this document is

This is the retained result of one optimization pass over the warm restore path of a `node:22` Generation on x86_64 KVM.
It records where the path started, every change that was made, what each change was measured against, what was refused, and where the path finished.

The two documents it consolidates stay authoritative for their own detail:

- [the guest-side breakdown](2026-08-30-x86_64-warm-path-guest-breakdown.md), which established the debug-to-release step and attributed the post-resume interval to named guest steps;
- [the three cuts](2026-08-30-x86_64-warm-path-three-cuts.md), which measured each of the three code changes against the state immediately before it.

Nothing here relaxes a bound or trades a guarantee.
Readiness still requires authenticated repair plus the fixed self-probe through the production executor.
The launch page is still consumed exactly once, zeroed, verified erased by the host, and retired.
The Noise pattern, its binding, and its transcript are untouched, and every deadline, kill, and descendant sweep is unchanged.

## Execution environment

- SOMA Git revision: the `perf/warm-path` branch, rebased onto `origin/main` at `780e9ca`. The measured tree is the branch head; the rebase moved documents only.
- Host kernel: `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC` x86_64, Ubuntu 24.04.4 LTS.
- CPU: Intel Core Ultra 9 275HX, 24 logical processors, `kvm_intel` loaded. The host carried other interactive load throughout; the run-to-run spread below is mostly that.
- Rust toolchain `1.98.0 (88d9e12ae 2026-08-18)`. Release profile for the test process and for `soma-kvm`; `x86_64-unknown-linux-musl` release profile for the guest agent, which it always was.
- Test process container: `ubuntu:24.04` (`sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`) started with `--device /dev/kvm --user 1000:1000 --group-add 993 --security-opt seccomp=unconfined`, repository and scratch directory bind-mounted at their host paths.
- Guest kernel: `vmlinux-6.12.107-soma-v1`, SHA-256 `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`.
- Machine shape: 1 vCPU, 1 GiB RAM, 256 MiB writable class, captured guest CID 3, EROFS root `sha256:48a6cf92bd0b4a57ee7ea87f0d3efe774ad26bd47d6db4ed6c23c83dcfe8aa48`.
- Guest agent binaries, all statically linked and stripped:
  - uninstrumented, before the pass: 757,936 bytes, SHA-256 `3354349169daf0cd539212823963d0d9cfb50613073af3650b6e5bc6cf893139`;
  - instrumented, used for the diagnosis: 766,128 bytes, SHA-256 `22d6d7b909788f0d353db874e5ea1c688b6fcce2016caf104150d90380e989c6`;
  - after the three cuts, still rendering the timing report: 766,128 bytes, SHA-256 `db037ea3345f90717961853ba6cf4fb30b1df555a1e328cc537b31e87bb2328c`;
  - branch head, with the timing report behind a non-default feature so the shipped agent no longer carries it: 766,128 bytes, SHA-256 `b687587abff9614b502044a5bc260c5d401604f0e6f34fe8192ef60a47644cbd`. The final figures below were measured with this one.

## Reproduction

```sh
./scripts/build-guest-agent.sh
SOMA_X86_64_VMLINUX=.../vmlinux-6.12.107-soma-v1 \
SOMA_EROFS_TOOLS=.../erofs-utils-1.9.4 \
SOMA_GUEST_AGENT=target/x86_64-unknown-linux-musl/release/soma-guest-agent \
SOMA_OCI_NODE_LAYOUT=.../oci-node22 \
  cargo test --locked --release -p soma-kvm --test x86_64_snapshot_restore \
    -- --ignored --test-threads=1 --nocapture warm_restore_timing
```

Every percentile in this document is nearest-rank over raw per-iteration samples of that milestone.
Each milestone's samples are sorted independently, so a difference of two percentiles is not the percentile of a difference, and the rows of a milestone table are not additive.

## The two baselines

Both are the same ten-iteration loop over the same Generation on the same host, and both are cumulative from the first byte of the manifest.

| Baseline | `Ready` p50 | `Ready` p99 | min | max | n |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Debug baseline** (2026-08-29, debug host build) | 27.18 ms | 30.00 ms | 22.08 ms | 30.00 ms | 10 |
| **Release baseline** (2026-08-30, release host build, no code change) | 17.80 ms | 19.44 ms | — | 19.44 ms | 10 |

The debug baseline is the retained table in [the 2026-08-29 snapshot restore evidence](2026-08-29-x86_64-snapshot-restore.md), `ready` p50 27,182,350 ns and p99 29,996,251 ns.
The release baseline is the same test with an optimized host build and no source change at all: `ready` p50 17,803,594 ns and p99 19,443,465 ns.

The 9.4 ms between them is entirely host-side device, serial, and vsock emulation running optimized.
The VMM work before `resume` did not move, as it should not have: it is dominated by `KVM_CREATE_VCPU` and by mapping the launch-page slot, which are kernel calls no host optimization level can shorten.
The readiness probe interval did not move either — 6.70 ms debug against 7.12 ms release — and that was the first evidence that it was guest-side.

Because the debug-to-release step is a build profile and not a change to SOMA, the three code changes below are all measured against the **release** baseline.

## Milestone table after each change

Nanoseconds since the restore began reading the manifest, twenty samples per column (two runs of the ten-iteration loop), nearest-rank over them.
Columns are cumulative states, in the order the changes were made, each measured against the one to its left.
`launch page slot mapped` sits between `eventfds and interrupt state` and `fresh launch page written` in the first three columns and between `map memory privately` and `register memory slots` in the last, which is change D.

| Milestone | before p50 | before p99 | after A p50 | after A p99 | after B p50 | after B p99 | after D p50 | after D p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| validate manifest | 92,807 | 138,003 | 87,465 | 116,636 | 76,432 | 91,627 | 77,612 | 140,980 |
| create VM | 837,758 | 1,430,065 | 888,182 | 1,707,958 | 1,020,265 | 1,776,282 | 938,816 | 1,454,558 |
| map memory privately | 854,806 | 1,443,878 | 900,602 | 1,721,720 | 1,033,241 | 1,789,885 | 952,494 | 1,470,061 |
| register memory slots | 1,085,169 | 5,350,091 | 1,014,340 | 4,076,138 | 1,075,168 | 1,816,889 | 988,215 | 1,544,257 |
| irqchip, PIT, routes | 1,387,932 | 6,415,434 | 1,514,084 | 4,327,903 | 1,421,075 | 2,305,161 | 1,158,303 | 1,765,177 |
| devices restored | 1,408,686 | 6,442,557 | 1,538,311 | 4,346,405 | 1,436,023 | 2,326,851 | 1,180,434 | 1,785,482 |
| vCPU created | 2,848,332 | 8,449,132 | 2,908,754 | 5,965,804 | 3,243,157 | 4,374,607 | 2,669,825 | 3,994,814 |
| vCPU state restored | 2,901,127 | 8,524,576 | 2,954,482 | 6,042,562 | 3,308,768 | 4,421,121 | 2,733,356 | 4,045,254 |
| eventfds and interrupt state | 2,940,004 | 8,574,063 | 2,986,284 | 6,078,476 | 3,343,439 | 4,459,910 | 2,766,243 | 4,079,452 |
| launch page slot mapped | 5,425,588 | 12,007,952 | 4,957,699 | 9,118,138 | 5,414,949 | 6,546,609 | 979,869 | 1,519,420 |
| fresh launch page written | 5,582,775 | 12,209,295 | 5,113,690 | 9,234,905 | 5,599,907 | 6,709,587 | 2,883,174 | 4,197,665 |
| device thread serving | 5,614,609 | 12,251,095 | 5,157,782 | 9,273,719 | 5,638,555 | 6,748,617 | 2,918,487 | 4,243,246 |
| resume | 5,792,060 | 12,530,762 | 5,305,231 | 9,466,732 | 5,886,323 | 6,951,746 | 3,008,906 | 4,456,540 |
| launch page consumed | 8,357,392 | 14,870,236 | 6,605,772 | 12,159,782 | 6,937,642 | 8,052,651 | 4,227,455 | 5,526,337 |
| vsock connected | 8,838,068 | 14,905,107 | 7,544,209 | 14,291,457 | 7,773,163 | 8,757,713 | 5,070,802 | 7,504,900 |
| handshake done | 13,016,252 | 20,558,588 | 11,242,264 | 22,102,795 | 11,419,371 | 12,257,449 | 8,708,733 | 10,671,175 |
| repair done | 13,526,282 | 22,084,022 | 11,646,915 | 22,683,482 | 11,746,868 | 12,754,300 | 9,057,721 | 10,946,148 |
| **ready** | **19,076,348** | **29,774,813** | **16,494,712** | **26,403,302** | **14,206,348** | **15,275,296** | **11,699,029** | **13,386,794** |
| execute done | 67,305,372 | 92,183,780 | 67,968,634 | 85,100,141 | 53,074,019 | 57,138,671 | 53,253,398 | 62,651,128 |

The p99 columns of this table are not an interior order statistic.
With twenty samples the nearest-rank p99 is index 19, the largest of the twenty, so each is one worst-case draw.
The 29.77 ms in the before column is a single pathological restore; a second session re-measured the same two endpoints at 21.59 ms and 14.95 ms.
Read those columns as the maximum of the samples behind them and take the spread, not a merged percentile; an interior p99 would need about a hundred samples.

## Final result

Ten iterations at the branch head, release profile, in the container, uninstrumented shipped agent.
Nearest-rank over the ten raw samples, cumulative from the first byte of the manifest:

| Milestone | p50 ns | p99 ns | min ns | max ns |
| --- | ---: | ---: | ---: | ---: |
| resume | 2,705,568 | 4,298,878 | 2,175,962 | 4,298,878 |
| launch page consumed | 4,073,682 | 5,531,434 | 3,365,114 | 5,531,434 |
| vsock connected | 5,408,361 | 7,279,467 | 4,211,533 | 7,279,467 |
| handshake done | 8,862,671 | 10,647,215 | 7,788,189 | 10,647,215 |
| repair done | 9,560,233 | 11,207,403 | 8,127,906 | 11,207,403 |
| **ready** | **12,204,263** | **13,855,896** | **10,780,066** | **13,855,896** |
| execute done | 54,680,880 | 64,692,409 | 51,957,008 | 64,692,409 |

**`Ready` p50 12.20 ms, `Ready` p99 13.86 ms, over ten iterations**, with the ten raw samples in nanoseconds:

```text
10780066, 11562902, 11733069, 11792379, 12204263,
13297518, 13442808, 13533796, 13545783, 13855896
```

With ten samples the nearest-rank p99 is the tenth, so 13.86 ms is the maximum of this run and not an interior percentile.

Against the two baselines, at p50: **27.18 ms debug, 17.80 ms release, 12.20 ms final.**
The debug-to-release step is a build profile, not a change to SOMA.
The part this pass is responsible for is **17.80 ms to 12.20 ms**, and it does not reproduce to four figures.
Three sessions on the same host measured the same two endpoints at 19.08 to 11.70 ms, 18.44 to 12.97 ms, and the ten-iteration run above.
What they support together is a reduction **between 30% and 39%**; they do not support either endpoint as a point estimate.

## Exactly what changed

Three commits change behaviour on the warm path. Each was measured against the state immediately before it.

### A. `perf(launch-page): observe the fresh page within a fraction of a millisecond`

`crates/soma-guest-agent/src/main.rs`, `crates/soma-kvm/src/x86_64/sandbox/launch.rs`.

`PAGE_POLL`, the guest's sleep between launch-page domain probes, went from 2 ms to 100 us.
`CONSUME_POLL`, the host's sleep between reads of the launch-page slot, went from 1 ms to 100 us.

The host writes the fresh page into its slot before it resumes the vCPU, so the page is already present when the restored guest runs again; the guest-side breakdown measured the guest's own work on that page at 93 us in total, which refuted the uncached-mapping hypothesis and left the interval as waiting on two poll intervals.
The extra guest wakeups are paid only while a machine sits parked at the disconnected repair point waiting to be captured, never by a running Instance, and each is one 16-byte volatile read measured at 1 us.
The host read is 16 bytes under one uncontended lock over a window that is now a fraction of a millisecond.

The window this change owns, resume to the page being observed consumed, fell from 2.57 ms to 1.30 ms.
It did not fall to the 0.2 ms the two intervals alone would predict; what remains is the vCPU thread entering `KVM_RUN` and the first faults taken against an empty second-stage page table, which this document does not measure and no change here addresses.

### B. `perf(executor): check for a reapable child sooner than the ceiling wait`

`crates/soma-guest-agent/src/executor.rs` and its tests.

`pipes::stream` returns when both of the child's pipes reach their end, which the kernel does in `exit_files()`, but the task only becomes reapable later, in `exit_notify()`.
The executor lost that race on most runs and then slept a flat `WAIT_POLL` of 5 ms before looking again; guest instrumentation showed `wait` bimodal at 13 us or 5,116 us across every configuration.

The first check now waits `FIRST_WAIT_POLL` of 50 us and each further wait doubles up to the same 5 ms ceiling.
A child that outlives its own pipes still costs one cheap check per ceiling interval.
The absolute deadline, the kill on expiry, and the complete descendant reaping are unchanged.

Measured against the state after A, the effect is on the slow branch and on the tail rather than on the median: `wait` p99 fell 5,375 us to 588 us, its mean fell 2,100 us to 317 us, and the slow-branch samples fell from 8 of 20 to 0 of 20.
The median rose from 23 us to 458 us, because every sample now takes the same short path and finds the child on the fourth check, 350 us of sleeps in.

### D. `perf(restore): add the launch-page memory slot while the VM has no vCPU`

`crates/soma-kvm/src/x86_64/snapshot/restore.rs`, and the milestone order in the test report.

`LaunchPageSlot::map_and_register` moved from after the vCPU had been created and its state restored to immediately after the guest memory object is mapped.
The slot is one 4 KiB anonymous mapping and one `KVM_SET_USER_MEMORY_REGION`, and in the old position it cost 2.07 ms; the guest RAM slot is the same ioctl over a gigabyte and cost 42 us.
In the new position the launch-page slot costs 27 us at p50 in the tabulated session, and 86 us at p50 with 303 us on iteration 0 in an independent session at the branch head.
The single figure is session-specific; the order of magnitude is the claim.

Moving the call changed two things at once — whether the VM already has a vCPU, and how many slots the memslot array already holds — and this run separates neither.
What it measures is that a memory-slot addition costs two milliseconds at that point in the restore and tens of microseconds before the machine is built.

The slot is bound before the machine adopts the VM, so on any later failure the VM is released before the mapping is, which is the ownership order `RamMapping` documents.
It is still the machine's own slot, still absent from every snapshot, still empty until the material is published just before the resume, and still consumed, verified erased, and retired exactly as before.

### Supporting changes that are not warm-path behaviour

- `perf(guest-agent): stop the transport clock once the handshake has read it` and `refactor(guest-agent): keep the timing report out of the shipped agent`: the per-step timing instrumentation is correct about transport and now sits behind a non-default Cargo feature, so the shipped agent does not carry it.
- `test(soma-kvm): fail the snapshot suite when its fixture cannot be built`: until this commit, seven of the thirteen live KVM tests returned `ok` while executing nothing when the `node:22` layout was absent. A missing prerequisite is now a failed run in every one of them, which is what makes the pass claim below worth its words.
- `test(soma-kvm): give every live run its own scratch tree`: two concurrent runs of one worktree previously shared a scratch tree.
- `test(soma-kvm): pin the launch-page slot ahead of the vCPU` and `test(guest-agent): assert the wait schedule the reap loop walks`: regression tests for D and B.
- `docs(soma-kvm): stop the warm loop calling its own output debug-build`: the loop's own banner was wrong about the profile it ran under.

## What was refused, and why

**C. The handshake was not touched.**
Between `vsock connected` and `handshake done` the host measures 3.64 ms at p50 in the final state, of which the guest accounts for 3.34 ms:

| Guest step | p50 us |
| --- | ---: |
| `ident`, identity repair | 1,639 |
| `net`, network repair | 1,229 |
| `hswork`, Noise responder work | 447 |
| `hswait` plus `hssend`, handshake transport | 21 |

The handshake is two messages and therefore one round trip, which is the minimum for `Noise_NKpsk0_25519_ChaChaPoly_BLAKE2s`, and 21 us of transport confirms the guest never waits on the peer.
The 447 us is X25519, BLAKE2s, and ChaCha20-Poly1305 on one vCPU.
Reducing it would mean fewer or weaker Diffie-Hellman operations.
That is a cryptographic guarantee and it is not on offer, so no change was made.

**Pre-warming the identity and network copy-ups was refused.**
Identity repair at 1.64 ms and network repair at 1.23 ms are now the largest post-resume items, and both are dominated by writes to `/etc` through the Instance-private overlay, where the first write to a file forces a copy-up from the immutable root over virtio-blk.
Pre-warming them before capture would put placeholder identity files into the published overlay template, which is asserted sterile.
That is a change to the Generation contract, not a poll interval, and it was not made.

**Moving `KVM_CREATE_VM` or `KVM_CREATE_VCPU` off the request path was refused here.**
The 3.01 ms before resume is now almost entirely two kernel calls that create objects holding no tenant state:

| Prologue step | p50 |
| --- | ---: |
| read and validate the manifest | 0.08 ms |
| `KVM_CREATE_VM` | 0.86 ms |
| `KVM_CREATE_VCPU` | 1.49 ms |
| everything else, including both memory slots | 0.58 ms |

Neither can be moved inside `restore`, which is one function that builds the whole machine.
A prepared worker in `soma-hostd` could hold a created VM before any request arrives, since `KVM_CREATE_VM` depends on nothing from the snapshot.
A prepared vCPU is not as simple: `KVM_CREATE_VCPU` must follow `KVM_CREATE_IRQCHIP`, and the platform this restore recreates takes its TSS address and its interrupt routes from the captured `VmState`, so a worker could only pre-create a vCPU against a platform it fixed in advance and then verified against each manifest.
That verification is a design question and it belongs with the worker, not with this pass.

**Relaxing any bound was refused.**
No deadline was lengthened, no kill was softened, no descendant sweep was shortened, the launch page's consume-zero-verify-retire sequence is unchanged, and the readiness definition — authenticated repair plus the fixed self-probe through the production executor — is the same one the baselines were measured against.

## External context: Isorun

For competitive context only, and **not** a like-for-like comparison.

The following figures are **an external service, measured over the network on 2026-08-30** by the coordinator, and are reproduced from [the Isorun creation telemetry evidence](2026-08-30-isorun-create-latency.md):

| Isorun cohort | reported `create_ms` p50 | reported `create_ms` p99 |
| --- | ---: | ---: |
| `node:22`, single create, sequential, n=10 | 22 ms | 27 ms |
| `node:22`, concurrency 100, n=100 | 73 ms | 207 ms |

`create_ms` is a field Isorun returns in its own create response; its timer endpoints are undocumented and SOMA did not observe the interval it measures.
The measuring host was on another continent, so the harness wall clock includes intercontinental transport.

**Their `create_ms` excludes image preparation.**
One create from `denoland/deno:alpine-2.0.5` reported `create_ms` 52 while the caller waited 4,808 ms of wall clock; a `node:22` create from the same host immediately afterwards reported 25 ms and completed in 283 ms.
That is consistent with the reported field excluding image acquisition and preparation for an image the service had not already prepared, and one request does not prove it.

These figures are recorded here so the SOMA numbers are read against something, not because the two are the same measurement.
SOMA's `Ready` requires authenticated repair and one bounded command through the production executor; the Isorun field is an unknown vendor-defined creation stage.
The SOMA figures also exclude the private head, admission, and transport, which the Isorun wall clock includes.
Do not subtract, divide, or rank these two columns against each other.

## Verification

`cargo fmt --all --check`, `cargo build --locked --workspace --all-targets`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo test --locked --workspace`, `cargo deny check`, and `scripts/check-architecture.sh` pass at the branch head and after each of the three warm-path commits.

All thirteen live KVM tests pass in the container that receives `/dev/kvm`: `kvm_probe` 1, `x86_64_halt_guest` 2, `x86_64_kernel_boot` 2, `x86_64_sandbox_boot` 2, `x86_64_snapshot_restore` 6.
Since `test(soma-kvm): fail the snapshot suite when its fixture cannot be built`, a missing prerequisite fails the run rather than reporting `ok` on an empty test, so that count means thirteen tests actually executed.

## What this does not prove

- **This is a single-restore measurement.** Every figure is one restore at a time, `--test-threads=1`, in a loop. It says nothing whatsoever about concurrent restores: no contention for the KVM device, the memslot array, the host page cache, memory bandwidth, or CPU was measured, and no claim about behaviour under concurrency can be derived from any number here. The Isorun concurrency-100 column above is the external service's own behaviour and is not a SOMA result.
- **One host, one machine shape.** A single loaded development laptop that was not quiesced, one 1 vCPU / 1 GiB / 256 MiB `node:22` Generation. There is no second host, no second CPU vendor, no server-class machine, and no other machine shape.
- **In a container.** The test process ran inside `ubuntu:24.04` with `/dev/kvm` passed through and `seccomp=unconfined`. The container adds no privilege, but it is not a production host and the VMM was not under the jail launcher.
- **Warm host page cache.** The loop re-maps a `memory.raw` the host page cache already holds. No cold-cache measurement was taken.
- **Not a certified budget and not a latency objective.** Nothing here is an SLO, a certification, or a commitment. The endpoints do not reproduce to four significant figures; only the range of the reduction does.
- **p99 here is a maximum, not an interior percentile.** Ten samples make the nearest-rank p99 the largest sample. An interior p99 would need roughly a hundred.
- **Nothing outside the restore.** No burst, no jail, no prepared workers, no network egress, no admission, no private head, no transport, no certification, and no cold boot.
- **The `wake` slot cannot split the interrupted sleep.** It cannot separate the pre-capture part from the post-resume part; only the host anchor bounds the post-resume part.
- **Change D confounds two variables.** Whether the VM has a vCPU and how many memslots already exist both changed with the move, and this run separates neither.
