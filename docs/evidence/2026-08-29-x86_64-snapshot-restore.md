# x86_64 snapshot capture and restore - 2026-08-29

## Evidence boundary

This result proves that SOMA can, on a real Ubuntu 24.04 x86_64 host with `/dev/kvm`, boot a Generation compiled from a real OCI image to the disconnected repair point its pinned guest agent reaches before any launch material exists, prove the quiesce preconditions of the device surface, pause vCPU 0 outside `KVM_RUN`, read every certified KVM and device state group in the fixed order, publish a memory object, a sterile overlay template, and a canonical state manifest, and then restore that one snapshot repeatedly into independent Instances that each consume a fresh launch page, repair their identity, entropy, and transport, authenticate over a fresh vsock endpoint, pass the fixed readiness probe, execute a bounded command, and shut down orderly.
Decision-map ticket #7 now has live x86_64 evidence for both halves, and the restore ordering of the machine contract has been exercised end to end.

It does not prove a cold page cache measurement, a hundred-way burst, a jail around the VMM process, prepared workers, network egress, certification, density, or any latency objective.
Every number is a debug-build, single-host, in-container observation and is not a certified budget.

## Execution environment

- SOMA Git revision: the `feat/live-snapshot` branch at the commit that adds this document, developed on top of `bd0234e`.
- Host kernel: `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC` x86_64, Ubuntu 24.04.4 LTS.
- CPU: Intel Core Ultra 9 275HX, microcode `0x11b`, `kvm_intel` loaded.
  This is a hybrid processor, which is what exposed the CPU-template determinism problem recorded below.
- Rust toolchain: `1.98.0 (88d9e12ae 2026-08-18)`, debug profile for the test process, `x86_64-unknown-linux-musl` release profile for the guest agent.
- Test process container: the host's interactive seat session ended earlier in this work and `systemd-logind` moved the `uaccess` ACL on `/dev/kvm` to the display-manager user, so the prebuilt test binary was executed inside an `ubuntu:24.04` container (image `sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`) started with `--device /dev/kvm --user 1000:1000 --group-add 993 --security-opt seccomp=unconfined`, with the repository, the pinned-kernel checkout, and the scratch directory bind-mounted at their host paths.
  The container adds no privilege beyond the device node and runs on the same host kernel and KVM module.
- The existing live proofs were re-run on the same tree and container: the halt guest, the PVH kernel boot, and the busybox sandbox boot all passed.

## Identities

- Guest kernel: `vmlinux-6.12.107-soma-v1`, 21,530,432 bytes, SHA-256 `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`.
- Guest agent: `scripts/build-guest-agent.sh` output, 823,480 bytes, SHA-256 `4727325b713a1538fdff5702f6ef2fa04a101b0129401f2ef07217afc4caf605`, statically linked and stripped.
- Source image: `docker.io/library/node:22`, exported by `docker save` into an OCI layout; the importer selected `linux/amd64`.
- EROFS root: `sha256:48a6cf92bd0b4a57ee7ea87f0d3efe774ad26bd47d6db4ed6c23c83dcfe8aa48`, 1,129,172,992 bytes, the same digest the cold-boot proof recorded for this image.
- Sterile overlay template from the compiler, 256 MiB class: `sha256:ecfecc597f7dfa7b98dec28adb5eeb3a15357e090cbadf62fb1c627dc41fb790`, 268,435,456 bytes.
- Initramfs, layout v2: `sha256:7fb437573d057a4080caa3556392683872def8c7c7ca8ff19752fcbcc7d104f6`.
- `GenerationId`: `sha256:0f482644d20e016240fae112e786a0ed101ff7398bbe4deed9b1f7209b7d10b0`; it changes between runs because the responder key bound into the initramfs is generated per run.
- Machine shape: 1 vCPU, 1 GiB RAM, 256 MiB writable class, captured guest CID 3.

## Published snapshot

