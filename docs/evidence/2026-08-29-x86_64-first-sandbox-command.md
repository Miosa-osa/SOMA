# x86_64 first authenticated sandbox command - 2026-08-29

## Evidence boundary

This result proves that SOMA can, on a real Ubuntu 24.04 x86_64 host with `/dev/kvm`, take a Generation compiled by the production compiler from a real OCI image, cold-boot it on the `soma-kvm` sandbox machine with the five fixed virtio-mmio devices wired to KVM through ioeventfds and irqfds, deliver fresh launch material through the dedicated launch-page slot, run the statically linked guest agent as PID 1 through early init, EROFS plus ext4 overlay composition, and switch-root, observe the guest consume and erase the launch page, complete the Noise handshake over the vsock control device, commit repair by verifying the erased page and retiring its slot, require the fixed readiness probe, execute one bounded command and receive its exact bytes and exit status over the authenticated channel, acknowledge an authenticated shutdown, observe the orderly reset exit, and release every thread, descriptor, route, and mapping it created.
It is the first working-sandbox milestone: decision-map tickets #5 and #8 have live x86_64 evidence for the cold-boot path, and ticket #6 phase 4 is partially exercised.

It does not prove network egress, snapshot capture or restore, a jail around the VMM process, prepared workers, certification, density, or any latency objective.
Every number is a single-sample, debug-build, cold-boot observation on a busy development host and is not a benchmark.

## Execution environment

- SOMA Git revision: the `feat/kvm-integration` branch at the commit that adds this document, developed on top of `9f3a656` and rebased onto `origin/main` at `b1bb606` after the runs; the rebase touched no code the machine or the guest agent executes, and the busybox proof passed again on the rebased tree.
- Host kernel: `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC` x86_64, Ubuntu 24.04.4 LTS.
- CPU: Intel Core Ultra 9 275HX, microcode `0x11b`, `kvm_intel` loaded.
- Rust toolchain: `1.98.0 (88d9e12ae 2026-08-18)`, debug profile for the test process, `x86_64-unknown-linux-musl` release profile for the guest agent.
- Test process container: the host's interactive seat session ended during this work and `systemd-logind` moved the `uaccess` ACL on `/dev/kvm` to the display-manager user, so without `sudo` the test process could no longer open the device directly.
  The prebuilt test binary was therefore executed inside an `ubuntu:24.04` container (image `sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`) started with `--device /dev/kvm --user 1000:1000 --group-add 993 --security-opt seccomp=unconfined`, with the repository, the pinned-kernel checkout, and the scratch directory bind-mounted at their host paths.
  The container adds no privilege beyond the device node, runs on the same host kernel and KVM module, and supplied the same glibc 2.39 and e2fsprogs 1.47.0 as the host; the pinned erofs-utils 1.9.4 build came from the scratch directory.
  The image export used Docker on the host before the container run.
- The two earlier live proofs, the halt guest and the PVH kernel boot, passed on the host after the machine refactor and before the ACL change; the halt guest also passed inside the container, while the kernel-boot proof was not re-run there because its init fixture is built through Docker and Python, which the container lacks.

## Identities

- Guest kernel: `vmlinux-6.12.107-soma-v1`, 21,530,432 bytes, SHA-256 `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746`, with its configuration text SHA-256 `5369fdac4e4d691a0e21da5d99355c7006d55ae1ec8e9f3f439581d92771b28a`.
- Guest agent: `scripts/build-guest-agent.sh` output, 819,384 bytes, SHA-256 `6f3f657366a422d497b66f76b75cead972c6ad2cc9d1c00cdd38057ce0ca0eb0`, statically linked and stripped.
  After the runs below, `boot.rs` was split into `boot/devices.rs` to respect the file-size limit without changing behavior; the agent rebuilt from the committed source is 823,480 bytes with SHA-256 `d4c29837dd72c3fb8ec533e7c148a61aed1d890930dbe73eec558882a0e6b132`, and the busybox test passed again with it (`Ready` 137.6 ms after `KVM_RUN`, `GenerationId` `sha256:537c06203beb409333c11be41c379d7812237e16a95b3ad5652012dc14a3f795`) without the numbers below being re-recorded.
