# x86_64 warm path: three cuts between resume and Ready - 2026-08-30

## Evidence boundary

This result changes the warm restore path and measures each change on its own.
Three commits are measured here, in the order they were made and each against the state before it: the launch-page poll on both sides of the resume, the executor's wait for a reapable child, and the point in the restore at which the launch-page memory slot is added.

Nothing here relaxes a bound or trades a guarantee.
Readiness still requires authenticated repair plus the fixed self-probe through the production executor; the launch page is still consumed exactly once, zeroed, verified erased by the host, and retired; the Noise pattern, its binding, and its transcript are untouched; every deadline, kill, and descendant sweep is unchanged.

It does not prove a cold page-cache measurement, a burst, a jail, prepared workers, network egress, certification, or any latency objective.
Every number is a single-host, in-container observation of one machine shape on a host that was not quiesced, and is not a certified budget.

## Execution environment

- SOMA Git revision: the `perf/warm-path` branch at the commit that adds this document, on top of `d790555`.
- Host kernel: `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC` x86_64, Ubuntu 24.04.4 LTS.
- CPU: Intel Core Ultra 9 275HX, 24 logical processors, `kvm_intel` loaded. The host carried other interactive load throughout; the run-to-run spread below is mostly that.
- Rust toolchain `1.98.0 (88d9e12ae 2026-08-18)`, release profile for the test process and for `soma-kvm`, `x86_64-unknown-linux-musl` release profile for the guest agent.
- Test process container: `ubuntu:24.04` (`sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`) started with `--device /dev/kvm --user 1000:1000 --group-add 993 --security-opt seccomp=unconfined`, repository and scratch directory bind-mounted at their host paths.
- Guest kernel: `vmlinux-6.12.107-soma-v1`, SHA-256 `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`.
- Guest agent before these changes: 766,128 bytes, SHA-256 `22d6d7b909788f0d353db874e5ea1c688b6fcce2016caf104150d90380e989c6`.
- Guest agent after them: 766,128 bytes, SHA-256 `db037ea3345f90717961853ba6cf4fb30b1df555a1e328cc537b31e87bb2328c`. Statically linked and stripped.
- Guest agent at the branch head, which no longer renders the timing report: 766,128 bytes, SHA-256 `b687587abff9614b502044a5bc260c5d401604f0e6f34fe8192ef60a47644cbd`. The third session below was measured with it.
- Machine shape: 1 vCPU, 1 GiB RAM, 256 MiB writable class, captured guest CID 3, EROFS root `sha256:48a6cf92bd0b4a57ee7ea87f0d3efe774ad26bd47d6db4ed6c23c83dcfe8aa48`.

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

Each column below is two such runs of the ten-iteration loop, so twenty raw samples, with nearest-rank percentiles over them.
A percentile of a milestone is not the percentile of an interval, so the differences between adjacent rows are indicative, not additive.

## The four states

Nanoseconds since the restore began reading the manifest.
`launch page slot mapped` sits between `eventfds and interrupt state` and `fresh launch page written` in the first three columns and between `map memory privately` and `register memory slots` in the last, which is the third change.

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

Cumulative, from the first byte of the manifest, `Ready` p50 fell from about **19 ms to about 12 ms**.
Neither endpoint reproduces to the four figures printed above.
A second session on the same machine, the same two commits and the same profile measured 18.44 ms to 12.97 ms.
A third, at the branch head after the corrections below, measured ten restores at p50 12.20 ms with a spread of 10.78 ms to 13.86 ms.
What the three sessions support is the reduction, between 30% and 39%, and that the result is below Isorun's 22 ms; they do not support either endpoint as a point estimate.

The p99 columns are not a cumulative claim and are not quoted as one.
With twenty samples the nearest-rank p99 is index 19, the largest of the twenty, so every p99 printed here is a single worst-case draw rather than an interior order statistic.
The 29.77 ms in the before column is one pathological restore: the second session re-measured the same two endpoints at 21.59 ms and 14.95 ms, a 31% reduction rather than the 55% those two figures imply.
Read the p99 columns as the maximum of the samples behind them, per run, and take the spread rather than a merged percentile; an interior p99 would need about a hundred samples.

## A. The launch-page pickup was two poll intervals

The host writes the fresh page into its slot before it resumes the vCPU, so the page is already present when the restored guest runs again.
The 2026-08-30 breakdown measured the guest's own work on that page at 93 us in total, which refuted the uncached-mapping hypothesis and left the interval as waiting.
The waiting was the guest's 2 ms sleep between domain probes and the host's 1 ms sleep between reads of the slot; both are now 100 us.

