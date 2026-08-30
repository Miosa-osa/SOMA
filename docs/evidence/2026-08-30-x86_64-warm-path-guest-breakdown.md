# x86_64 warm path: optimized host build and the guest half of the timeline - 2026-08-30

## Evidence boundary

This result re-measures the warm restore of the same `node:22` Generation with an **optimized host build**, and adds **per-step timing inside the guest agent** so the interval between resume and `Ready` is attributed to a named step instead of four host-observed gaps.
It is measurement only: nothing on the warm path was made faster, no bound was relaxed, and no guarantee was traded.
Readiness still requires authenticated repair plus the fixed self-probe through the production executor, and the launch page is still consumed, zeroed, verified, and retired before repair commits.

It does not prove a cold page-cache measurement, a burst, a jail, prepared workers, network egress, certification, or any latency objective.
Every number is a single-host, in-container observation of one machine shape and is not a certified budget.

## Execution environment

- SOMA Git revision: the `perf/warm-path` branch at the commit that adds this document, on top of `d790555`.
- Host kernel: `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC` x86_64, Ubuntu 24.04.4 LTS.
- CPU: Intel Core Ultra 9 275HX, `kvm_intel` loaded.
- Rust toolchain `1.98.0 (88d9e12ae 2026-08-18)`.
  **Release profile for the test process and for `soma-kvm`**, which is the change from the 2026-08-29 tables; `x86_64-unknown-linux-musl` release profile for the guest agent, which is what `scripts/build-guest-agent.sh` has always produced.
- Test process container: `ubuntu:24.04` (`sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`) started with `--device /dev/kvm --user 1000:1000 --group-add 993 --security-opt seccomp=unconfined`, repository and scratch directory bind-mounted at their host paths.
- Guest kernel: `vmlinux-6.12.107-soma-v1`, SHA-256 `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`.
- Guest agent without the instrumentation: 757,936 bytes, SHA-256 `3354349169daf0cd539212823963d0d9cfb50613073af3650b6e5bc6cf893139`.
- Guest agent with the instrumentation: 766,128 bytes, SHA-256 `22d6d7b909788f0d353db874e5ea1c688b6fcce2016caf104150d90380e989c6`.
  Both are statically linked and stripped; the guest agent was already a release build in the 2026-08-29 runs, so the debug-to-release change below is entirely host-side.
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

## Debug and release host builds side by side

Nanoseconds since the restore began reading the manifest.
The debug columns are the retained numbers from [the 2026-08-29 snapshot restore evidence](2026-08-29-x86_64-snapshot-restore.md); the release columns are the same test on the same host with an optimized host build and the uninstrumented agent.
The single-sample columns are the first iteration of each ten-iteration loop, which is the table the debug evidence published; the percentile columns are nearest-rank over the ten raw samples, so a difference of two percentiles is not the percentile of a difference.

| Milestone | DEBUG single | RELEASE single | DEBUG p50 | RELEASE p50 | RELEASE p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| validate manifest | 496,420 | 100,372 | 385,735 | 77,537 | 100,372 |
| create VM | 1,242,274 | 685,276 | 1,141,846 | 772,818 | 1,722,213 |
| map memory privately | 1,264,452 | 702,291 | 1,159,567 | 783,207 | 1,735,983 |
| register memory slots | 1,299,737 | 990,465 | 1,187,867 | 990,465 | 1,978,769 |
| irqchip, PIT, routes | 1,382,164 | 1,067,332 | 1,673,203 | 1,197,031 | 2,335,383 |
| devices restored | 1,438,252 | 1,085,832 | 1,723,999 | 1,219,305 | 2,355,873 |
| vCPU created | 2,431,960 | 2,166,326 | 2,708,117 | 2,853,890 | 4,224,735 |
| vCPU state restored | 2,514,469 | 2,223,911 | 2,778,664 | 2,907,353 | 4,317,520 |
| eventfds and interrupt state | 2,560,578 | 2,259,212 | 2,816,470 | 2,944,151 | 4,358,214 |
| launch page slot mapped | 4,379,311 | 4,196,240 | 4,663,792 | 4,924,364 | 6,791,523 |
| fresh launch page written | 4,486,913 | 4,400,195 | 4,796,691 | 5,114,300 | 6,975,160 |
| device thread serving | 4,533,324 | 4,443,491 | 4,842,356 | 5,150,277 | 7,010,384 |
| resume | 4,666,571 | 4,520,670 | 5,140,512 | 5,341,913 | 7,307,173 |
| launch page consumed | 13,575,846 | 6,643,352 | 12,095,626 | 6,643,352 | 8,726,477 |
| vsock connected | 13,578,481 | 6,643,932 | 12,098,169 | 7,177,145 | 9,125,945 |
| handshake done | 23,595,492 | 10,120,993 | 19,958,986 | 10,430,554 | 12,457,810 |
| repair done | 24,152,334 | 10,447,562 | 20,480,019 | 10,688,275 | 13,352,425 |
| **ready** | **31,527,640** | **17,920,730** | **27,182,350** | **17,803,594** | **19,443,465** |
| execute done | 83,677,392 | 66,015,405 | 73,402,052 | 60,355,561 | 66,015,405 |