- Source image: `docker.io/library/busybox:stable-musl`, exported by `docker save` on Docker 29.3.0 into an OCI layout whose top index names the multi-platform index `sha256:3c6ae8008e2c2eedd141725c30b20d9c36b026eb796688f88205845ef17aa213`; the importer selected `linux/amd64`.
- Normalized tree: `sha256:5c47256d83adfa1d6162df9991dcd5e0f65660111e7e3f9391472069356094e1`, 424 entries.
- EROFS root: `sha256:6eeb5664f2ec671974c623638d4d4047cbfe5f6d5d03c41e3ed8f7d0f430ea5e`, 1,511,424 bytes, built by erofs-utils 1.9.4 (`mkfs.erofs` SHA-256 `c461be6b3e9716df4ca6bb52cda519e9b545d2bd9b9996686e8acf453100a7b6`); identical in every run.
- Sterile overlay template, 64 MiB class: `sha256:ada7d6262e53685413ca7fc2159f61a8f8339e78ab34eb7280bf329f4f48a3cb`, 67,108,864 bytes, built by e2fsprogs 1.47.0.
- Initramfs, layout v2: `sha256:e1605383813d4e34f9601699831802e683801caa720892412c8aaabfa5836ad1`, 1,640,960 bytes, carrying `/init` and `/bin/soma-guest-agent` (both the agent above), `/dev/console`, `/dev/null`, and the per-run responder private key at `/etc/soma/responder.key`.
- `GenerationId`: `sha256:0b6a3a4ea53accf5d4aabfd3156f29653297de23497ac0d58b51ad506ddbebcc`.
  The previous run produced `sha256:4a064668c586d94aa83643e86304f91278cb2e27549e47986d68dcfb9a72a73a` with the same tree, root, and overlay digests, because the responder key bound into the initramfs is generated per run.
- Machine shape: 1 vCPU, 256 MiB RAM, 64 MiB writable class, guest CID 3, effective MAC `02:53:4f:4d:41:01`, link-down loopback network backend.

## Command line

The machine composes the line in `crates/soma-kvm/src/x86_64/cmdline.rs` and the test requires it to equal the `command_line` bytes bound into the `SOMAGEN` manifest:

```text
console=ttyS0 reboot=k panic=1 nomodule random.trust_cpu=off pci=off acpi=off noapic cryptomgr.notests virtio_mmio.device=4K@0xd0000000:5:0 virtio_mmio.device=4K@0xd0001000:6:1 virtio_mmio.device=4K@0xd0002000:7:2 virtio_mmio.device=4K@0xd0003000:8:3 virtio_mmio.device=4K@0xd0004000:9:4 rdinit=/init soma.lower=/dev/vda soma.upper=/dev/vdb
```

## Invocation

```sh
SOMA_X86_64_VMLINUX=kernel/out/vmlinux-6.12.107-soma-v1 \
SOMA_EROFS_TOOLS=/path/to/erofs-utils-1.9.4 \
SOMA_GUEST_AGENT=target/x86_64-unknown-linux-musl/release/soma-guest-agent \
  cargo test --locked -p soma-kvm --test x86_64_sandbox_boot -- --ignored --test-threads=1 --nocapture
```

`SOMA_OCI_BUSYBOX_LAYOUT` and `SOMA_OCI_NODE_LAYOUT` may name pre-exported OCI layouts; otherwise the test runs `docker save`.
The test fails explicitly when `/dev/kvm`, the kernel, its configuration text, the erofs-utils directory, or the guest agent is missing, and the node test prints a skip when the image cannot be exported.

## Measured boundary

The timeline clock starts when `SandboxMachine::create` begins, before the kernel and initramfs are read, and every milestone is the monotonic offset from that instant.
`RunStart` is the moment vCPU 0 has entered `KVM_RUN` with its interrupt mask installed.
`KernelInit` and `AgentReadyLine` are the host instants at which the console lines `Run /init as init process` and `soma-guest-agent: ready` completed in the 16550 model.
`LaunchPageConsumed` is observed by a 1 ms host poll of the slot for the vanished page domain, so it lags the guest by up to 1 ms.
`VsockConnected` is the host observing the accepted connection; `Handshake`, `Ready`, `Execute`, and `Shutdown` are marked by the test after the corresponding `soma-guest` call returned.
`LaunchPageRetired` is marked inside the repair commit after the host verified all 4,096 bytes read as zero and removed the slot with a zero-length `KVM_SET_USER_MEMORY_REGION`.
`GuestExit` is the vCPU thread returning the orderly `Reset` exit; `Cleanup` follows the device thread join, route deregistration, and the release of every mapping and descriptor.
Descriptor and thread counts are taken outside the timer, before the artifacts are opened and after `finish` returned.