| Object | SHA-256 | Bytes |
| --- | --- | ---: |
| `memory.raw` | `6f9c24e7ddbe7c6d3a5f860fb9ea5408f6fd3d17fd3399daa278406984c7bbca` | 1,073,741,824 |
| `overlay.raw` | `672b7cc39b81f0b452787708d852b6209338b1238b18e6d37178d6d3ac6d3bcc` | 268,435,456 |
| `state.somasnap` | `88daf80e3fbb9cd5e6c0dd7c1676dd659fe5e6e46f1c4651aa78a4d2951e817b` | 9,566 |

`memory.raw` is exactly the certified guest RAM size and covers guest-physical `[0, 0x40000000)`.
The launch-page slot is at `0xd0100000`, far above that range, so the slot the snapshot must not contain cannot be inside the image by construction.

`overlay.raw` is the private overlay head of the captured machine after the guest flushed it, and it is the sterile template every restore clones.
Publishing the quiesced head rather than the compiler's pristine template is what keeps the guest's page cache in `memory.raw` consistent with the filesystem a restored Instance is given; the compiler's template digest and the published one therefore differ.

Each object was written to a private staging name, flushed with `fsync`, hashed by reading back through the same handle, compared with the digest accumulated while writing, and published with a hard link that fails when the certified name already exists.
The state manifest was decoded independently from its staged bytes and compared with the manifest that was built before anything was published, and the directory was flushed after the last link.

## Capture

The machine was created and started with **no launch page written at all**.
The console tap observed `soma-guest-agent: awaiting launch material`, which the agent prints immediately after flushing filesystems and immediately before it blocks in the launch-page wait.
The retained console is 12,113 bytes and ends at that line.

The quiesce preconditions were proven in the fixed order: the Generation's agent booted and announced the repair point, ingress was disabled (network link down, no vsock connection, packet, or event), the device thread was joined, the overlay was flushed, vCPU 0 was kicked out of `KVM_RUN` and its descriptor reclaimed, a final bounded drain ran on the capture thread, and every guest-driven queue was proven to hold no head the device had not taken.
The receive and event queues held buffers the driver had posted in advance: network 0, vsock receive 256, vsock event 8.
Those are ordinary posted capacity, not unserviced work, and they are restored with the queue.

State was then read in the certified order: memory-slot layout, vCPU state, interrupt controller, SOMA-owned routing, KVM clock, PIT, and the five device states.
The source machine had entered `KVM_RUN` 40,424,127 ns after its creation began, and the capture point is reached about 190 ms of guest time later.

## Restore: one Instance to an executed command

The restored Instance consumed a launch page it had never seen, repaired entropy, identity, and network state, adopted its fresh vsock context identifier, authenticated, answered the probe, and returned `v22.23.2` from `/usr/local/bin/node --version` with exit status 0 and empty stderr.
Its console contains no `Run /init as init process` line, because no kernel booted: the only console output a restored Instance produces is the agent's own.

WARM timeline, single sample, debug build, in the container described above, in nanoseconds from the moment the restore began reading the manifest:

| Milestone | ns since restore began | delta ns |
| --- | ---: | ---: |
| validate manifest | 985,282 | 985,282 |
| create VM | 1,575,293 | 590,011 |
| map memory privately | 1,596,300 | 21,007 |
| register memory slots | 1,635,885 | 39,585 |
| irqchip, PIT, routes | 1,706,101 | 70,216 |
| devices restored | 1,774,340 | 68,239 |
| vCPU created | 3,004,797 | 1,230,457 |
| vCPU state restored | 3,369,557 | 364,760 |
| eventfds and interrupt state | 3,420,883 | 51,326 |
| launch page slot mapped | 4,973,769 | 1,552,886 |
| fresh launch page written | 5,234,702 | 260,933 |
| device thread serving | 5,672,348 | 437,646 |
| resume | 6,101,351 | 429,003 |
| launch page consumed | 17,438,844 | 11,337,493 |
| vsock connected | 17,441,726 | 2,882 |
| handshake done | 29,205,354 | 11,763,628 |
| repair done | 29,845,762 | 640,408 |
| ready | 33,024,843 | 3,179,081 |
| execute done | 135,176,777 | 102,151,934 |
| shutdown acknowledged | 186,606,864 | 51,430,087 |
| guest exit | 188,180,615 | 1,573,751 |
| cleanup | 206,649,422 | 18,468,807 |