The VMM work before resume is unchanged, as it should be: it is dominated by `KVM_CREATE_VCPU` and by mapping the launch-page slot, which are kernel calls that a host optimization level cannot shorten.
Everything after resume moves:

| Interval | DEBUG single | RELEASE single | DEBUG p50 | RELEASE p50 |
| --- | ---: | ---: | ---: | ---: |
| restore through resume | 4.67 ms | 4.52 ms | 5.14 ms | 5.34 ms |
| resume to launch page consumed | 8.91 ms | 2.12 ms | 6.96 ms | 1.30 ms |
| launch page consumed to vsock connected | 0.00 ms | 0.00 ms | 0.00 ms | 0.53 ms |
| vsock connected to handshake done | 10.02 ms | 3.48 ms | 7.86 ms | 3.25 ms |
| handshake done to repair done | 0.56 ms | 0.33 ms | 0.52 ms | 0.26 ms |
| repair done to ready | 7.38 ms | 7.47 ms | 6.70 ms | 7.12 ms |
| **resume to ready** | **26.86 ms** | **13.40 ms** | **22.04 ms** | **12.46 ms** |

An optimized host build removes 13.5 ms of the 26.9 ms that follows resume in the published single sample, and 9.6 ms of the 22.0 ms at p50.
All of that saving is host-side device, serial, and vsock emulation.
The readiness probe interval does not improve at all, which is the first evidence that it is guest-side.
`Ready` at 17.8 ms p50 and 19.4 ms p99 measured from the first manifest byte is now below the 22 ms p50 the coordinator measured against the live Isorun API on 2026-08-30, though the two boundaries are not the same measurement and this one excludes the private head, admission, and transport.

## Per-step timing inside the guest

The agent now measures each repair step with one monotonic pair, stores it in a fixed slot, and renders two bounded console lines **after** it has already announced readiness, so the measurement is outside the interval it measures.
The lines from the first restored Instance of the run:

```text
soma-guest-agent: ready
soma-guest-agent: timing 1 wake=3660 look=1 copy=2 erase=0 parse=88 hwrng=2438 mix=19 crng=5 cid=24 vsock=287 ident=1456
soma-guest-agent: timing 2 net=1119 hswait=12 hssend=9 hswork=466 req=42 report=10 spawn=1168 stream=403 wait=5327 reap=541 term=45
```

Every value is microseconds.
Ten restores, nearest-rank percentiles over the raw samples:

| Step | What it measures | p50 us | p99 us | min us | max us |
| --- | --- | ---: | ---: | ---: | ---: |
| `wake` | observed length of the 2 ms launch-page poll sleep the restore interrupted | 3,660 | 3,974 | 3,459 | 3,974 |
| `look` | the 16-byte domain probe that found the page | 1 | 2 | 1 | 2 |
| `copy` | copying 4,096 bytes out of the `/dev/mem` view into locked memory | 2 | 3 | 0 | 3 |
| `erase` | overwriting the view with zeroes and reading every byte back | 0 | 1 | 0 | 1 |
| `parse` | validating and parsing the locked copy | 89 | 158 | 87 | 158 |
| `hwrng` | reading one 64-byte contribution from the virtio entropy device | 205 | 2,438 | 168 | 2,438 |
| `mix` | two `RNDADDENTROPY` calls plus `RNDRESEEDCRNG` | 20 | 44 | 18 | 44 |
| `crng` | proving `getrandom` no longer blocks | 7 | 10 | 5 | 10 |
| `cid` | waiting for the vsock device to report the assigned context identifier | 28 | 50 | 24 | 50 |
| `vsock` | creating and connecting the control socket | 289 | 618 | 272 | 618 |
| `ident` | identity repair: hostname, machine identity, `/run` and `/tmp`, wall clock | 1,532 | 1,826 | 1,425 | 1,826 |
| `net` | network repair: MAC, address, netmask, links, route, resolver files | 1,154 | 1,330 | 1,119 | 1,330 |
| `hswait` | handshake time blocked reading the host's first message | 12 | 15 | 11 | 15 |
| `hssend` | handshake time writing the second message | 9 | 14 | 9 | 14 |
| `hswork` | handshake time that is not transport, which is the Noise work | 466 | 534 | 367 | 534 |
| `req` | blocked waiting for `PrepareAndProbe` | 47 | 352 | 42 | 352 |
| `report` | sending `RepairComplete` | 10 | 11 | 7 | 11 |
| `spawn` | `fork` plus `execve` of the probe, to the parent's exec-status pipe | 1,291 | 2,294 | 1,168 | 2,294 |
| `stream` | polling the probe's two pipes until both close | 247 | 645 | 11 | 645 |
| `wait` | waiting for the probe's exit status after its pipes closed | 5,155 | 5,327 | 13 | 5,327 |
| `reap` | killing, reaping, and sweeping every descendant | 541 | 684 | 517 | 684 |
| `term` | sending the terminal report | 49 | 168 | 42 | 168 |

Grouped, with the percentile taken over the per-iteration sums rather than over the parts:

| Guest stage | p50 us | p99 us | min us | max us |
| --- | ---: | ---: | ---: | ---: |
| launch page copy, erase, validate | 93 | 162 | 88 | 162 |
| entropy repair | 240 | 2,462 | 192 | 2,462 |
| vsock transport | 316 | 668 | 302 | 668 |
| identity repair | 1,532 | 1,826 | 1,425 | 1,826 |
| network repair | 1,154 | 1,330 | 1,119 | 1,330 |
| Noise handshake | 487 | 558 | 387 | 558 |
| repair report exchange | 58 | 359 | 52 | 359 |
| readiness probe | 7,319 | 7,951 | 2,453 | 7,951 |
| **guest total, excluding the poll sleep** | **11,019** | **13,462** | **7,085** | **13,462** |

The guest accounts for 11.02 ms of the 12.46 ms the host measures between resume and `Ready`.
The 1.4 ms difference is the host's own 1 ms `CONSUME_POLL` observation lag, the remainder of the guest's interrupted poll sleep after resume, and the host-side halves of the two exchanges.

The instrumented run's host milestones are statistically indistinguishable from the uninstrumented one: `ready` p50 17.19 ms against 17.80 ms, p99 20.00 ms against 19.44 ms.
The two console lines are 232 bytes written after `Ready` was already marked, so they cannot inflate any milestone up to and including `Ready`.

## Diagnosis

### The launch-page pickup is a poll interval, not work

The guest's own launch-page work is **93 us**: 1 us to probe the domain, 2 us to copy 4,096 bytes out of the `/dev/mem` view into locked memory, under 1 us to overwrite the view and read every byte back, and 89 us to validate and parse the locked copy.
The mapping is not the expensive part; the suspicion that byte-at-a-time volatile access through `/dev/mem` would cost milliseconds is refuted by `copy=2` and `erase=0`.