## Result: busybox Generation, retained run

The command `/bin/busybox uname -a` with a 10 s timeout and a 65,536-byte output allowance returned `Exited(0)` with exactly these stdout bytes and empty stderr:

```text
Linux soma-4021a60cea3a 6.12.107-soma-v1 #1 SMP PREEMPT_DYNAMIC 2026-08-29T00:00:00Z x86_64 GNU/Linux
```

The hostname is the Instance-derived label the agent installed during identity repair.

COLD timeline, single sample, debug build, inside the container described above:

| Milestone | ns since creation | delta ns | ns since `KVM_RUN` |
| --- | ---: | ---: | ---: |
| CreateVm | 23,173,405 | 23,173,405 | - |
| MapRegister | 23,173,704 | 299 | - |
| Platform | 23,259,464 | 85,760 | - |
| Devices | 23,293,678 | 34,214 | - |
| LaunchPageMapped | 24,788,085 | 1,494,407 | - |
| LoadGuest | 37,923,246 | 13,135,161 | - |
| Vcpu | 39,179,904 | 1,256,658 | - |
| Events | 39,231,502 | 51,598 | - |
| LaunchPageWritten | 39,305,623 | 74,121 | - |
| EventLoop | 39,401,210 | 95,587 | - |
| RunStart | 39,503,267 | 102,057 | 0 |
| KernelInit | 179,554,518 | 140,051,251 | 140,051,251 |
| LaunchPageConsumed | 220,037,351 | 40,482,833 | 180,534,084 |
| VsockConnected | 220,042,773 | 5,422 | 180,539,506 |
| Handshake | 229,860,678 | 9,817,905 | 190,357,411 |
| LaunchPageRetired | 231,006,481 | 1,145,803 | 191,503,214 |
| Ready | 232,097,391 | 1,090,910 | 192,594,124 |
| AgentReadyLine | 232,204,687 | 107,296 | 192,701,420 |
| Execute | 239,693,975 | 7,489,288 | 200,190,708 |
| Shutdown | 241,193,525 | 1,499,550 | 201,690,258 |
| GuestExit | 242,431,399 | 1,237,874 | 202,928,132 |
| Cleanup | 261,362,829 | 18,931,430 | 221,859,562 |

`CreateVm` includes reading the 21.5 MiB kernel and 1.6 MiB initramfs (8.9 ms) and the capability probe, which creates and destroys a throwaway VM (13.5 ms).
The KVM phase durations were: `ReadKernel` 8,859,860; `Open` 8,431; `Probe` 13,521,754; `CreateVm` 743,083; `MapMemory` 6,939; `RegisterMemory` 32,539; `TssAddress` 2,366; `IrqChip` 24,986; `Pit` 56,872; `Devices` 36,491; `LaunchPage` 1,494,430; `LoadGuest` 13,135,066; `CreateVcpu` 1,146,188; `Cpuid` 99,494; `Regs` 10,745; `Events` 51,926; `EventLoop` 169,220; `Run` 203,030,857; `Cleanup` 18,931,381 ns.

The immediately preceding run of the same test, with the same artifacts except the per-run key, reached `KernelInit` at 115.7 ms, `Handshake` at 162.0 ms, `Ready` at 163.7 ms, `Execute` at 170.8 ms, and `GuestExit` at 173.1 ms after `KVM_RUN`, so the boot-to-Ready interval varied between 164 ms and 193 ms across the two samples on a loaded host.

Counters from the retained run:

- Port bus: `serial_in` 11,897, `serial_out` 12,724, `i8042_in` 1, `i8042_out` 1, `other_in` 0, `other_out` 0.
- UART: 12,343 transmit writes, 352 `IER` writes, 11,691 `LSR` reads, 74 transmit interrupts raised.
- MMIO exits on the vCPU thread: 156 reads, 187 writes, 0 transport violations, 0 queue-notify exits; every in-range notify was absorbed by its ioeventfd.
- Device thread, by slot: root 14 wakeups, 189 chains completed, 14 interrupts; overlay 21 wakeups, 51 chains, 21 interrupts; network 2 wakeups, 1 chain, 1 interrupt; vsock 14 wakeups, 25 chains, 22 interrupts; entropy 3 wakeups, 3 chains, 3 interrupts; 13 host-work wakeups; no chain rejected and no device fault.
- Exit: `Ok(Reset)`; launch page retired: `true`; descriptors 4 before and 4 after; threads 2 before and 2 after; peak 6 threads and 36 descriptors while running.
- The EROFS root read back with the same SHA-256 after the run; the private overlay head no longer matched the sterile template digest it was copied from.