The guest wakeups this buys are paid only while a machine sits at the disconnected repair point waiting to be captured, never by a running Instance, and each is one 16-byte volatile read that the same evidence measures at 1 us.
The host read is 16 bytes under one uncontended lock over a window that is now a fraction of a millisecond.

This change's own window is resume to the page being observed consumed, and it fell from 2.57 ms to 1.30 ms at the difference of the two medians.
The cumulative `Ready` p50 fell further over the same two columns, 19.08 ms to 16.49 ms, but that is not this change's result.
The remainder is the executor's child-exit race described under B, a roughly five-millisecond bimodal step this change does not touch, whose branch ratio differed between the two columns; no cumulative delta measured across them is attributable until B removes that bimodality.
It did not fall to the 0.2 ms the two poll intervals alone would predict.
The host stops seeing the domain as soon as the guest's erase writes the first byte, and everything the guest does up to that point is `look` at 1 us, `copy` at under 1 us, and the start of `erase`; the parse that follows is not in the window the host measures.
So essentially the whole 1.30 ms is not guest work and not, any longer, either poll.
That is the time between `start` returning on the host and the restored guest executing its next instruction: the vCPU thread entering `KVM_RUN` and the first faults taken against an empty second-stage page table.
This document does not measure that split, and no change here addresses it.

## B. The readiness probe was one flat five-millisecond sleep

`pipes::stream` returns when both of the child's pipes reach their end, which the kernel does in `exit_files()`, but the task only becomes reapable later in `exit_notify()`.
The executor lost that race on most runs, and the wait loop then slept a flat 5 ms before looking again.
The guest instrumentation showed `wait` bimodal at 13 us or 5,116 us across every run of every configuration.

The first check now waits 50 us and each further wait doubles up to the same 5 ms ceiling.
A child that outlives its own pipes still costs one cheap check per ceiling interval; the absolute deadline, the kill on expiry, and the complete descendant reaping are unchanged.

Inside the guest, `wait` is bimodal on the state this change was applied to: about 13 us when the parent wins the race and about 5,100 us when it loses, so its median reports whichever branch a run happened to take more often.
That median was 5,116 us in the column before the poll change and 23 us in the column after it, which is the state this change was made against.
Measured there, the effect is on the slow branch and on the tail, not on the median: `wait` p99 fell 5,375 us to 588 us, its mean fell 2,100 us to 317 us, and the samples on the slow branch fell from 8 of 20 to 0 of 20.
The median rose, 23 us to 458 us, because every sample now takes the same short path: the loop finds the child on the fourth check, 350 us of sleeps in.
`Ready` p50 fell 16.49 ms to 14.21 ms across the same two columns.

## C. The handshake was already collapsed by the release build

No change was made, and none is available without weakening the protocol.
Between `vsock connected` and `handshake done` the host measures 3.64 ms at p50 in the final state, of which the guest accounts for 3.34 ms:

| Guest step | p50 us |
| --- | ---: |
| `ident`, identity repair | 1,639 |
| `net`, network repair | 1,229 |
| `hswork`, Noise responder work | 447 |
| `hswait` plus `hssend`, handshake transport | 21 |

The handshake is two messages and therefore one round trip, which is the minimum for `Noise_NKpsk0_25519_ChaChaPoly_BLAKE2s`, and the 21 us of transport confirms the guest is not waiting on the peer.
The 447 us is X25519, BLAKE2s, and ChaCha20-Poly1305 on one vCPU.
Reducing it would mean fewer or weaker Diffie-Hellman operations, which is a cryptographic guarantee and is not on offer.
What actually dominates the window is identity and network repair, which are the state-repair steps themselves, not the protocol.

## D. A memory slot added after the vCPU exists costs two milliseconds

The launch-page slot is one 4 KiB anonymous mapping and one `KVM_SET_USER_MEMORY_REGION`, and it cost 2.07 ms.
The guest RAM slot is the same ioctl over a gigabyte and cost 42 us.
What differs is position in the sequence: the RAM slot is registered before the vCPU is created, and the launch page was registered after it.
Moving the call changed two things at once, whether the VM already has a vCPU and how many slots the memslot array already holds, and this run separates neither.
What it measures is that a memory-slot addition costs two milliseconds at that point in the restore and tens of microseconds before the machine is built.