Mapping the 1 GiB memory object took 21,007 ns because nothing is copied: it is one `MAP_PRIVATE | MAP_NORESERVE` mapping of the published file, handed to the machine, and registered as one KVM slot.
`launch page consumed` lags the guest by up to 1 ms because the host polls the slot at that interval.

## WARM percentiles over ten sequential restores

Ten further restores of the same snapshot, each with a fresh private head cloned from `overlay.raw`, each running `node --version` and shutting down.
Nearest-rank percentiles over the raw samples, no interpolation and no averaging; nanoseconds from the start of the restore.

| Milestone | p50 ns | p99 ns | min ns | max ns |
| --- | ---: | ---: | ---: | ---: |
| validate manifest | 458,042 | 528,473 | 410,435 | 528,473 |
| create VM | 1,160,126 | 1,301,297 | 1,062,461 | 1,301,297 |
| map memory privately | 1,177,897 | 1,319,790 | 1,078,632 | 1,319,790 |
| register memory slots | 1,209,946 | 1,379,862 | 1,109,412 | 1,379,862 |
| irqchip, PIT, routes | 1,451,878 | 1,723,555 | 1,263,545 | 1,723,555 |
| devices restored | 1,502,987 | 1,784,940 | 1,310,219 | 1,784,940 |
| vCPU created | 3,080,337 | 4,237,434 | 2,386,664 | 4,237,434 |
| vCPU state restored | 3,196,339 | 4,315,971 | 2,460,826 | 4,315,971 |
| eventfds and interrupt state | 3,242,037 | 4,357,088 | 2,501,985 | 4,357,088 |
| launch page slot mapped | 5,061,410 | 7,019,483 | 4,356,625 | 7,019,483 |
| fresh launch page written | 5,194,054 | 7,113,295 | 4,479,955 | 7,113,295 |
| device thread serving | 5,239,791 | 7,152,330 | 4,521,959 | 7,152,330 |
| resume | 5,372,448 | 7,236,619 | 4,614,848 | 7,236,619 |
| launch page consumed | 13,450,259 | 14,985,973 | 12,418,352 | 14,985,973 |
| vsock connected | 13,452,810 | 14,990,375 | 12,420,945 | 14,990,375 |
| handshake done | 23,106,530 | 24,000,527 | 20,633,746 | 24,000,527 |
| repair done | 23,851,648 | 24,746,739 | 21,234,215 | 24,746,739 |
| ready | 26,985,771 | 31,963,989 | 23,525,745 | 31,963,989 |
| execute done | 79,869,264 | 124,241,554 | 70,767,939 | 124,241,554 |
| shutdown acknowledged | 122,700,729 | 168,789,316 | 114,236,690 | 168,789,316 |
| guest exit | 123,820,372 | 170,485,530 | 114,620,349 | 170,485,530 |
| cleanup | 142,319,666 | 189,410,410 | 135,031,875 | 189,410,410 |

The host-side `restore` call itself, from the first manifest byte to a machine waiting for its launch page, had p50 5,164,715 ns and p99 7,120,250 ns over the same ten iterations.

Work kept outside these numbers: compiling the Generation, publishing the snapshot, and cloning the private overlay head from `overlay.raw`.
The head clone is a full 256 MiB copy here because the scratch filesystem is ext4; the [XFS reflink profile](2026-08-29-xfs-reflink-profile.md) already established that heads must come from a prepared pool rather than from an on-demand clone, and this run does not change that.