## Serial excerpt

The full 168-line, 12,343-byte log is retained by the test under `target/tmp/x86_64-sandbox-boot/busybox/serial.log`.
Device discovery, root composition, and the agent's own lines:

```text
[    0.104289] virtio-mmio: Registering device virtio-mmio.0 at 0xd0000000-0xd0000fff, IRQ 5.
[    0.105035] virtio-mmio: Registering device virtio-mmio.1 at 0xd0001000-0xd0001fff, IRQ 6.
[    0.107277] virtio-mmio: Registering device virtio-mmio.2 at 0xd0002000-0xd0002fff, IRQ 7.
[    0.107973] virtio-mmio: Registering device virtio-mmio.3 at 0xd0003000-0xd0003fff, IRQ 8.
[    0.109234] virtio-mmio: Registering device virtio-mmio.4 at 0xd0004000-0xd0004fff, IRQ 9.
[    0.120852] serial8250: ttyS0 at I/O 0x3f8 (irq = 4, base_baud = 115200) is a 16550A
[    0.121846] random: crng init done
[    0.122930] virtio_blk virtio0: [vda] 369 4096-byte logical blocks (1.51 MB/1.44 MiB)
[    0.124908] virtio_blk virtio1: [vdb] 16384 4096-byte logical blocks (67.1 MB/64.0 MiB)
[    0.135096] Run /init as init process
[    0.135910] erofs: (device vda): mounted with root inode @ nid 36.
[    0.173387] EXT4-fs (vdb): mounted filesystem be4ea2dd-333d-4fa2-a314-aa8832de9f2d r/w with ordered data mode. Quota mode: disabled.
[    0.174294] overlayfs: "xino" feature enabled using 2 upper inode bits.
soma-guest-agent: ready
soma-guest-agent: shutdown acknowledged
[    0.197552] reboot: Restarting system
[    0.197715] reboot: machine restart
```

The kernel used the PIC in virtual wire mode (`noapic`), the five interrupt lines were delivered as edge-triggered irqfd pulses through KVM's default GSI routing, and `random: crng init done` at 0.12 s came from the virtio entropy device before the agent's own reseed.

## Host residency for one 1-vCPU guest with 256 MiB RAM

Debug build, single sample per line, taken by a 2 ms sampler thread inside the test process; the guest mapping is anonymous `MAP_PRIVATE`, so its resident pages are part of `RssAnon`.
The `VmRSS` and per-mapping values in one sample come from separate `/proc` reads and are not atomic with each other.

| Sample | VmRSS | RssAnon | RssFile | Guest mapping Rss | Threads | Open fds |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Test process before the run, artifacts open | 9,352 kB | 2,896 kB | 6,456 kB | none | 2 | 8 |
| Last sample with the guest mapped | 41,976 kB | 35,216 kB | 6,760 kB | 32,168 kB | 3 | 18 |
| Peak `VmRSS` while running | 46,800 kB | 40,168 kB | 6,632 kB | 16,836 kB | 3 | 9 |
| Maximum seen at any poll | - | - | - | - | 6 | 36 |

In the last consistent sample the guest kernel had touched 32,168 kB of the 262,144 kB registered, and the non-guest anonymous residency of the test process was about 3.0 MiB; the kernel image buffer is dropped after loading, unlike the earlier kernel-boot proof.
The 36-descriptor peak is the baseline 4 plus the KVM, VM, and vCPU descriptors, the serial irqfd, five device irqfds, eight queue ioeventfds and their eight duplicates for the vCPU thread, the host-work eventfd and its duplicate, the stop eventfd, the epoll instance, the root, overlay, and entropy files, and the test's own artifact handles.

## Result: node:22 Generation

The same test compiled the locally cached `docker.io/library/node:22` image (Docker image `sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d`, exported by `docker save` into an 802 MB OCI layout) into a 1 vCPU, 1 GiB RAM, 1 GiB writable-class Generation and booted it in the same container.