The slot is now added immediately after the memory object is mapped, and it is bound before the machine adopts the VM so that on any later failure the VM is released before the mapping is, which is the ownership order `RamMapping` documents.
It is still the machine's own slot, still absent from every snapshot, still empty until the material is published just before the resume, and still consumed, verified, and retired exactly as before.

In its new position the slot costs well under a millisecond: 27 us at p50 in the session tabulated above, 86 us at p50 and 303 us on iteration 0 in an independent session at the branch head, against roughly two milliseconds in the old one.
The single figure is session-specific and the order of magnitude is the claim.
Only iteration 0 yields a paired interval, because the percentile printer sorts each milestone's samples independently, so a difference of two medians is not the median of a difference.
`Ready` p50 fell 14.21 ms to 11.70 ms across these two columns.

## The guest half, before and after

Microseconds, twenty restores each, nearest-rank over the raw samples.

| Step | before p50 | before p99 | after p50 | after p99 |
| --- | ---: | ---: | ---: | ---: |
| `wake` | 4,246 | 6,754 | 1,532 | 1,855 |
| `look` | 1 | 3 | 1 | 2 |
| `copy` | 1 | 2 | 0 | 1 |
| `erase` | 1 | 2 | 1 | 7 |
| `parse` | 113 | 209 | 122 | 137 |
| `hwrng` | 227 | 2,392 | 183 | 2,391 |
| `mix` | 24 | 42 | 19 | 40 |
| `crng` | 7 | 16 | 6 | 18 |
| `cid` | 32 | 141 | 25 | 81 |
| `vsock` | 397 | 958 | 430 | 765 |
| `ident` | 1,874 | 2,790 | 1,639 | 1,887 |
| `net` | 1,501 | 2,079 | 1,229 | 1,551 |
| `hswait` | 15 | 77 | 12 | 90 |
| `hssend` | 12 | 18 | 9 | 14 |
| `hswork` | 498 | 836 | 447 | 634 |
| `req` | 121 | 401 | 106 | 436 |
| `report` | 12 | 20 | 10 | 41 |
| `spawn` | 1,769 | 2,857 | 1,472 | 2,153 |
| `stream` | 194 | 619 | 198 | 885 |
| `wait` | 5,116 | 5,374 | 457 | 646 |
| `reap` | 675 | 874 | 550 | 744 |
| `term` | 52 | 110 | 48 | 71 |

`wake` is the observed length of one interrupted poll sleep and spans the capture boundary, so it measures the guest's perception of the pause rather than post-resume time; it fell because the sleep it interrupts is now 100 us rather than 2 ms.

## What is left, and what a prepared worker could take

The 3.01 ms before resume is now almost entirely two kernel calls that create objects holding no tenant state:

| Prologue step | p50 |
| --- | ---: |
| read and validate the manifest | 0.08 ms |
| `KVM_CREATE_VM` | 0.86 ms |
| `KVM_CREATE_VCPU` | 1.49 ms |
| everything else, including both memory slots | 0.58 ms |

Neither can be moved off the request path inside `restore`, which is one function that builds the whole machine.
A prepared worker in `soma-hostd` could hold a created VM before any request arrives: `KVM_CREATE_VM` depends on nothing from the snapshot, and the memory slots are registered afterwards from the manifest.
A prepared vCPU is not as simple: `KVM_CREATE_VCPU` must follow `KVM_CREATE_IRQCHIP`, and the platform this restore recreates takes its TSS address and its interrupt routes from the captured `VmState`, so a worker could only pre-create a vCPU against a platform it fixed in advance and then verified against each manifest.
That verification is the design question, and it belongs with the worker rather than here.

After resume the largest remaining items are identity repair at 1.64 ms and network repair at 1.23 ms.
Both are dominated by writes to `/etc` through the Instance-private overlay, where the first write to a file copies it up from the immutable root over virtio-blk.
Pre-warming those copy-ups before capture would put placeholder identity files into the published overlay template, which is asserted sterile; that is a change to the Generation contract, not a poll interval, and it was not made.

## Verification

`cargo fmt --all --check`, `cargo build --locked --workspace --all-targets`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo test --locked --workspace`, `cargo deny check`, and `scripts/check-architecture.sh` pass after each of the three commits.
All thirteen live KVM tests pass in the container that receives `/dev/kvm`: `kvm_probe` 1, `x86_64_halt_guest` 2, `x86_64_kernel_boot` 2, `x86_64_sandbox_boot` 2, `x86_64_snapshot_restore` 6.