## Cold and warm side by side

The cold numbers are the `node:22` result in [the first sandbox command evidence](2026-08-29-x86_64-first-sandbox-command.md), measured from the moment machine creation began.
That machine had a 1 GiB writable class where this one has 256 MiB; the class affects the compiler and the head clone, not the machine timeline.

| Milestone | COLD, ns since creation began | WARM p50, ns since restore began |
| --- | ---: | ---: |
| vCPU running | 32,443,455 | 5,372,448 |
| guest agent authenticated | 160,131,840 | 23,106,530 |
| repair committed | 160,930,085 | 23,851,648 |
| `Ready` | 161,583,495 | 26,985,771 |
| command returned | 201,423,884 | 79,869,264 |
| cleanup complete | 218,847,957 | 142,319,666 |

Reaching `Ready` took about 6.0 times as long from a cold boot as from a restore on this host and build, and the interval that disappears is the kernel boot: the cold machine spent 108 ms between entering `KVM_RUN` and running `/init`, and the restored machine spends none.
These are debug-build, single-host, in-container numbers with a warm host page cache for `memory.raw` after the first restore.
They are not a certified budget, not a production-host measurement, and not a claim about the 10 ms objective.

## Two Instances from one snapshot are independent

Two restores of the same snapshot ran sequentially with context identifiers 5 and 6 against a captured identifier of 3.

- Instance identities differ, and so do the values derived from them: machine identities `27dffd73ac1f34e452696e32c712d8e0` and `ba46d5f6b111290bb556ceb1de68fa3c`, hostnames `soma-27dffd73ac1f` and `soma-ba46d5f6b111`, read from `/etc/machine-id` and `/proc/sys/kernel/hostname` inside each guest through the authenticated channel.
- Context identifiers differ and neither is the captured one.
  Reaching `Ready` is itself the proof that each guest kernel adopted its own assignment: the agent refuses to connect while the vsock device reports a context identifier other than the one on its launch page, so an Instance that kept the captured identifier could not have authenticated.
- The first Instance created `/soma-first-instance` on its writable root and could see it; the second Instance looked for the same path and its `ls` exited non-zero.
- The two private heads have different digests, and the first head differs from the sterile template it was cloned from.
- `memory.raw` has the same digest before and after both Instances ran, so writes through the private mapping never reached the shared object.

## Rejections

Each of these was produced from a sibling directory holding hard links to the untouched objects and one replaced object, and each was refused before any VM, vCPU, or mapping of the replaced object existed.

- One flipped byte in `state.somasnap`: `section role 0x0002 digest mismatch`, from the codec's per-section digest.
- One flipped byte in `memory.raw` under installation-time verification: `memory digest sha256:620c7cdd... does not match sha256:6f9c24e7...`.
  Without that verification the same object maps and runs, which the test also asserts: re-hashing a gigabyte is the installation and audit boundary that [snapshot format v1](../research/snapshot-format-v1.md) places outside the request path, not a warm-restore check.
- One flipped byte in the manifest's CPU-template digest: `CpuTemplate { expected: sha256:214170df..., actual: sha256:204170df... }`, from the constant-size header comparison that runs before any section payload is decoded.

## What the published objects contain

- The launch-page domain `SOMA-LAUNCH-PAGE` occurs twice in `memory.raw` and never in `overlay.raw` or `state.somasnap`.
  It occurs once in the pinned guest agent binary, because the agent's own code holds the constant it compares against.
  Every occurrence in the memory image was fed to the production launch-page decoder and none of them decoded: the image contains no launch page, which is expected, because no launch page was ever written to this machine.
- The Generation-scoped responder private key occurs twice in `memory.raw` and never in `overlay.raw` or `state.somasnap`.
  This is by construction and is recorded rather than hidden: the compiler binds that key into the initramfs as a fifth input and the agent holds it for the life of the machine, so it is in guest RAM before any capture can happen.
  It is Generation identity, identical for every Instance of the Generation, and it is not Instance authority.
  Per-Instance authority - Instance identity, operation identity, nonce, pre-shared key, assigned context identifier, and network identity - cannot be in the image because none of it had been created when the machine was captured (ADR 0024).