- Normalized tree: `sha256:2e48535fca4ce401af5271cd6df8e39fa6a723fe4ee0abd570dfcf02f2a1e41e`, 33,512 entries.
  The development Mac's recorded `node:22` normalization digest `sha256:5dac6c571b970375a978c3f2f8777883e5bdd582fb4b43a5b872f929a2c7adf6` with 33,534 entries was not reproduced, and the differing entry count shows the two hosts normalized different `node:22` image revisions rather than the same input; a same-input cross-host comparison remains to be made.
- EROFS root: `sha256:48a6cf92bd0b4a57ee7ea87f0d3efe774ad26bd47d6db4ed6c23c83dcfe8aa48`, 1,129,172,992 bytes.
- Sterile overlay template, 1 GiB class: `sha256:0a24f757ffd3e6208621593cdda61b0e9bb6c2bb6d5b59d5bf943a4379105cc2`, 1,073,741,824 bytes.
- Initramfs: `sha256:ee9468c9836a7a25774212e71312cf2750c950300a7336f0aded7ac60932c3f7`; `GenerationId`: `sha256:190ad2f23b4b9f0dd899e8b88d8c51dd7927f44a569fdad68bf53ab7170a978d`.
- The command `/usr/local/bin/node --version` with a 30 s timeout returned `Exited(0)`, stdout exactly `v22.23.2\n`, and empty stderr.
- The whole test took 369 s of wall time, almost all of it in import, normalization, EROFS formatting and independent verification of the 1.1 GB tree, and the 1 GiB template build; the machine part is below.

COLD timeline, single sample, debug build, inside the container:

| Milestone | ns since creation | delta ns | ns since `KVM_RUN` |
| --- | ---: | ---: | ---: |
| CreateVm | 20,227,840 | 20,227,840 | - |
| MapRegister | 20,228,198 | 358 | - |
| Platform | 20,716,140 | 487,942 | - |
| Devices | 20,740,920 | 24,780 | - |
| LaunchPageMapped | 22,378,522 | 1,637,602 | - |
| LoadGuest | 30,851,059 | 8,472,537 | - |
| Vcpu | 32,049,732 | 1,198,673 | - |
| Events | 32,080,414 | 30,682 | - |
| LaunchPageWritten | 32,130,861 | 50,447 | - |
| EventLoop | 32,189,460 | 58,599 | - |
| RunStart | 32,443,455 | 253,995 | 0 |
| KernelInit | 140,766,233 | 108,322,778 | 108,322,778 |
| LaunchPageConsumed | 152,394,281 | 11,628,048 | 119,950,826 |
| VsockConnected | 152,397,863 | 3,582 | 119,954,408 |
| Handshake | 160,131,840 | 7,733,977 | 127,688,385 |
| LaunchPageRetired | 160,930,085 | 798,245 | 128,486,630 |
| Ready | 161,583,495 | 653,410 | 129,140,040 |
| AgentReadyLine | 161,792,391 | 208,896 | 129,348,936 |
| Execute | 201,423,884 | 39,631,493 | 168,980,429 |
| Shutdown | 202,712,074 | 1,288,190 | 170,268,619 |
| GuestExit | 203,767,186 | 1,055,112 | 171,323,731 |
| Cleanup | 218,847,957 | 15,080,771 | 186,404,502 |

The `Execute` delta of 39.6 ms is the complete round trip of loading the 1.1 GB image's Node binary from EROFS through the root block device and reporting `v22.23.2`; the root slot serviced 272 wakeups and 950 chains during the run against 14 and 189 for busybox.
The KVM phase durations were: `ReadKernel` 7,934,590; `Open` 6,914; `Probe` 10,786,513; `CreateVm` 1,240,787; `MapMemory` 4,811; `RegisterMemory` 253,656; `TssAddress` 2,205; `IrqChip` 32,684; `Pit` 452,625; `Devices` 25,831; `LaunchPage` 1,636,966; `LoadGuest` 8,473,150; `CreateVcpu` 1,136,538; `Cpuid` 55,093; `Regs` 6,957; `Events` 30,805; `EventLoop` 108,608; `Run` 171,578,922; `Cleanup` 15,080,127 ns.