What is left is two polling loops, one on each side.
The host writes the page **before** it resumes the vCPU, so the page is already present when the guest starts running; the guest is asleep in `thread::sleep(PAGE_POLL)` with `PAGE_POLL` 2 ms, and the host then notices the erased domain only on the next tick of `CONSUME_POLL`, which is 1 ms.
The host anchor bounds the whole thing: resume to launch page consumed is 1.30 ms p50, the guest erases the domain after 3 us of its own work, and up to 1 ms of the interval is the host's own observation granularity.
The guest therefore observes the page somewhere between 0.3 ms and 1.3 ms after resume, and does 93 us of work on it.

The `wake` slot records 3.66 ms p50 for a 2 ms request, measured on the guest's monotonic clock across the capture and restore boundary.
It shows that the guest arrives at the successful check through an ordinary poll wake-up rather than through a long stall, but it cannot on its own separate the part spent before the capture from the part spent after the resume, because this run does not establish how guest monotonic time relates to host time across the pause.
The host anchor above is what bounds the post-resume part, and it does not depend on that relationship.

The same interval was 8.91 ms in the published debug sample and 2.12 ms in the release sample, 6.96 ms against 1.30 ms at p50: an optimized host build removed most of it, and what survives is **two poll intervals and 93 us of real work**.

### The handshake interval is mostly not the handshake

The host milestone named `handshake done` closes an interval that contains three guest steps, because the agent repairs identity and network state between connecting the socket and answering the first handshake message:

| Inside `vsock connected` to `handshake done` | p50 us |
| --- | ---: |
| identity repair | 1,532 |
| network repair | 1,154 |
| Noise handshake, all three parts | 487 |
| accounted | 3,173 |
| host-observed interval | 3,253 |

The handshake itself is **487 us**, and it is not round trips and not scheduling: `hswait` is 12 us and `hssend` is 9 us, so 466 us of it is `Noise_NKpsk0_25519_ChaChaPoly_BLAKE2s` responder work on the guest, and the peer is never the reason the guest waits.
Identity repair is 1.53 ms of filesystem and mount syscalls on the overlay: two hostname writes, an atomic `machine-id` replacement that forces an overlay copy-up, two tmpfs mounts, and `clock_settime`.
Network repair is 1.15 ms of `ioctl` calls on an `AF_INET` socket plus two more overlay writes for the resolver and hosts files.

The debug table's 10.02 ms was therefore about 6.8 ms of debug-build host emulation over the same 3.2 ms of guest work, of which only 0.49 ms is cryptography.

### The readiness probe is a fixed 5 ms poll

The probe interval is the only one an optimized host build does not improve: 6.70 ms debug against 7.12 ms release.
The reason is visible in one slot.

`wait` is bimodal across ten iterations: `[13, 13, 14, 5137, 5155, 5193, 5214, 5295, 5304, 5327]`.
Three iterations paid nothing and seven paid almost exactly 5 ms, which is `executor::WAIT_POLL`.
`pipes::stream` returns as soon as both of the child's pipes reach end of file, which the kernel does in `exit_files()`; the task only becomes reapable later, in `exit_notify()`.
When the parent loses that race, `Child::try_wait` returns `Ok(None)` once and `wait_for_child` sleeps a full fixed 5 ms before asking again.

The rest of the probe is small and is not dynamic linking, because the probe is `/proc/self/exe --soma-ready-probe-v1` and the agent is a statically linked musl binary that returns from `main` immediately:

| Probe part | p50 us |
| --- | ---: |
| `fork` plus `execve`, to the parent's exec-status pipe | 1,291 |
| both pipes polled to end of file, which is the child's whole life | 247 |
| waiting for the exit status after the pipes closed | 5,155 |
| kill, reap the group, reap orphans, sweep `/proc` | 541 |
| terminal report | 49 |
| total | 7,283 |

The floor, on the three iterations that won the race, is 2.45 ms.

## What this does not prove

- No optimization: nothing on the warm path was changed except the addition of measurement.
- No cold page-cache measurement: the loop re-maps a `memory.raw` the host page cache already holds.
- No burst, no jail, no prepared workers, no networking, no certification.
- No production-host claim: the runs are in a container on a loaded development laptop.
- The `wake` slot cannot separate the pre-capture part of the interrupted sleep from the post-resume part; only the host anchor bounds the post-resume part.