## Implementation facts the live runs produced

Three defects were found only by running this on a real machine, and all three are now contract rather than accident.

- `KVM_GET_SUPPORTED_CPUID` answers from whichever host processor services the call.
  On this hybrid CPU the same host returned different values between consecutive calls in leaf `0x1` (initial APIC identifier), leaves `0xb` and `0x1f` (x2APIC identifier), leaf `0x4` (core and thread sharing counts and cache geometry), and leaf `0x80000006` (cache description), so the certified CPU-template digest was not reproducible and a restore on the capturing host was rejected as a foreign CPU.
  The version 1 template now pins all of them; a 2,000-sample probe on this host showed no remaining variation.
- `IA32_XSS` must be carried in the snapshot.
  Without it a restored guest resumed with its extended-supervisor-state register back at zero, and the first task to return to user mode took a general-protection fault inside `XRSTORS`, which the guest reported as `Bad FPU state detected at restore_fpregs_from_fpstate` followed by a stack guard page hit and a kernel panic.
- `IA32_SPEC_CTRL` must be carried as well, or a restored machine would silently run with weaker speculation mitigations than the machine that was captured.

A fourth observation is a contract clarification rather than a defect: receive and event queues legitimately hold buffers the driver posted in advance, so requiring every queue to be empty at the capture point is wrong.
Only guest-driven queues must hold no unserviced head.

## Reproduction

```sh
SOMA_X86_64_VMLINUX=/path/to/vmlinux-6.12.107-soma-v1 \
SOMA_EROFS_TOOLS=/path/to/erofs-utils-1.9.4 \
SOMA_GUEST_AGENT=target/x86_64-unknown-linux-musl/release/soma-guest-agent \
SOMA_OCI_NODE_LAYOUT=/path/to/oci-node22 \
  cargo test --locked -p soma-kvm --test x86_64_snapshot_restore -- --ignored --test-threads=1 --nocapture
```

The six tests share one compiled Generation and one captured snapshot, so the suite must run single-threaded in one process.
Every test fails explicitly when `/dev/kvm`, the pinned kernel, its configuration text, the erofs-utils directory, or the guest agent is missing, and prints a skip when the `node:22` layout cannot be exported.
The whole suite took 483 s of wall time in this run, most of it compiling the Generation from the 1.1 GB image.

## What this does not prove

- No cold page-cache measurement: the ten-iteration loop re-maps a `memory.raw` that the host page cache already holds, and no measurement was taken with that cache dropped.
- No burst: restores were sequential, one Instance at a time, because the machine's `KVM_RUN` interrupt handler is process-wide and the current watchdog serializes proofs in one process.
  Nothing here shows a hundred-way or even a two-way concurrent restore.
- No jail: the VMM ran with the test's own privileges, with no seccomp, namespace, or cgroup policy, and the container is a host-access workaround rather than an isolation claim.
- No prepared workers or allocator: the test drove the machine directly through crate-internal seams, and each private head was copied on demand rather than taken from a prepared pool.
- No networking: the restored network device sits behind the link-down loopback backend, and no frame left any machine.
- No certification: compiler phase 5 and every conformance count remain unimplemented, and the responder public key is carried by the test rather than bound into the manifest.
- No latency claim: these are unoptimized debug-build numbers on a loaded development host inside a container, and they must not be compared with any Ready or first-command objective.
- No cross-host restore: capture and restore ran on the same host, so the compatibility check has been exercised against a deliberately corrupted CPU-template digest but never against a genuinely different host.
- Nothing here proves the snapshot survives a restart of the host, an artifact store, or an installation boundary; the objects were published and consumed from one scratch directory.