Counters: 415 MMIO reads and 446 writes with 0 transport violations and 0 notify exits; overlay slot 21 wakeups and 52 chains; network 3 wakeups and 2 chains; vsock 15 wakeups and 29 chains; entropy 3 wakeups and 3 chains; 16 host-work wakeups; no rejected chain and no fault; exit `Ok(Reset)`; launch page retired; descriptors 4 before and 4 after; threads 2 before and 2 after; the EROFS root digest unchanged and the private head changed.

Host residency with the 1 GiB guest mapped: last consistent sample `VmRSS` 100,648 kB, `RssAnon` 93,896 kB, `RssFile` 6,752 kB, guest mapping `Rss` 67,468 kB of 1,048,576 kB registered, so the guest kernel touched about 66 MiB to boot and run `node --version`; the test process itself started the run at `VmRSS` 32,608 kB because the compiler's buffers were still resident; peak 6 threads and 36 descriptors.

The node serial log has the same shape as the busybox one: `[vda] 275677 4096-byte logical blocks (1.13 GB/1.05 GiB)`, `[vdb] 262144 4096-byte logical blocks (1.07 GB/1.00 GiB)`, `Run /init as init process` at 0.1055 s guest time, the EROFS and ext4 mounts, the overlay, `soma-guest-agent: ready`, one `node (67)` process, `soma-guest-agent: shutdown acknowledged`, and the restart.

## Diagnostic observations

- The first boot reached the agent immediately but failed early init with `EEXIST`: the compiler's sterile overlay template already carries empty `upper` and `work` directories while the agent only knew how to create them; the agent now creates or verifies them, rejecting a non-empty or non-directory entry.
- The second boot consumed the launch page, repaired entropy, and connected on the vsock port, then failed identity repair with `tmpfs: Unknown parameter 'nosuid'`: the agent passed `nosuid,nodev` inside the tmpfs option string as well as in the mount flags, and the new mount API rejects unknown parameters; both stay mount flags only.
- The guest agent's stop path now uses `LINUX_REBOOT_CMD_RESTART`: with no ACPI and no paravirtual power-off, a power-off request degrades to `halt`, which parks the vCPU inside KVM with interrupts disabled and is invisible to the host, while `reboot=k` turns restart into the keyboard-controller reset pulse the machine already treats as the orderly `Reset` exit.
- The initramfs needed `/dev/console` and `/dev/null` nodes: the Rust runtime aborts PID 1 before `main` when descriptors 0 through 2 are closed and `/dev/null` cannot be opened, so layout v2 carries both nodes and the responder key.
- Zero transport violations were recorded even though the agent installs the MAC with `SIOCSIFHWADDR`: the machine reports the launch-page MAC in virtio-net configuration space, and Linux skips the driver call when the requested address already matches, so no configuration-space write reached the read-only device model.
- The host polls the launch page every 1 ms after `start`; a production launcher should instead treat the vsock connection as the consumption signal, since the guest connects only after the page was consumed and erased.

## What this does not prove

- No network egress: the network device sits behind the link-down loopback backend; the guest configured `eth0` and a default route, but no frame left the machine and no TAP or host network profile exists.
- No snapshot capture or restore: this is a cold boot from the compiled artifacts; the snapshot codec was not exercised and no memory object was captured.
- No jail: the VMM process ran with the test's own privileges, no seccomp, namespace, or cgroup policy was applied, and the container used for the run is a host-access workaround, not an isolation claim.
- No prepared workers, allocator, or `soma-vmm` process topology: the test process drove the machine directly through crate-internal seams and `soma-guest`.
- No certification: phase 5 of the compiler and every conformance count remain unimplemented, and the responder public key is carried by the test rather than bound into the manifest.
- No latency claim: the numbers are unoptimized debug-build cold-boot single samples under host contention and inside a container, and must not be compared with a restored snapshot or any Ready or first-command objective.
- The guest's own entropy, identity, and network repair effects were exercised once on a cold boot only; nothing here shows that a cloned Instance discards captured state.

## Superseded artifact facts

[ADR 0024, per-Instance guest responder authority](../adr/0024-per-instance-guest-responder-authority.md) removed the responder private key from the initramfs on 2026-08-30 and raised the layout to v3.
The initramfs digest and the layout-v2 statements above describe the run that was executed on 2026-08-29 and are retained unchanged as evidence of that run.
A run of the same test after that decision produces a different initramfs digest and no `/etc/soma/responder.key` entry.
